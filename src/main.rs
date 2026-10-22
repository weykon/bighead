mod a2a;
mod app_input;
mod desktop_status;
mod frontend;
mod runtime_paths;
mod session_log;
mod tui;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use termimad::MadSkin;

use app_input::{bind_paste_image_keys, expand_image_placeholders, line_to_spark};
use big_head_ai::{Agent, ApiProvider};
use big_head_loop::A2aOrigin;
use big_head_loop::AgentRuntime;
use big_head_state::{StateHook, TerminalRenderer};
use big_head_view::{ViewChannel, ViewEvent, ViewServer, ViewSnapshotHandle};
use frontend::FrontendMode;
use runtime_paths::RuntimePaths;
use session_log::SessionLog;
use tokio::sync::{mpsc as tokio_mpsc, oneshot};

const TICK_MS: u64 = 50;

pub(crate) enum InputEvent {
    Line(String),
    QuestionAnswer { id: String, answer: Option<String> },
    Shutdown,
}

struct PendingA2a {
    task_id: String,
    message_id: String,
    conversation_id: String,
    from: String,
    entity: bevy_ecs::entity::Entity,
    reply: oneshot::Sender<a2a::A2aTaskReply>,
    tasks: a2a::A2aTaskStore,
}

fn annotate_user_spark(source: &str, text: &str) -> String {
    format!("Interaction Context:\n- source: {source}\n\nUser Spark:\n{text}")
}

async fn complete_a2a_tasks(rt: &AgentRuntime, pending: &mut Vec<PendingA2a>) {
    let mut remaining = Vec::with_capacity(pending.len());
    for task in pending.drain(..) {
        let Some(spark) = rt.world.get::<big_head_loop::Spark>(task.entity) else {
            let reply = a2a::A2aTaskReply {
                task_id: task.task_id.clone(),
                message_id: task.message_id.clone(),
                conversation_id: task.conversation_id.clone(),
                status: "failed".into(),
                from: "big-head".into(),
                to: task.from.clone(),
                text: String::new(),
                errors: vec!["A2A Spark disappeared before completion".into()],
            };
            task.tasks
                .lock()
                .await
                .insert(task.task_id.clone(), reply.clone());
            let _ = task.reply.send(reply);
            continue;
        };
        let status = match &spark.status {
            big_head_loop::SparkStatus::Done => "done",
            big_head_loop::SparkStatus::Failed => "failed",
            big_head_loop::SparkStatus::Blocked => "blocked",
            big_head_loop::SparkStatus::Interrupted => "interrupted",
            big_head_loop::SparkStatus::Pending | big_head_loop::SparkStatus::Processing => {
                remaining.push(task);
                continue;
            }
        };
        let mut errors = Vec::new();
        if status != "done" && !spark.result_summary.is_empty() {
            errors.push(spark.result_summary.clone());
        }
        let reply = a2a::A2aTaskReply {
            task_id: task.task_id.clone(),
            message_id: task.message_id.clone(),
            conversation_id: task.conversation_id.clone(),
            status: status.into(),
            from: "big-head".into(),
            to: task.from.clone(),
            text: spark.result_summary.clone(),
            errors,
        };
        task.tasks
            .lock()
            .await
            .insert(task.task_id.clone(), reply.clone());
        let _ = task.reply.send(reply);
    }
    *pending = remaining;
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().collect();
    let frontend_mode = FrontendMode::from_args(&args);
    let runtime_paths =
        RuntimePaths::from_args(&args).expect("failed to initialize Big Head world manifest");
    let checkpoint_path = runtime_paths.checkpoint.to_string_lossy().to_string();
    let view_snapshot_path = runtime_paths.view_snapshot.to_string_lossy().to_string();

    let agent = Agent::from_env(ApiProvider::MiniMax).expect("MiniMax_API_KEY not set in .env");
    let mut rt = AgentRuntime::new(agent);
    rt.set_world_manifest(runtime_paths.world.clone());

    // ── Cooperative cancellation via Ctrl+C ────────────────────────
    // Interactive modes own their input handler. Only headless mode
    // installs tokio's SIGINT handler, so it cannot compete with rustyline.
    //
    // Pattern:
    //   1st Ctrl+C → flag=true + graceful shutdown request. The main loop
    //                saves checkpoint and exits after the current safe point.
    //   2nd Ctrl+C → std::process::exit(130) (immediate, no checkpoint).
    //   Ctrl+D     → graceful shutdown request.
    //
    // Trade-off: during an in-flight LLM call, the 1st Ctrl+C won't
    // abort the HTTP request (no reqwest abort handle exposed). The
    // LLM call completes, then the cancel flag is observed. The 2nd
    // Ctrl+C bypasses this with an immediate, explicitly unsaved exit.
    let cancel_flag_for_reader = rt.cancel_flag.clone();

    // ── Local A2A bridge ─────────────────────────────────────────
    // The bridge is opt-in and binds to loopback only. It queues untrusted
    // messages into the normal Spark path; it never calls tools directly.
    let a2a_enabled = args.iter().any(|a| a == "--a2a-server");
    let mut a2a_rx: Option<tokio_mpsc::UnboundedReceiver<a2a::InboundA2a>> = None;
    let mut pending_a2a = Vec::new();
    let mut a2a_port = None;
    if a2a_enabled {
        let (tx, rx) = tokio_mpsc::unbounded_channel();
        let port = std::env::var("A2A_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3040);
        let server = a2a::A2aServer {
            port,
            world_id: runtime_paths.world_id().to_string(),
            tx,
        };
        tokio::spawn(async move {
            if let Err(error) = server.run().await {
                eprintln!("A2A server error: {error}");
            }
        });
        a2a_rx = Some(rx);
        a2a_port = Some(port);
        println!("A2A bridge enabled on http://127.0.0.1:{port} (--a2a-server)");
    }

    // ── View server (3D floating panel) ─────────────────────────
    // 默认关闭. 启动 binary 时加 `--view-server` 启用 axum server + WebSocket.
    // 启用后 agent 可以调 `render_view` tool 推 panel 到浏览器
    // (http://localhost:3030).
    let view_server_enabled = args.iter().any(|a| a == "--view-server");
    // Bridge channel: tokio task forwards ViewChannel.rx events into a
    // std::sync::mpsc the main tick loop polls (same shape as the rustyline
    // `rx`). `DragEnd` is synthesized server-side as a Move command, so the
    // agent doesn't need to handle it here.
    let mut view_rx: Option<mpsc::Receiver<ViewEvent>> = None;
    let mut view_snapshot: Option<ViewSnapshotHandle> = None;
    let mut view_port: Option<u16> = None;
    if view_server_enabled {
        let (channel, sink) = ViewChannel::new();
        let bridge_channel = channel.clone();
        rt.set_view_channel(Some(channel));
        let port: u16 = std::env::var("VIEW_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3030);
        view_port = Some(port);
        let server = ViewServer::new(sink, port);
        view_snapshot = Some(server.snapshot_handle());
        tokio::spawn(async move {
            if let Err(e) = server.run_with_global_sink().await {
                eprintln!("view server error: {e}");
            }
        });
        println!("View server enabled on http://localhost:{port} (--view-server)");

        let (view_tx, view_bridge_rx) = mpsc::channel::<ViewEvent>();
        tokio::spawn(async move {
            while let Some(event) = bridge_channel.recv_event().await {
                if view_tx.send(event).is_err() {
                    break;
                }
            }
        });
        view_rx = Some(view_bridge_rx);
    }
    let desktop_transcript_enabled = view_server_enabled;

    let mut log = SessionLog::new();
    let mut boot_messages = Vec::new();
    if let Some(port) = view_port {
        boot_messages.push(format!(
            "View server enabled on http://localhost:{port} (--view-server)"
        ));
    }
    if let Some(port) = a2a_port {
        boot_messages.push(format!(
            "A2A bridge enabled on http://127.0.0.1:{port} (--a2a-server)"
        ));
    }

    if std::path::Path::new(&checkpoint_path).exists() {
        match rt.restore(std::path::Path::new(&checkpoint_path)) {
            Ok(()) => {
                let message = format!(
                    "Restored world {} from {}",
                    runtime_paths.world_id(),
                    checkpoint_path
                );
                println!("{message}");
                boot_messages.push(message);
            }
            Err(e) => {
                let message = format!("Restore failed: {e}");
                eprintln!("{message}");
                boot_messages.push(message);
            }
        }
    }
    let (legacy_tick_actions, legacy_cleanup_sparks) = rt.purge_legacy_spark_cleanup_artifacts();
    if legacy_tick_actions > 0 || legacy_cleanup_sparks > 0 {
        let message = format!(
            "Removed legacy Spark cleanup artifacts: {legacy_tick_actions} TickAction(s), {legacy_cleanup_sparks} active Spark(s)"
        );
        println!("{message}");
        boot_messages.push(message);
        match rt.checkpoint(std::path::Path::new(&checkpoint_path)) {
            Ok(()) => log.checkpoint(&checkpoint_path, true),
            Err(e) => {
                eprintln!("Checkpoint failed after legacy cleanup purge: {}", e);
                log.checkpoint(&checkpoint_path, false);
                log.error(&format!("Checkpoint after legacy cleanup purge: {}", e));
            }
        }
    }
    // Re-hydrate view panels from the companion snapshot. Must happen after
    // ECS restore so the view channel is wired up (set_view_channel above).
    if let Some(ref handle) = view_snapshot {
        if std::path::Path::new(&view_snapshot_path).exists() {
            match handle.load(std::path::Path::new(&view_snapshot_path)).await {
                Ok(spawns) => {
                    let count = spawns.len();
                    if let Some(ref ch) = rt.view_handler.as_ref().and_then(|h| h.channel.as_ref())
                    {
                        for cmd in spawns {
                            ch.send(cmd);
                        }
                    }
                    let message = format!(
                        "Restored {} view panel(s) from {}",
                        count, view_snapshot_path
                    );
                    println!("{message}");
                    boot_messages.push(message);
                }
                Err(e) => {
                    let message = format!("View snapshot restore failed: {e}");
                    eprintln!("{message}");
                    boot_messages.push(message);
                }
            }
        }
    }
    rt.ensure_agent_entity();
    if view_server_enabled {
        rt.set_checkpoint_path(std::path::PathBuf::from(&checkpoint_path));
        rt.publish_character_demo(true);
    }

    // ── State hook + frontend ─────────────────────────
    let hook = StateHook::new(256);
    rt.set_state_hook(hook.clone());
    big_head_ai::log::set_quiet(true);
    let clipboard_images = app_input::new_clipboard_image_store();

    // Markdown 渲染器 —— 将 agent 的 markdown 输出转为 ANSI 终端格式
    let skin = MadSkin::default();

    // 从 ECS 中恢复命令历史（由 Spark 实体重建），然后启动前端 reader
    rt.set_checkpoint_path(std::path::PathBuf::from(&checkpoint_path));
    let history: Vec<String> = rt.past_spark_texts();
    let (tx, rx) = mpsc::channel::<InputEvent>();
    let tui_handle = if frontend_mode.is_tui() {
        rt.enable_user_questions();
        let handle = tui::spawn(hook.clone(), tx.clone(), clipboard_images.clone(), history);
        handle.status("Idle");
        handle.runtime(format!("World: {}", runtime_paths.world_id()));
        for event in rt.runtime_event_lines(30) {
            handle.runtime(format!("Restored runtime: {event}"));
        }
        for message in &boot_messages {
            handle.runtime(message.clone());
        }
        Some(handle)
    } else if frontend_mode.is_headless() {
        let shutdown_tx = tx.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel_flag_for_reader.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = shutdown_tx.send(InputEvent::Shutdown);
            }
        });
        println!(
            "Big Head Runtime [{}] — headless mode. Use the view/status server to send sparks. Ctrl+C saves and exits at the next safe point.",
            runtime_paths.world_id()
        );
        None
    } else {
        let _renderer = TerminalRenderer::new(hook.clone()).start();
        let clipboard_images_for_reader = clipboard_images.clone();
        thread::spawn(move || {
            let mut editor = DefaultEditor::new().expect("Failed to create line editor");
            bind_paste_image_keys(&mut editor, clipboard_images_for_reader);
            for text in history {
                editor.add_history_entry(&text).ok();
            }
            loop {
                match editor.readline("> ") {
                    Ok(line) => {
                        if tx.send(InputEvent::Line(line)).is_err() {
                            break;
                        }
                    }
                    Err(ReadlineError::Interrupted) => {
                        // Two-Ctrl+C pattern:
                        //   1st press: cooperative cancel + graceful shutdown.
                        //              The main loop will checkpoint and exit
                        //              after the current safe point.
                        //   2nd press: swap(true) returns true → immediate
                        //              process::exit(130). No checkpoint save,
                        //              but the user asked for hard-quit.
                        if cancel_flag_for_reader.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            println!("^C  Hard-quit immediately; checkpoint will not be saved.");
                            std::process::exit(130);
                        }
                        println!(
                            "^C  Cancellation requested. Saving checkpoint and exiting at the next safe point. Press Ctrl+C again to hard-quit without saving."
                        );
                        if tx.send(InputEvent::Shutdown).is_err() {
                            break;
                        }
                        continue;
                    }
                    Err(ReadlineError::Eof) => {
                        let _ = tx.send(InputEvent::Shutdown);
                        break;
                    }
                    Err(e) => {
                        eprintln!("read error: {}", e);
                        let _ = tx.send(InputEvent::Shutdown);
                        break;
                    }
                }
            }
        });

        println!(
            "Big Head Runtime [{}] — type a spark and press Enter. Ctrl+V pastes a clipboard image (Alt+V fallback). Ctrl+C saves and exits; press Ctrl+C twice to hard-quit.",
            runtime_paths.world_id()
        );
        None
    };

    if std::env::var("BIGHEAD_LAYA_SHADOW").as_deref() != Ok("0") {
        if let Err(error) = rt.enable_laya_shadow(std::path::Path::new("outputs/laya")) {
            log.error(&format!("Laya shadow disabled: {error}"));
        }
    }

    let mut last_process_poll = Instant::now() - Duration::from_secs(5);

    // Tick loop: 每 TICK_MS ms 醒一次，试下有冇输入，冇就 sleep
    loop {
        if let Some(tui) = &tui_handle { tui.questions(rt.pending_user_questions()); }
        let mut answer_display = None;
        let input = match rx.try_recv() {
            Ok(InputEvent::QuestionAnswer { id, answer }) => {
                answer_display = Some(answer.as_ref().map(|text| format!("我的選擇：{text}")).unwrap_or_else(|| "已取消問題".into()));
                match rt.answer_user_question(&id, answer.as_deref()) {
                    Ok(Some(text)) => Ok(InputEvent::Line(text)),
                    Ok(None) => {
                        if let Some(tui) = &tui_handle { tui.log("已取消問題，沒有繼續相關操作。"); }
                        if let Err(error) = rt.checkpoint(std::path::Path::new(&checkpoint_path)) { log.error(&error); }
                        Err(mpsc::TryRecvError::Empty)
                    }
                    Err(error) => {
                        if let Some(tui) = &tui_handle { tui.error(error); }
                        Err(mpsc::TryRecvError::Empty)
                    }
                }
            }
            other => other,
        };
        match input {
            Ok(InputEvent::Line(line)) => {
                let line = expand_image_placeholders(&line, &clipboard_images);
                let spark = match line_to_spark(&line) {
                    Ok(spark) => spark,
                    Err(e) => {
                        if let Some(tui) = &tui_handle {
                            tui.log(format!("INPUT ERROR: {e}"));
                        } else {
                            eprintln!("\x1b[31m  INPUT ERROR:\x1b[0m {e}");
                        }
                        log.error(&format!("Input: {e}"));
                        None
                    }
                };
                if spark.is_none() {
                    // 空行：忽略，下个 tick
                } else if spark.as_deref().is_some_and(|spark| {
                    spark.eq_ignore_ascii_case("quit") || spark.eq_ignore_ascii_case("exit")
                }) {
                    break;
                } else {
                    let spark = spark.expect("checked Some above");
                    log.spark(&spark);
                    if let Some(tui) = &tui_handle {
                        tui.spark(answer_display.as_deref().unwrap_or(&spark));
                    }
                    if desktop_transcript_enabled {
                        desktop_status::started(&rt, answer_display.as_deref().unwrap_or(&spark));
                    }
                    rt.push_spark_front(annotate_user_spark(frontend_mode.source_label(), &spark));
                    let result = rt.run_until_stable().await;

                    for err in &result.errors {
                        if let Some(tui) = &tui_handle {
                            tui.error(err.clone());
                        } else {
                            println!("\x1b[31m  ERROR:\x1b[0m {}", err);
                        }
                        log.error(err);
                    }
                    for (agent, response) in &result.operation_responses {
                        log.agent(agent, response);
                        if let Some(tui) = &tui_handle {
                            tui.runtime(format!("[{agent}] {response}"));
                        }
                    }
                    for (agent, response) in &result.agent_responses {
                        if let Some(tui) = &tui_handle {
                            tui.agent(agent, response);
                        } else {
                            println!("\x1b[36m  [{}]\x1b[0m", agent);
                            skin.print_text(response);
                        }
                        log.agent(agent, response);
                    }
                    if desktop_transcript_enabled {
                        desktop_status::finished(&rt, &result.agent_responses, &result.errors);
                    }

                    match rt.checkpoint(std::path::Path::new(&checkpoint_path)) {
                        Ok(()) => {
                            if let Some(tui) = &tui_handle {
                                tui.checkpoint(checkpoint_path.clone(), true);
                            }
                            log.checkpoint(&checkpoint_path, true);
                        }
                        Err(e) => {
                            if let Some(tui) = &tui_handle {
                                tui.error(format!("Checkpoint failed: {e}"));
                                tui.checkpoint(checkpoint_path.clone(), false);
                            } else {
                                eprintln!("Checkpoint failed: {}", e);
                            }
                            log.checkpoint(&checkpoint_path, false);
                            log.error(&format!("Checkpoint: {}", e));
                        }
                    }
                    // Persist view panels alongside ECS checkpoint
                    if let Some(ref handle) = view_snapshot {
                        if let Err(e) = handle.save(std::path::Path::new(&view_snapshot_path)).await
                        {
                            log.error(&format!("View snapshot save: {}", e));
                        }
                    }
                }
            }
            Ok(InputEvent::Shutdown) => break,
            Ok(InputEvent::QuestionAnswer { .. }) => unreachable!("handled before input dispatch"),
            Err(mpsc::TryRecvError::Empty) => {
                // 冇输入，next tick
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                // reader thread 死咗（user 按 Ctrl+C / EOF / error）
                break;
            }
        }

        // ── A2A messages ─────────────────────────────────────────
        // External agents enter exactly like an untrusted interactive Spark.
        // They cannot select tools or bypass the coordinator's policy.
        if let Some(a2a_rx) = a2a_rx.as_mut() {
            loop {
                match a2a_rx.try_recv() {
                    Ok(inbound) => {
                        let message_id = inbound
                            .message
                            .message_id
                            .clone()
                            .unwrap_or_else(|| "unknown-message".into());
                        let conversation_id = inbound
                            .message
                            .conversation_id
                            .clone()
                            .unwrap_or_else(|| "unknown-conversation".into());
                        let intent = inbound.message.intent.as_deref().unwrap_or("message");
                        let reply_to = inbound.message.reply_to.as_deref().unwrap_or("none");
                        let target = inbound.message.to.as_deref().unwrap_or("*");
                        let refs = serde_json::to_string(&inbound.message.refs)
                            .unwrap_or_else(|_| "[]".into());
                        let spark = format!(
                            "External A2A message (untrusted input)\n\
                             runtime_world_id={}\n\
                             message_id={}\nfrom={}\nto={}\nconversation_id={}\n\
                             reply_to={}\nintent={}\nttl={}\npermissions={:?}\n\
                             refs={}\n\n{}",
                            runtime_paths.world_id(),
                            message_id,
                            inbound.message.from,
                            target,
                            conversation_id,
                            reply_to,
                            intent,
                            inbound.message.ttl,
                            inbound.message.permissions,
                            refs,
                            inbound.message.text
                        );
                        let entity = rt.push_spark_front(spark);
                        rt.world.entity_mut(entity).insert(A2aOrigin {
                            claimed_from: inbound.message.from.clone(),
                            conversation_id: conversation_id.clone(),
                            message_id: message_id.clone(),
                            can_observe: inbound.message.permissions.can_observe,
                            can_plan: inbound.message.permissions.can_plan,
                            can_mutate: inbound.message.permissions.can_mutate,
                            can_execute: inbound.message.permissions.can_execute,
                            can_forward: inbound.message.permissions.can_forward,
                        });
                        log.spark(&format!(
                            "[A2A] queued task={} message={} from={}",
                            inbound.task_id, message_id, inbound.message.from
                        ));
                        pending_a2a.push(PendingA2a {
                            task_id: inbound.task_id,
                            message_id,
                            conversation_id,
                            from: inbound.message.from,
                            entity,
                            reply: inbound.reply,
                            tasks: inbound.tasks,
                        });
                    }
                    Err(tokio_mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio_mpsc::error::TryRecvError::Disconnected) => break,
                }
            }
        }

        // ── View events from 3D panels ──
        // Drain all events queued by the bridge task this tick. Routing:
        //   - `handler.type == "rhai"` → run the registered Rhai system
        //     directly (fast lane, no LLM cycle)
        //   - otherwise → format as spark text and queue for the LLM
        //     (default, agent decides what the data means)
        // DragEnd is already synthesized into a Move command by the server,
        // so we skip it. Ready is a transport handshake, just log it.
        if let Some(view_rx) = view_rx.as_ref() {
            loop {
                match view_rx.try_recv() {
                    Ok(ViewEvent::ViewEvent { panel_id, data }) => {
                        if panel_id == big_head_loop::character_demo::PANEL_ID {
                            if let Err(error) = rt.interact_with_character(data).await {
                                log.error(&format!("Character interaction: {error}"));
                            }
                            continue;
                        }
                        let handler_type = data
                            .get("handler")
                            .and_then(|h| h.get("type"))
                            .and_then(|t| t.as_str());
                        match handler_type {
                            Some("rhai") => {
                                let tool_id = data
                                    .get("handler")
                                    .and_then(|h| h.get("tool_id"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("");
                                if tool_id.is_empty() {
                                    log.error(&format!(
                                        "view event rhai handler missing tool_id (panel={panel_id})"
                                    ));
                                    if let Some(tui) = &tui_handle {
                                        tui.error(format!(
                                            "view event rhai handler missing tool_id (panel={panel_id})"
                                        ));
                                    }
                                } else {
                                    log.spark(&format!(
                                        "view event [rhai] panel={panel_id} tool={tool_id}"
                                    ));
                                    if let Some(tui) = &tui_handle {
                                        tui.spark(format!(
                                            "view event [rhai] panel={panel_id} tool={tool_id}"
                                        ));
                                    }
                                    if let Err(e) = rt.run_event_handler(tool_id, &data) {
                                        log.error(&format!(
                                            "view event handler '{tool_id}' failed: {e}"
                                        ));
                                        if let Some(tui) = &tui_handle {
                                            tui.error(format!(
                                                "view event handler '{tool_id}' failed: {e}"
                                            ));
                                        }
                                    }
                                }
                            }
                            _ => {
                                let (spark, source) = if data.get("type").and_then(|v| v.as_str())
                                    == Some("spark")
                                    && data.get("surface").and_then(|v| v.as_str())
                                        == Some("desktop")
                                {
                                    (
                                        data.get("text")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or_default()
                                            .to_string(),
                                        Some("macOS desktop app"),
                                    )
                                } else {
                                    (format!("view event: panel={panel_id} data={data}"), None)
                                };
                                if spark.trim().is_empty() {
                                    continue;
                                }
                                log.spark(&spark);
                                if let Some(tui) = &tui_handle {
                                    tui.spark(&spark);
                                }
                                if desktop_transcript_enabled && source.is_some() {
                                    desktop_status::started(&rt, &spark);
                                }
                                let queued_spark = source
                                    .map(|source| annotate_user_spark(source, &spark))
                                    .unwrap_or(spark);
                                rt.push_spark_front(queued_spark);
                            }
                        }
                    }
                    Ok(ViewEvent::Ready { client_id }) => {
                        log.spark(&format!("[view ready] client={client_id}"));
                        if let Some(tui) = &tui_handle {
                            tui.runtime(format!("View ready: client={client_id}"));
                        }
                    }
                    Ok(ViewEvent::DragEnd { .. }) => {
                        // Server 已经合成成 ViewCommand::Move 广播，唔再需要处理
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }

        // OS process state is useful to inspect, but probing every 50ms would
        // make the supervisor itself noisy. Reconcile persisted ManagedProcess
        // facts at a low cadence and push one snapshot to the TUI.
        if last_process_poll.elapsed() >= Duration::from_secs(5) {
            let processes = rt.poll_managed_processes();
            if let Some(tui) = &tui_handle {
                tui.processes(processes);
            }
            last_process_poll = Instant::now();
        }

        // ── TickAction 评估 ──
        if view_server_enabled {
            rt.tick_programmable_scene();
        }
        let tick_reports = rt.evaluate_tick_actions();
        // Check pending saves even when no action ran; otherwise a quiet world
        // could retain unsaved changes indefinitely after a throttled tick.
        if let Some(Err(error)) = rt.checkpoint_ticks_if_due(std::path::Path::new(&checkpoint_path))
        {
            log.error(&error);
            if let Some(tui) = &tui_handle {
                tui.error(error);
            }
        }
        log.tick_action(&tick_reports);
        if let Some(tui) = &tui_handle {
            tui.tick_actions(tick_reports.clone());
        }

        // A capability-scoped LLM system observes noteworthy TickAction
        // reports. Routine repeats are coalesced by the runtime pipeline; the
        // agent authors any durable interpretation through normal ECS tools.
        match rt.advance_activity(std::path::Path::new(&checkpoint_path)) {
            Ok(Some(progress)) => {
                log.agent("activity-runtime", &progress.to_string());
                if let Some(tui) = &tui_handle { tui.runtime(format!("Activity: {progress}")); }
            }
            Ok(None) => {}
            Err(error) => log.error(&format!("activity-runtime: {error}")),
        }

        match rt.run_tick_observer().await {
            Ok(Some(summary)) => {
                log.agent("tick-steward", &summary);
                if let Some(tui) = &tui_handle {
                    tui.runtime(format!("Tick steward: {summary}"));
                }
            }
            Ok(None) => {}
            Err(error) => {
                log.error(&error);
                if let Some(tui) = &tui_handle {
                    tui.error(error);
                }
            }
        }

        match rt.run_interpretation_observer().await {
            Ok(Some(id)) => {
                log.agent("interpretation-observer", &format!("observation {id}"));
                if let Some(tui) = &tui_handle {
                    tui.runtime(format!("Interpretation observation: {id}"));
                }
            }
            Ok(None) => {}
            Err(error) => {
                log.error(&error);
                if let Some(tui) = &tui_handle {
                    tui.error(error);
                }
            }
        }

        // TickAction 可能通过 push_spark 产生了新 Spark，立即处理
        if rt.pending_spark_count() > 0 {
            let result = rt.run_until_stable().await;

            for err in &result.errors {
                if let Some(tui) = &tui_handle {
                    tui.error(err.clone());
                } else {
                    println!("\x1b[31m  ERROR:\x1b[0m {}", err);
                }
                log.error(err);
            }
            for (agent, response) in &result.operation_responses {
                log.agent(agent, response);
                if let Some(tui) = &tui_handle {
                    tui.runtime(format!("[{agent}] {response}"));
                }
            }
            for (agent, response) in &result.agent_responses {
                if let Some(tui) = &tui_handle {
                    tui.agent(agent, response);
                } else {
                    println!("\x1b[36m  [{}]\x1b[0m", agent);
                    skin.print_text(response);
                }
                log.agent(agent, response);
            }
            if desktop_transcript_enabled {
                desktop_status::finished(&rt, &result.agent_responses, &result.errors);
            }

            match rt.checkpoint(std::path::Path::new(&checkpoint_path)) {
                Ok(()) => {
                    if let Some(tui) = &tui_handle {
                        tui.checkpoint(checkpoint_path.clone(), true);
                    }
                    log.checkpoint(&checkpoint_path, true);
                }
                Err(e) => {
                    if let Some(tui) = &tui_handle {
                        tui.error(format!("Checkpoint failed: {e}"));
                        tui.checkpoint(checkpoint_path.clone(), false);
                    } else {
                        eprintln!("Checkpoint failed: {}", e);
                    }
                    log.checkpoint(&checkpoint_path, false);
                    log.error(&format!("Checkpoint: {}", e));
                }
            }
            // Persist view panels alongside ECS checkpoint
            if let Some(ref handle) = view_snapshot {
                if let Err(e) = handle.save(std::path::Path::new(&view_snapshot_path)).await {
                    log.error(&format!("View snapshot save: {}", e));
                }
            }
            complete_a2a_tasks(&rt, &mut pending_a2a).await;
        }

        // ── 心跳 ──
        tokio::time::sleep(Duration::from_millis(TICK_MS)).await;
    }

    if let Some(tui) = &tui_handle {
        tui.status("Saving checkpoint...");
    } else {
        println!("\nSaving checkpoint...");
    }
    match rt.checkpoint(std::path::Path::new(&checkpoint_path)) {
        Ok(()) => {
            if let Some(tui) = &tui_handle {
                tui.checkpoint(checkpoint_path.clone(), true);
            } else {
                println!("Saved to {}", checkpoint_path);
            }
            log.checkpoint(&checkpoint_path, true);
        }
        Err(e) => {
            if let Some(tui) = &tui_handle {
                tui.error(format!("Checkpoint failed: {e}"));
                tui.checkpoint(checkpoint_path.clone(), false);
            } else {
                eprintln!("Checkpoint failed: {}", e);
            }
            log.checkpoint(&checkpoint_path, false);
        }
    }
    // Persist view panels alongside final checkpoint
    if let Some(ref handle) = view_snapshot {
        if let Err(e) = handle.save(std::path::Path::new(&view_snapshot_path)).await {
            log.error(&format!("View snapshot save: {}", e));
        }
    }
    rt.finish_laya_shadow().await;
    log.close();
    if let Some(tui) = tui_handle {
        tui.shutdown();
    }
}
