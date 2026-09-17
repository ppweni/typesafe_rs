use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};
use typesafe_rs::{
    Answer, ApiErrorKind, Choice, Client, Content, Error, Noul, NoulCriteria, Question,
    RetryPolicy, Score, Value, header::HeaderMap, json,
};

#[derive(Clone)]
struct Reply {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
    body_delay: Duration,
    disconnect: bool,
}
impl Reply {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: body.to_string(),
            headers: vec![],
            body_delay: Duration::ZERO,
            disconnect: false,
        }
    }
    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

#[derive(Debug)]
struct Captured {
    line: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

struct Server {
    url: String,
    count: Arc<AtomicUsize>,
    requests: mpsc::UnboundedReceiver<Captured>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let (tx, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let number = seen.fetch_add(1, Ordering::SeqCst);
                        let reply = replies[number.min(replies.len() - 1)].clone();
                        let tx = tx.clone();
                        connections.spawn(async move {
                            let mut bytes = Vec::new();
                            let header_end = loop {
                                let mut chunk = [0; 4096];
                                let n = socket.read(&mut chunk).await.unwrap();
                                if n == 0 { return; }
                                bytes.extend_from_slice(&chunk[..n]);
                                if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") { break pos + 4; }
                            };
                            let text = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                            let mut lines = text.lines();
                            let line = lines.next().unwrap().to_owned();
                            let headers: BTreeMap<_, _> = lines.filter_map(|line| line.split_once(':'))
                                .map(|(k, v)| (k.to_lowercase(), v.trim().to_owned())).collect();
                            let length: usize = headers.get("content-length").map(|v| v.parse().unwrap()).unwrap_or(0);
                            while bytes.len() < header_end + length {
                                let mut chunk = [0; 4096];
                                let n = socket.read(&mut chunk).await.unwrap();
                                if n == 0 { return; }
                                bytes.extend_from_slice(&chunk[..n]);
                            }
                            let _ = tx.send(Captured { line, headers, body: bytes[header_end..header_end + length].to_vec() });
                            if reply.disconnect { return; }
                            let mut response = format!("HTTP/1.1 {} Test\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n", reply.status, reply.body.len());
                            for (name, value) in reply.headers { response.push_str(&format!("{name}: {value}\r\n")); }
                            response.push_str("\r\n");
                            if socket.write_all(response.as_bytes()).await.is_err() { return; }
                            tokio::time::sleep(reply.body_delay).await;
                            let _ = socket.write_all(reply.body.as_bytes()).await;
                        });
                    }
                    Some(result) = connections.join_next() => { result.unwrap(); }
                }
            }
        });
        Self {
            url,
            count,
            requests,
            task,
        }
    }
    fn client(&self) -> Client {
        Client::builder()
            .api_key("test-key")
            .base_url(&self.url)
            .model("test-model")
            .retry(RetryPolicy::disabled())
            .build()
            .unwrap()
    }
    fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
    async fn request(&mut self) -> Captured {
        tokio::time::timeout(Duration::from_secs(2), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
}

fn models() -> Value {
    json!({"models": [{"name": "jev-latest", "description": "Default model", "release_date": "2026-01-01"}]})
}
fn inference() -> Value {
    json!({
        "model": "test-model", "usage": {"input_tokens": 12}, "future_field": true,
        "answers": {
            "billing": {"type": "noul", "noul": 0.95},
            "tone": {"type": "choice", "choice": "angry", "confidence": 0.9, "probabilities": {"calm": 0.1, "angry": 0.9}},
            "urgency": {"type": "score", "score": 0.8, "confidence": 0.8, "legend": {"0": "later", "1": {"text": "now"}}, "probabilities": {"0": 0.2, "1": 0.8}},
            "future": {"type": "future-kind", "new": [1, 2, 3]}
        }
    })
}
fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        backoff_initial: Duration::ZERO,
        ..RetryPolicy::default()
    }
}

#[tokio::test]
async fn system_one_wire_contract_and_borrowed_answers() {
    let mut server = Server::start(vec![
        Reply::json(200, inference()).header("x-typesafe-request-id", "req-123"),
    ])
    .await;
    let client = server.client();
    let response = client
        .system_one(json!({"document": "charged twice"}))
        .question(
            "billing",
            Noul::new("Billing?").criteria(NoulCriteria {
                yes: Some("yes".into()),
                no: None,
            }),
        )
        .question("tone", Choice::new(["calm", "angry"]).instructions("Tone?"))
        .question("urgency", Score::new(["later", "now"]))
        .send()
        .await
        .unwrap();
    assert_eq!(response.noul("billing").unwrap().noul, 0.95);
    assert_eq!(response.choice("tone").unwrap().choice, "angry");
    assert_eq!(response.score("urgency").unwrap().probabilities[&1], 0.8);
    assert!(matches!(
        response.score("urgency").unwrap().legend[&1],
        Content::Object(_)
    ));
    assert!(matches!(response.answers["future"], Answer::Unknown));
    assert_eq!(response.nouls().count(), 1);
    assert_eq!(response.choices().count(), 1);
    assert_eq!(response.scores().count(), 1);
    assert!(response.choice("billing").is_none());
    assert!(response.noul("missing").is_none());
    assert_eq!(response.usage.input_tokens, Some(12));
    assert_eq!(response.usage.output_tokens, None);
    assert_eq!(response.request_id(), Some("req-123"));
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap(),
        inference()
    );

    let request = server.request().await;
    assert_eq!(request.line, "POST /v1/systemone HTTP/1.1");
    assert_eq!(request.headers["authorization"], "Bearer test-key");
    assert_eq!(request.headers["content-type"], "application/json");
    assert_eq!(request.headers["accept"], "application/json");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({
            "state": {"document": "charged twice"}, "model": "test-model",
            "questions": {
                "billing": {"type": "noul", "instructions": "Billing?", "criteria": {"true": "yes"}},
                "tone": {"type": "choice", "instructions": "Tone?", "criteria": {"calm": null, "angry": null}},
                "urgency": {"type": "score", "criteria": ["later", "now"]}
            }
        })
    );
}

#[tokio::test]
async fn model_listing_prefix_headers_and_custom_http_client() {
    let mut server = Server::start(vec![Reply::json(200, models())]).await;
    let mut defaults = HeaderMap::new();
    defaults.insert("authorization", "wrong".parse().unwrap());
    defaults.insert("x-typesafe-retry-count", "99".parse().unwrap());
    let http = reqwest::Client::builder()
        .default_headers(defaults.clone())
        .build()
        .unwrap();
    defaults.insert("x-custom", "default".parse().unwrap());
    let client = Client::builder()
        .api_key("test-key")
        .base_url(format!("{}/gateway/", server.url))
        .headers(defaults)
        .http_client(http)
        .build()
        .unwrap();
    let mut overrides = HeaderMap::new();
    for (name, value) in [
        ("x-custom", "override"),
        ("authorization", "bad"),
        ("accept", "bad"),
        ("user-agent", "bad"),
        ("x-typesafe-sdk", "bad"),
        ("x-typesafe-runtime", "bad"),
    ] {
        overrides.insert(name, value.parse().unwrap());
    }
    let response = client
        .models()
        .list()
        .headers(overrides)
        .send()
        .await
        .unwrap();
    assert_eq!(response.models[0].name, "jev-latest");
    assert_eq!(response.request_id(), None);
    let request = server.request().await;
    assert_eq!(request.line, "GET /gateway/v1/models HTTP/1.1");
    assert!(request.body.is_empty());
    assert_eq!(request.headers["x-custom"], "override");
    assert_eq!(request.headers["authorization"], "Bearer test-key");
    assert_eq!(request.headers["accept"], "application/json");
    assert_eq!(request.headers["x-typesafe-retry-count"], "0");
    assert!(request.headers["x-typesafe-sdk"].starts_with("typesafe-rs/"));
    assert_eq!(request.headers["x-typesafe-runtime"], "rust");
}

#[tokio::test]
async fn retries_preserve_body_and_increment_attempt_headers() {
    let mut server = Server::start(vec![
        Reply::json(503, json!({"message": "busy"})),
        Reply::json(429, json!({"error": "slow down"})).header("retry-after-ms", "1"),
        Reply::json(200, inference()),
    ])
    .await;
    let response = server
        .client()
        .system_one("hello")
        .question("billing", Noul::default())
        .retry(fast_retry())
        .send()
        .await
        .unwrap();
    assert_eq!(response.model, "test-model");
    assert_eq!(server.count(), 3);
    let mut bodies = Vec::new();
    for attempt in 0..3 {
        let request = server.request().await;
        assert_eq!(
            request.headers["x-typesafe-retry-count"],
            attempt.to_string()
        );
        bodies.push(request.body);
    }
    assert!(bodies.windows(2).all(|b| b[0] == b[1]));
}

#[tokio::test]
async fn retry_budget_respects_server_delay_and_disabled_override() {
    let server = Server::start(vec![
        Reply::json(429, json!({"error": {"message": "slow down"}}))
            .header("retry-after-ms", "60000")
            .header("x-typesafe-request-id", "rate-123"),
    ])
    .await;
    let client = Client::builder()
        .api_key("test-key")
        .base_url(&server.url)
        .retry(fast_retry())
        .build()
        .unwrap();
    let Error::Api(error) = client.models().list().send().await.unwrap_err() else {
        panic!("expected API error")
    };
    assert_eq!(error.kind(), ApiErrorKind::RateLimit);
    assert_eq!(error.retry_after(), Some(Duration::from_secs(60)));
    assert_eq!(error.request_id(), Some("rate-123"));
    assert_eq!(error.message, "slow down");
    assert_eq!(server.count(), 1);
    client
        .models()
        .list()
        .retry(RetryPolicy::disabled())
        .send()
        .await
        .unwrap_err();
    assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn api_error_kinds_messages_and_no_retry_for_client_errors() {
    for (status, kind) in [
        (400, ApiErrorKind::BadRequest),
        (401, ApiErrorKind::Authentication),
        (403, ApiErrorKind::PermissionDenied),
        (404, ApiErrorKind::NotFound),
        (422, ApiErrorKind::UnprocessableEntity),
        (500, ApiErrorKind::InternalServer),
        (418, ApiErrorKind::Other),
    ] {
        let server = Server::start(vec![Reply::json(
            status,
            json!({"detail": [{"loc": ["body", "questions", "x"], "msg": "invalid"}]}),
        )])
        .await;
        let retry = if status < 500 {
            fast_retry()
        } else {
            RetryPolicy::disabled()
        };
        let Error::Api(error) = server
            .client()
            .models()
            .list()
            .retry(retry)
            .send()
            .await
            .unwrap_err()
        else {
            panic!("expected API error")
        };
        assert_eq!(error.kind(), kind);
        assert_eq!(error.status.as_u16(), status);
        assert_eq!(error.message, "questions.x: invalid");
        assert!(error.endpoint.ends_with("/v1/models"));
        assert_eq!(server.count(), 1);
    }
}

#[tokio::test]
async fn malformed_success_is_not_retried_and_retains_context() {
    let mut body = inference();
    body["answers"]["tone"]["confidence"] = json!("bad");
    let server = Server::start(vec![
        Reply::json(200, body.clone()).header("x-typesafe-request-id", "bad-123"),
    ])
    .await;
    let Error::ResponseValidation(error) = server
        .client()
        .system_one("hello")
        .question("x", Noul::default())
        .retry(fast_retry())
        .send()
        .await
        .unwrap_err()
    else {
        panic!("expected validation error")
    };
    assert!(
        error.field_path.contains("answers.tone"),
        "{}",
        error.field_path
    );
    assert_eq!(error.request_id(), Some("bad-123"));
    assert_eq!(serde_json::from_slice::<Value>(&error.body).unwrap(), body);
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn invalid_json_missing_fields_and_trailing_data_are_rejected() {
    for body in [
        "",
        "not json",
        "{}",
        "{\"models\": []} trailing",
        "{\"models\": [{\"name\": \"x\"}]}",
    ] {
        let mut reply = Reply::json(200, models());
        reply.body = body.into();
        let server = Server::start(vec![reply]).await;
        assert!(matches!(
            server.client().models().list().send().await,
            Err(Error::ResponseValidation(_))
        ));
    }
}

#[tokio::test]
async fn validation_happens_before_network_io() {
    let server = Server::start(vec![Reply::json(200, inference())]).await;
    let client = server.client();
    assert!(matches!(
        client.system_one("hello").send().await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        client
            .system_one(42)
            .question("x", Noul::default())
            .send()
            .await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        client
            .system_one("hello")
            .question("x", Score::new(Vec::<String>::new()))
            .send()
            .await,
        Err(Error::InvalidInput(_))
    ));
    for raw in [
        json!({}),
        json!({"type": ""}),
        json!({"type": "choice"}),
        json!({"type": "score", "criteria": []}),
    ] {
        assert!(matches!(
            client
                .system_one("hello")
                .question("x", Question::Raw(raw))
                .send()
                .await,
            Err(Error::InvalidInput(_))
        ));
    }
    assert!(matches!(
        client.models().list().timeout(Duration::ZERO).send().await,
        Err(Error::Configuration(_))
    ));
    assert!(matches!(
        client
            .models()
            .list()
            .retry(RetryPolicy {
                backoff_jitter: f64::NAN,
                ..fast_retry()
            })
            .send()
            .await,
        Err(Error::Configuration(_))
    ));
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn per_request_model_raw_questions_and_shallow_body_overrides() {
    let mut server = Server::start(vec![Reply::json(200, inference())]).await;
    let client = server.client();
    client
        .system_one("original")
        .question("x", Question::Raw(json!({"type": "future", "extra": true})))
        .model("override")
        .send()
        .await
        .unwrap();
    let request: Value = serde_json::from_slice(&server.request().await.body).unwrap();
    assert_eq!(request["model"], "override");
    assert_eq!(
        request["questions"]["x"],
        json!({"type": "future", "extra": true})
    );
    client
        .system_one(json!({"old": true}))
        .question("x", Noul::default())
        .model("override")
        .extra_body(
            json!({"state": {"new": true}, "model": "extra-model", "questions": {}, "extra": null})
                .as_object()
                .unwrap()
                .clone(),
        )
        .send()
        .await
        .unwrap();
    let request: Value = serde_json::from_slice(&server.request().await.body).unwrap();
    assert_eq!(
        request,
        json!({"state": {"new": true}, "model": "extra-model", "questions": {}, "extra": null})
    );
}

#[tokio::test]
async fn timeout_includes_body_read_and_can_be_overridden() {
    let mut slow = Reply::json(200, models());
    slow.body_delay = Duration::from_millis(100);
    let server = Server::start(vec![slow]).await;
    let client = Client::builder()
        .api_key("test-key")
        .base_url(&server.url)
        .timeout(Duration::from_millis(20))
        .retry(RetryPolicy::disabled())
        .build()
        .unwrap();
    assert!(matches!(
        client.models().list().send().await,
        Err(Error::Timeout(_))
    ));
    client
        .models()
        .list()
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .unwrap();
    assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn connection_failures_and_timeouts_retry_independently() {
    let mut broken = Reply::json(200, models());
    broken.disconnect = true;
    let server = Server::start(vec![broken.clone(), Reply::json(200, models())]).await;
    server
        .client()
        .models()
        .list()
        .retry(fast_retry())
        .send()
        .await
        .unwrap();
    assert_eq!(server.count(), 2);
    let server = Server::start(vec![broken]).await;
    assert!(matches!(
        server
            .client()
            .models()
            .list()
            .retry(RetryPolicy {
                api_connection_error: false,
                ..fast_retry()
            })
            .send()
            .await,
        Err(Error::Transport(_))
    ));
    assert_eq!(server.count(), 1);

    let mut slow = Reply::json(200, models());
    slow.body_delay = Duration::from_millis(150);
    let server = Server::start(vec![slow, Reply::json(200, models())]).await;
    server
        .client()
        .models()
        .list()
        .timeout(Duration::from_millis(50))
        .retry(fast_retry())
        .send()
        .await
        .unwrap();
    assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn concurrent_clones_do_not_share_attempt_counters() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Client>();
    let mut server = Server::start(vec![Reply::json(200, models())]).await;
    let client = server.client();
    let clone = client.clone();
    let (a, b) = tokio::join!(client.models().list().send(), clone.models().list().send());
    a.unwrap();
    b.unwrap();
    assert_eq!(server.count(), 2);
    assert_eq!(
        server.request().await.headers["x-typesafe-retry-count"],
        "0"
    );
    assert_eq!(
        server.request().await.headers["x-typesafe-retry-count"],
        "0"
    );
}

#[tokio::test]
async fn dropping_future_cancels_retry_sleep() {
    let server = Server::start(vec![
        Reply::json(503, json!({"message": "busy"})).header("retry-after-ms", "100"),
    ])
    .await;
    let client = server.client();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(30),
            client.models().list().retry(fast_retry()).send()
        )
        .await
        .is_err()
    );
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(server.count(), 1);
}

#[test]
fn configuration_validation_and_secret_redaction() {
    for key in ["", "  ", "secret\r\ninjected: value"] {
        let error = Client::builder().api_key(key).build().unwrap_err();
        assert!(matches!(error, Error::Configuration(_)));
        assert!(!format!("{error:?}").contains("injected"));
    }
    for url in [
        "not a url",
        "ftp://example.com",
        "https://user:secret@example.com",
        "https://example.com?key=secret",
        "https://example.com#secret",
    ] {
        let error = Client::builder()
            .api_key("test-key")
            .base_url(url)
            .build()
            .unwrap_err();
        assert!(matches!(error, Error::Configuration(_)));
        assert!(!format!("{error:?}").contains("secret"));
    }
    assert!(
        Client::builder()
            .api_key("test-key")
            .timeout(Duration::ZERO)
            .build()
            .is_err()
    );
    assert!(
        Client::builder()
            .api_key("test-key")
            .retry(RetryPolicy {
                timeout: Some(Duration::ZERO),
                ..fast_retry()
            })
            .build()
            .is_err()
    );
    let builder = Client::builder()
        .api_key("super-secret")
        .base_url("https://example.com")
        .model("model");
    assert!(!format!("{builder:?}").contains("super-secret"));
    assert!(!format!("{:?}", builder.build().unwrap()).contains("super-secret"));
}
