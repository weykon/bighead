use super::*;

fn request() -> Value {
    json!({"goal":"Set an existing counter to 1", "state":{"count":0},
        "candidates":[{"operation":"modify_component","description":"Update existing component data"}]})
}

fn response() -> Value {
    json!({"protocol_version":1,"decision_id":"test-decision","model":"test-model",
        "revision":"test-revision","device":"cpu","mode":"advisory", "executed":false,
        "behavior_verified":false,"operation":"modify_component","confidence":0.9,
        "confidence_calibrated_for_ecs":false,"input_truncated":false,
        "probabilities":{"modify_component":0.9,"defer":0.1}})
}

fn evaluation_request() -> Value {
    json!({"spec_version":2,"spec_id":"counter.v1","spec_hash":"a".repeat(64),
        "state":{"count":0},"questions":{
            "action":{"type":"choice","instructions":"Choose an action",
                "criteria":{"change":"update it","defer":"wait"}},
            "ready":{"type":"noul","instructions":"Is the evidence sufficient?"},
            "risk":{"type":"score","instructions":"How risky is this?",
                "criteria":["low","high"]}}})
}

fn evaluation_response() -> Value {
    json!({"protocol_version":2,"decision_id":"decision-v2","spec_id":"counter.v1",
        "spec_hash":"a".repeat(64),"model":"test-model","revision":"test-revision",
        "device":"cpu","mode":"advisory","executed":false,"behavior_verified":false,
        "confidence_calibrated_for_ecs":false,"input_truncated":false,"answers":{
            "action":{"type":"choice","choice":"change","confidence":0.9,
                "probabilities":{"change":0.9,"defer":0.1}},
            "ready":{"type":"noul","noul":0.95,"confidence":0.8},
            "risk":{"type":"score","score":0.2,"confidence":0.8,
                "probabilities":{"0":0.8,"1":0.2}}}})
}

fn serve_once(
    response_status: &'static str,
    response_body: Value,
) -> (String, std::thread::JoinHandle<(String, Value)>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let (path, body) = loop {
            let mut block = [0; 1024];
            let n = socket.read(&mut block).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&block[..n]);
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..end]);
                let size: usize = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .unwrap()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + size {
                    let path = header
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .to_owned();
                    let body = serde_json::from_slice(&bytes[end + 4..end + 4 + size]).unwrap();
                    break (path, body);
                }
            }
        };
        let response_body = response_body.to_string();
        write!(socket,"HTTP/1.1 {response_status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",response_body.len()).unwrap();
        (path, body)
    });
    (format!("http://{address}"), thread)
}

#[test]
fn laya_candidates_respect_available_tools() {
    let available = big_head_ai::ecs_tool_definitions();
    assert!(validate(&request(), &available).is_ok());
    assert!(validate(&request(), &[]).is_err());
    let mut duplicate = request();
    duplicate["candidates"]
        .as_array_mut()
        .unwrap()
        .push(request()["candidates"][0].clone());
    assert!(validate(&duplicate, &available).is_err());
    let mut large = request();
    large["state"] = json!({"text":"x".repeat(8193)});
    assert!(validate(&large, &available).is_err());
}

#[test]
fn laya_rejects_remote_endpoints_and_untrusted_predictions() {
    for url in [
        "https://example.com",
        "http://localhost:3031",
        "http://127.0.0.1:3031/?secret=x",
        "http://u:p@127.0.0.1:3031",
    ] {
        assert!(endpoint(url).is_err());
    }
    assert_eq!(endpoint("http://127.0.0.1:3031").unwrap().path(), "/decide");
    let choices = validate(&request(), &big_head_ai::ecs_tool_definitions()).unwrap();
    assert!(validate_response(&response(), &choices).is_ok());
    for (key, value) in [
        ("operation", json!("despawn_entity")),
        ("executed", json!(true)),
        ("input_truncated", json!(true)),
        ("probabilities", json!({"modify_component":0.2,"defer":0.1})),
    ] {
        let mut invalid = response();
        invalid[key] = value;
        assert!(validate_response(&invalid, &choices).is_err());
    }
}

#[tokio::test]
async fn laya_http_roundtrip_and_local_error() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    for (status, body) in [
        ("200 OK", response()),
        (
            "400 Bad Request",
            json!({"error":"state exceeds model context"}),
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut block = [0; 1024];
                let n = socket.read(&mut block).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&block[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let size: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + size {
                        assert!(header.starts_with("post /decide "));
                        assert_eq!(
                            serde_json::from_slice::<Value>(&bytes[end + 4..]).unwrap(),
                            request()
                        );
                        break;
                    }
                }
            }
            let body = body.to_string();
            write!(socket,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        });
        let result = decide_at(
            &request(),
            &big_head_ai::ecs_tool_definitions(),
            &format!("http://{address}"),
        )
        .await;
        thread.join().unwrap();
        if status.starts_with("200") {
            assert_eq!(result.unwrap()["executed"], false);
        } else {
            assert!(result.unwrap_err().contains("state exceeds model context"));
        }
    }
}

#[tokio::test]
async fn laya_v2_evaluate_and_outcome_http_contract() {
    let request = evaluation_request();
    let (base, server) = serve_once("200 OK", evaluation_response());
    let result = evaluate_at(&request, &base).await.unwrap();
    let (path, received) = server.join().unwrap();
    assert_eq!(path, "/evaluate");
    assert_eq!(received, request);
    assert_eq!(result["answers"]["action"]["choice"], "change");

    let outcome = json!({"decision_id":"decision-v2","spec_id":"counter.v1",
        "spec_hash":"a".repeat(64),"action_id":"change","execution_attempted":true,
        "execution_succeeded":true,"behavior_verified":true,"goal_achieved":true,
        "verification":[],"evidence":{}});
    let (base, server) = serve_once(
        "200 OK",
        json!({"recorded":true,"decision_id":"decision-v2","training_eligible":false}),
    );
    assert!(submit_outcome_at(&outcome, &base).await.unwrap()["recorded"] == true);
    let (path, received) = server.join().unwrap();
    assert_eq!(path, "/outcome");
    assert_eq!(received, outcome);
}

#[test]
fn laya_v2_rejects_mismatched_or_malformed_answers() {
    let request = evaluation_request();
    let mut invalid = evaluation_response();
    invalid["answers"].as_object_mut().unwrap().remove("ready");
    assert!(validate_evaluation_response(&invalid, &request).is_err());

    let mut invalid = evaluation_response();
    invalid["answers"]["action"]["probabilities"] = json!({"change":0.8,"defer":0.8});
    assert!(validate_evaluation_response(&invalid, &request).is_err());

    let mut invalid = evaluation_response();
    invalid["answers"]["risk"]["type"] = json!("choice");
    assert!(validate_evaluation_response(&invalid, &request).is_err());
}

#[tokio::test]
#[ignore = "Requires the real local Laya service on port 3031"]
async fn laya_live_service() {
    let result = decide_at(
        &request(),
        &big_head_ai::ecs_tool_definitions(),
        "http://127.0.0.1:3031",
    )
    .await
    .unwrap();
    println!("{result}");
    assert_eq!(result["executed"], false);
    assert_eq!(result["input_truncated"], false);
}
