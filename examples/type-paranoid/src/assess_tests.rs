use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

use super::*;
use crate::engine::{Config, Engine, Observation, Protocol};

fn window() -> Window {
    let mut engine = Engine::new(Config::default());
    engine.observe(Observation {
        peer: Peer {
            remote: "198.51.100.2:8443".parse().unwrap(),
            protocol: Protocol::Tcp,
        },
        outbound: true,
        payload_bytes: 30_000_000,
    });
    engine.finish(10.0)
}

fn server(response: serde_json::Value) -> (Client, thread::JoinHandle<serde_json::Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let task = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let end = loop {
            let mut chunk = [0; 4096];
            let n = socket.read(&mut chunk).unwrap();
            assert_ne!(n, 0);
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&bytes[..end]);
        assert!(headers.starts_with("POST /v1/systemone HTTP/1.1"));
        let length: usize = headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
            .unwrap()
            .1
            .trim()
            .parse()
            .unwrap();
        while bytes.len() < end + length {
            let mut chunk = [0; 4096];
            let n = socket.read(&mut chunk).unwrap();
            assert_ne!(n, 0);
            bytes.extend_from_slice(&chunk[..n]);
        }
        let request = serde_json::from_slice(&bytes[end..end + length]).unwrap();
        let body = response.to_string();
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nx-typesafe-request-id: assessment-1\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        request
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let client = Client::builder()
        .api_key("test-key")
        .base_url(format!("http://{address}"))
        .http_client(http)
        .retry(typesafe_rs::RetryPolicy::disabled())
        .build()
        .unwrap();
    (client, task)
}

#[tokio::test]
async fn submits_candidate_metadata_and_decodes_structured_assessment() {
    let (client, server) = server(json!({"model": "test", "usage": {}, "answers": {
        "behavior_0": {"type": "choice", "choice": "unusual", "confidence": 0.8, "probabilities": {"unusual": 0.8}},
        "priority_0": {"type": "score", "score": 2.5, "confidence": 0.8, "legend": {"0": "low", "3": "high"}, "probabilities": {"2": 0.5, "3": 0.5}}
    }}));
    let (results, id) = assess(&client, &window()).await.unwrap();
    assert_eq!(results[0].classification, "unusual");
    assert_eq!(results[0].review_priority, 2.5);
    assert_eq!(id.as_deref(), Some("assessment-1"));
    let request = server.join().unwrap();
    assert_eq!(
        request["state"]["window"]["candidates"][0]["peer"]["remote"],
        "198.51.100.2:8443"
    );
    assert!(request["state"]["window"]["candidates"][0]["process"].is_null());
    assert_eq!(request["questions"]["behavior_0"]["type"], "choice");
    assert_eq!(
        request["questions"]["priority_0"]["criteria"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[tokio::test]
async fn incomplete_assessment_is_an_error_instead_of_a_clean_bill_of_health() {
    let (client, server) = server(json!({"model": "test", "usage": {}, "answers": {}}));
    assert!(
        assess(&client, &window())
            .await
            .unwrap_err()
            .to_string()
            .contains("missing classification")
    );
    server.join().unwrap();
}
