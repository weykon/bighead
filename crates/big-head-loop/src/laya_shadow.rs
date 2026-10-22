//! Bounded, nonblocking shadow observations. No decisions feed back into the ECS.

use crate::{AgentRuntime, TickActionReport};
use bevy_ecs::{resource::Resource, world::World};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

const MAX_PENDING: usize = 128;
const MAX_PREDICTIONS: u64 = 256;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Default)]
struct Tracker {
    pending: BTreeMap<String, Incident>,
}
struct Incident {
    id: String,
    fingerprint: String,
    script_hash: String,
    first: u64,
    repeats: u64,
    followed_up: bool,
}

fn evidence(report: &TickActionReport) -> Value {
    json!({"source_id":report.source_id,"correlation_id":report.correlation_id,
        "entity":format!("{:?}",report.entity),"script_hash":report.script_hash,
        "at_epoch":report.started_at_epoch,"duration_ms":report.duration_ms,
        "queued_sparks":report.queued_sparks,"execution_succeeded":report.error.is_none(),
        "error":report.error.as_ref().map(|s| s.chars().take(240).collect::<String>()),
        "error_preview_truncated":report.error.as_ref().is_some_and(|s| s.chars().count()>240)})
}

impl Tracker {
    fn ingest(&mut self, report: &TickActionReport) -> Vec<Value> {
        let mut events = Vec::new();
        // Stable source identity survives entity rotation. Full errors are hashed, never stored.
        let fingerprint = big_head_save::persistence::digest(
            format!("{}:{:?}", report.script_hash, report.error).as_bytes(),
        );
        if let Some(mut incident) = self.pending.remove(&report.source_id) {
            incident.repeats += 1;
            if !incident.followed_up {
                events.push(json!({"kind":"next_execution","incident_id":incident.id,
                    "evidence":evidence(report),"same_script":report.script_hash==incident.script_hash,
                    "decision_correct":null,"goal_achieved":null,"training_eligible":false}));
                incident.followed_up = true;
            }
            if report.error.is_none() {
                events.push(json!({"kind":"execution_recovered","incident_id":incident.id,
                    "evidence":evidence(report),"executions_since_open":incident.repeats,
                    "caused_by_laya":false,"decision_correct":null,"goal_achieved":null,"training_eligible":false}));
                return events;
            }
            if fingerprint == incident.fingerprint
                && report.started_at_epoch.saturating_sub(incident.first) < 86400
            {
                self.pending.insert(report.source_id.clone(), incident);
                return events;
            }
            events.push(json!({"kind":"incident_closed","incident_id":incident.id,
                "reason":"error_or_script_changed_or_24h_expired","goal_achieved":null,"training_eligible":false}));
        }
        if report.error.is_none() {
            return events;
        }
        if self.pending.len() >= MAX_PENDING {
            let oldest = self
                .pending
                .iter()
                .min_by_key(|(_, i)| i.first)
                .map(|(k, _)| k.clone())
                .unwrap();
            let removed = self.pending.remove(&oldest).unwrap();
            events.push(json!({"kind":"incident_closed","incident_id":removed.id,
                "reason":"tracking_capacity","goal_achieved":null,"training_eligible":false}));
        }
        let id = uuid::Uuid::new_v4().to_string();
        events.push(json!({"kind":"incident_opened","incident_id":id,"evidence":evidence(report),
            "request":{"scenario":"anomaly_triage","state":{
                "error":report.error.as_ref().map(|s|s.chars().take(240).collect::<String>()),
                "error_preview_truncated":report.error.as_ref().is_some_and(|s|s.chars().count()>240),
                "execution_succeeded":false,"consecutive_failures_observed":1,
                "pending_repair":null,"component_schema_checked":false,
                "queued_sparks":report.queued_sparks}},
            "training_eligible":false}));
        self.pending.insert(
            report.source_id.clone(),
            Incident {
                id,
                fingerprint,
                script_hash: report.script_hash.clone(),
                first: report.started_at_epoch,
                repeats: 0,
                followed_up: false,
            },
        );
        events
    }
}

#[derive(Default)]
struct Counters {
    queued: u64,
    dropped: u64,
    predictions: u64,
    errors: u64,
    skipped: u64,
    records: u64,
    stopped: bool,
}

#[derive(Resource)]
pub(crate) struct Shadow {
    tracker: Tracker,
    tx: mpsc::Sender<Value>,
    task: Option<tokio::task::JoinHandle<()>>,
    counters: Arc<Mutex<Counters>>,
    directory: PathBuf,
}

impl Drop for Shadow {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Shadow {
    pub(crate) fn ingest(&mut self, reports: &[TickActionReport]) {
        for report in reports {
            for event in self.tracker.ingest(report) {
                let accepted = self.tx.try_send(event).is_ok();
                let mut counts = self.counters.lock().unwrap();
                if accepted {
                    counts.queued += 1;
                } else {
                    counts.dropped += 1;
                }
            }
        }
    }
}

pub(crate) fn status(world: &World) -> Value {
    let Some(shadow) = world.get_resource::<Shadow>() else {
        return json!({"enabled":false,"mode":"shadow"});
    };
    let c = shadow.counters.lock().unwrap();
    json!({"enabled":true,"mode":"shadow","execution_enabled":false,
        "directory":shadow.directory,"pending_incidents":shadow.tracker.pending.len(),
        "queued_records":c.queued,"dropped_records":c.dropped,"written_records":c.records,
        "predictions":c.predictions,"inference_errors":c.errors,"inference_skipped":c.skipped,
        "writer_stopped":c.stopped,"max_predictions_per_session":MAX_PREDICTIONS,
        "queue_capacity":32,"max_pending_incidents":MAX_PENDING,"max_file_bytes":MAX_FILE_BYTES})
}

fn append(file: &mut std::fs::File, bytes: &mut u64, value: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    if *bytes + line.len() as u64 > MAX_FILE_BYTES {
        return Err(std::io::Error::other(
            "Laya shadow file reached 16 MiB session limit",
        ));
    }
    file.write_all(&line)?;
    file.flush()?;
    *bytes += line.len() as u64;
    Ok(())
}

async fn worker(
    mut rx: mpsc::Receiver<Value>,
    mut file: std::fs::File,
    base: String,
    counts: Arc<Mutex<Counters>>,
) {
    let mut bytes = 0;
    let mut next_inference = std::time::Instant::now();
    let mut attempts = 0;
    while let Some(event) = rx.recv().await {
        if let Err(error) = append(&mut file, &mut bytes, &event) {
            log::warn!("Laya shadow recording stopped: {error}");
            counts.lock().unwrap().stopped = true;
            break;
        }
        counts.lock().unwrap().records += 1;
        if event["kind"] != "incident_opened" {
            continue;
        }
        let now = std::time::Instant::now();
        let mut decision = json!({"kind":"shadow_prediction","incident_id":event["incident_id"],
            "executed":false,"decision_correct":null,"goal_achieved":null,"training_eligible":false});
        if now < next_inference || attempts >= MAX_PREDICTIONS {
            decision["skipped"] = json!("rate_limit_circuit_breaker_or_session_budget");
            counts.lock().unwrap().skipped += 1;
        } else {
            attempts += 1;
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                crate::laya_workflows::assess_at(&event["request"], &base),
            )
            .await;
            match result {
                Ok(Ok(prediction)) => {
                    decision["prediction"] = prediction;
                    counts.lock().unwrap().predictions += 1;
                    next_inference = now + Duration::from_secs(5);
                }
                error => {
                    decision["error"] = json!(match error {
                        Ok(Err(e)) => e,
                        _ => "Local Laya shadow inference timed out".into(),
                    });
                    counts.lock().unwrap().errors += 1;
                    next_inference = now + Duration::from_secs(300);
                }
            }
        }
        if let Err(error) = append(&mut file, &mut bytes, &decision) {
            log::warn!("Laya shadow recording stopped: {error}");
            counts.lock().unwrap().stopped = true;
            break;
        }
        counts.lock().unwrap().records += 1;
    }
}

impl AgentRuntime {
    /// Start session-local observations; no checkpoint content or scheduling changes.
    pub fn enable_laya_shadow(&mut self, root: &std::path::Path) -> Result<(), String> {
        if self.world.contains_resource::<Shadow>() {
            return Ok(());
        }
        let directory = root.join(format!("shadow-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("events.jsonl"))
            .map_err(|e| e.to_string())?;
        let header = json!({"kind":"session_started","session_id":self.session_id,
            "evidence_origin":if cfg!(test) {"isolated_test"} else {"runtime"},
            "world":self.world_manifest.as_ref().map(|m| m.paths.root_dir.display().to_string()),
            "mode":"shadow","execution_enabled":false,"training_eligible":false});
        append(&mut file, &mut 0, &header).map_err(|e| e.to_string())?;
        let base =
            std::env::var("BIGHEAD_LAYA_URL").unwrap_or_else(|_| "http://127.0.0.1:3031".into());
        let (tx, rx) = mpsc::channel(32);
        let counters = Arc::new(Mutex::new(Counters::default()));
        let handle = tokio::runtime::Handle::try_current().map_err(|e| e.to_string())?;
        let task = handle.spawn(worker(rx, file, base, counters.clone()));
        self.world.insert_resource(Shadow {
            tracker: Tracker::default(),
            tx,
            task: Some(task),
            counters,
            directory,
        });
        Ok(())
    }

    pub async fn finish_laya_shadow(&mut self) {
        let Some(mut shadow) = self.world.remove_resource::<Shadow>() else {
            return;
        };
        // Unknown is intentional: exit and queue loss never become successful labels.
        let final_record = json!({"kind":"session_ended","unresolved_incidents":shadow.tracker.pending.values().map(|i|i.id.clone()).collect::<Vec<_>>(),
            "dropped_records":shadow.counters.lock().unwrap().dropped,"goal_achieved":null,"training_eligible":false});
        let _ = tokio::time::timeout(Duration::from_secs(2), shadow.tx.send(final_record)).await;
        let mut task = shadow.task.take().unwrap();
        drop(shadow);
        if tokio::time::timeout(Duration::from_secs(3), &mut task)
            .await
            .is_err()
        {
            task.abort();
            log::warn!("Laya shadow drain timed out; pending evidence may be incomplete");
        }
    }
}

#[cfg(test)]
#[path = "laya_shadow_tests.rs"]
mod tests;
