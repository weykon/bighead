use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::Duration;

static LAYA_ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvRestore {
    url: Option<OsString>,
}

impl EnvRestore {
    fn capture() -> Self {
        Self {
            url: std::env::var_os("BIGHEAD_LAYA_URL"),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        unsafe {
            match &self.url {
                Some(value) => std::env::set_var("BIGHEAD_LAYA_URL", value),
                None => std::env::remove_var("BIGHEAD_LAYA_URL"),
            }
        }
    }
}

fn read_request(socket: &mut std::net::TcpStream) -> (String, Value) {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    loop {
        let mut block = [0; 2048];
        let count = socket.read(&mut block).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&block[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..end]);
            let content_length: usize = header
                .lines()
                .find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower.strip_prefix("content-length: ").map(str::to_owned)
                })
                .unwrap()
                .parse()
                .unwrap();
            if bytes.len() >= end + 4 + content_length {
                let path = header
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned();
                let body =
                    serde_json::from_slice(&bytes[end + 4..end + 4 + content_length]).unwrap();
                return (path, body);
            }
        }
    }
}

fn start_spec_service(
    decision_id: &str,
) -> (String, std::thread::JoinHandle<Vec<(String, Value)>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let decision_id = decision_id.to_owned();
    let thread = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let (path, body) = read_request(&mut socket);
            let response = if path == "/evaluate" {
                json!({"protocol_version":2,"decision_id":decision_id,
                    "spec_id":body["spec_id"],"spec_hash":body["spec_hash"],
                    "model":"mock-laya","revision":"test","device":"cpu",
                    "mode":"advisory","executed":false,"behavior_verified":false,
                    "confidence_calibrated_for_ecs":false,"input_truncated":false,
                    "answers":{"action":{"type":"choice","choice":"change",
                        "confidence":0.95,"probabilities":{"change":0.95,"defer":0.05}}}})
            } else {
                assert_eq!(path, "/outcome");
                json!({"recorded":true,"decision_id":body["decision_id"],
                    "training_eligible":false})
            };
            requests.push((path, body));
            let response = response.to_string();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
        }
        requests
    });
    (format!("http://{address}"), thread)
}

fn spec_call(mode: &str, entity_id: u32, expected: f64) -> ToolCall {
    ToolCall {
        id: format!("laya-spec-{mode}"),
        name: "laya_run_spec".into(),
        input: json!({"mode":mode,"spec":{
            "version":2,"id":"laya_gauge.set.v1","goal":"Set observed gauge to two",
            "state":{"entity_id":entity_id,"health":1.0},
            "questions":{"action":{"type":"choice","instructions":"Choose the bounded action",
                "criteria":{"change":"set health to two","defer":"leave it unchanged"}}},
            "policy":{"action_question":"action","defer_choices":["defer"],
                "gates":[{"question":"action","metric":"confidence","operator":"gte","value":0.8}],
                "on_gate_failure":"escalate"},
            "actions":{"change":{"tool":"modify_component","description":"Set LayaGauge to two",
                "input":{"entity_id":entity_id,"component":"LayaGauge","value":{"value":2.0}},
                "verify":[{"id":"health","tool":"get_component",
                    "input":{"entity_id":entity_id,"component":"LayaGauge"},
                    "expect":{"kind":"json_pointer_equals","pointer":"/value","value":expected}}]}}
        }}),
    }
}

fn call() -> ToolCall {
    ToolCall {
        id: "laya-executor-test".into(),
        name: "laya_decide".into(),
        input: json!({"goal":"Read the actual component values", "state":{"entity":42},
            "candidates":[{"operation":"get_entity","description":"Read entity component values"}]}),
    }
}

#[tokio::test]
async fn laya_catalog_and_candidate_scope() {
    let mut rt = AgentRuntime::new_for_test();
    let mut exec = make_executor(&mut rt);
    assert!(
        exec.dynamic_tool_definitions()
            .iter()
            .any(|t| t.name == "laya_decide")
    );
    let allowed = vec!["laya_decide".into()];
    exec.allowed_tools = Some(&allowed);
    let result = exec.execute(&call()).await;
    assert!(result.is_error);
    assert!(result.output.contains("available tools"));
    assert!(exec.actions_applied.is_empty());
}

#[tokio::test]
#[ignore = "Requires the real local Laya service on port 3031"]
async fn laya_live_executor_is_advisory() {
    let mut rt = AgentRuntime::new_for_test();
    rt.register_component_action::<TestHealth>("TestHealth");
    let entity = rt.world.spawn(TestHealth(42.0)).id();
    let entities_before = rt.world.entities().len();
    {
        let mut exec = make_executor(&mut rt);
        let mut request = call();
        request.input["state"] = json!({"entity":entity.index().index(),"TestHealth":42});
        let result = exec.execute(&request).await;
        assert!(!result.is_error, "{}", result.output);
        let result: Value = serde_json::from_str(&result.output).unwrap();
        println!("Live executor advice: {result}");
        assert_eq!(result["executed"], false);
        assert_eq!(result["behavior_verified"], false);
        assert!(exec.actions_applied.is_empty());
    }
    assert_eq!(rt.world.entities().len(), entities_before);
    assert_eq!(rt.world.get::<TestHealth>(entity).unwrap().0, 42.0);
}

#[tokio::test(flavor = "current_thread")]
async fn laya_spec_shadow_and_default_execute_verify_real_writes() {
    let _lock = LAYA_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _restore = EnvRestore::capture();
    let mut rt = AgentRuntime::new_for_test();
    rt.component_registry
        .define_dynamic_component(
            &mut rt.world,
            "LayaGauge",
            big_head_ecs::DynamicSchema {
                fields: BTreeMap::from([("value".into(), big_head_ecs::FieldType::Number)]),
            },
        )
        .unwrap();
    let entity = rt.world.spawn_empty().id();
    rt.component_registry
        .insert_component(&mut rt.world, entity, "LayaGauge", r#"{"value":1.0}"#)
        .unwrap();
    let entity_id = entity.index().index();

    let (url, server) = start_spec_service("shadow-decision");
    unsafe { std::env::set_var("BIGHEAD_LAYA_URL", url) };
    let result = make_executor(&mut rt)
        .execute(&spec_call("shadow", entity_id, 2.0))
        .await;
    assert!(!result.is_error, "{}", result.output);
    let value: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(value["policy"]["verdict"], "shadow");
    assert_eq!(value["execution"]["attempted"], false);
    assert_eq!(value["outcome"]["behavior_verified"], false);
    assert_eq!(
        crate::tools::helpers::execute_get_component(
            &rt.world,
            rt.component_registry.dynamic_store(),
            entity_id as u64,
            "LayaGauge"
        )
        .0,
        "LayaGauge: {\"value\":1.0}"
    );
    let requests = server.join().unwrap();
    assert_eq!(requests[1].0, "/outcome");
    assert_eq!(requests[1].1["execution_attempted"], false);

    let tx = crate::audit::begin_transaction(
        &mut rt.world,
        &mut rt.component_registry,
        "laya-spec-test",
        &[],
        None,
        None,
    );
    let (url, server) = start_spec_service("execute-decision");
    unsafe { std::env::set_var("BIGHEAD_LAYA_URL", url) };
    let result = {
        let mut executor = make_executor(&mut rt);
        executor.tx_id = Some(tx.tx_id.clone());
        executor
            .execute(&spec_call("execute", entity_id, 2.0))
            .await
    };
    assert!(!result.is_error, "{}", result.output);
    let value: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(value["policy"]["verdict"], "act");
    assert_eq!(value["execution"]["succeeded"], true);
    assert_eq!(
        value["verification"][0]["passed"], true,
        "unexpected read-back: {value}"
    );
    assert_eq!(value["outcome"]["behavior_verified"], true);
    assert_eq!(value["outcome"]["goal_achieved"], true);
    assert!(
        crate::tools::helpers::execute_get_component(
            &rt.world,
            rt.component_registry.dynamic_store(),
            entity_id as u64,
            "LayaGauge"
        )
        .0
        .contains("2.0")
    );
    let requests = server.join().unwrap();
    assert_eq!(requests[1].1["behavior_verified"], true);
    let records: Vec<_> = rt
        .world
        .query::<&crate::ActionRecord>()
        .iter(&rt.world)
        .filter(|record| record.tx_id == tx.tx_id)
        .map(|record| record.tool_name.clone())
        .collect();
    assert!(records.iter().any(|name| name == "modify_component"));
    assert!(records.iter().any(|name| name == "get_component"));

    let (url, server) = start_spec_service("failed-readback-decision");
    unsafe { std::env::set_var("BIGHEAD_LAYA_URL", url) };
    let result = make_executor(&mut rt)
        .execute(&spec_call("execute", entity_id, 99.0))
        .await;
    let value: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(value["execution"]["succeeded"], true);
    assert_eq!(value["verification"][0]["passed"], false);
    assert_eq!(value["outcome"]["behavior_verified"], false);
    assert!(value["outcome"]["goal_achieved"].is_null());
    let requests = server.join().unwrap();
    assert_eq!(requests[1].1["goal_achieved"], Value::Null);
}
