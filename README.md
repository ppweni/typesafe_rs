# typesafe_rs

An async Rust client for the [TypeSafe AI API](https://docs.typesafe.ai/), based on
the `typesafe-sdk` Python SDK 0.6.0. Uses Tokio, reqwest with Rustls, and Serde.

```no_run
use typesafe_rs::{Client, Choice, Noul, Score, json};

#[tokio::main]
async fn main() -> typesafe_rs::Result<()> {
    // Reads TYPESAFE_API_KEY from the environment.
    let client = Client::new()?;
    let response = client
        .system_one(json!({"document": "I was charged twice. Please fix this ASAP."}))
        .question("billing", Noul::new("Is this ticket about billing?"))
        .question("tone", Choice::new(["calm", "frustrated", "angry"])
            .instructions("What is the customer's tone?"))
        .question("urgency", Score::new(["can wait", "this week", "today"])
            .instructions("How urgent is this ticket?"))
        .send()
        .await?;

    if let Some(answer) = response.choice("tone") {
        println!("Tone: {} (confidence: {})", answer.choice, answer.confidence);
    }
    println!("Request ID: {:?}", response.request_id());

    for model in &client.models().list().send().await?.models {
        println!("{}: {}", model.name, model.description);
    }
    Ok(())
}
```

Add this crate as a path dependency while developing, and enable Tokio's runtime:

```toml
[dependencies]
typesafe_rs = { path = "../typesafe_rs" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Clone and reuse `Client` across tasks: clones share configuration and the connection
pool. Calls take `&self`; each call has its own retry state. Dropping a request
future cancels its pending I/O or retry sleep. No explicit close is needed.

Client configuration uses `Client::builder()` with `api_key`, `base_url`, `model`,
`timeout`, `retry`, `headers`, and `http_client`. Explicit values take precedence
over `TYPESAFE_API_KEY`, `TYPESAFE_BASE_URL`, and `TYPESAFE_DEFAULT_MODEL`;
blank environment values are ignored. Defaults are `https://api.typesafe.ai`,
`jev-latest`, and a 10-second timeout per attempt, including the response body.
Gateway path prefixes in base URLs are preserved.

Both request builders accept `timeout`, `retry`, and `headers` overrides. System One
also accepts `model`, `questions`, and `extra_body`. State can be text, JSON, or any
Serde-serializable struct or array. Question instructions and criteria use `Content`
(text, object, or array); use `Content::try_from(json!(...))?` for structured content.
`Noul::default()` omits instructions. Choice descriptions can be supplied through
the public `criteria` map, and `NoulCriteria` describes yes/no outcomes.
`Question::Raw` supports future question kinds and additional fields.

`RetryPolicy::default()` retries twice for connection errors, timeouts, HTTP 408,
429, and 5xx. Backoff starts at 500 ms, doubles up to 5 seconds, and subtracts up to
25% jitter. `Retry-After` (seconds or HTTP date) and `retry-after-ms` override backoff.
The 30-second retry budget stops new attempts; it does not interrupt an in-flight
attempt. Use `tokio::time::timeout` around `send()` for a hard deadline, or
`RetryPolicy::disabled()` to disable retries. Retrying an inference POST can repeat
server work if its first response was lost.

`Response<T>` dereferences to its typed data and retains status, headers, raw bytes,
and an optional request ID. `noul`, `choice`, and `score` look up borrowed answers;
`nouls()`, `choices()`, and `scores()` iterate without building additional maps.
Unknown response fields are ignored. Future answer kinds become `Answer::Unknown`,
with their original payload available in the raw response body. Score legends and
probability maps use integer keys.

Errors distinguish invalid configuration/input, serialization, timeouts, transport,
API status failures, and malformed success responses. API errors expose `kind()`,
`status`, `request_id()`, `retry_after()`, headers, and the body. Response validation
errors include the Serde field path and original response. They are never retried.

Authentication, JSON accept/content type, and SDK headers override custom headers.
The retry count header starts at zero. The default HTTP client does not follow
redirects; a supplied reqwest client's redirect policy remains in effect. The SDK
applies its own timeout even with a supplied client. Client/builder debug output
omits API keys and custom headers. Request and response bodies are not logged.
`extra_body` is deliberately a last-write-wins, shallow-merge escape hatch, including
for `state`, `model`, and `questions`, as in the Python SDK.

Run `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check`.
Tests use local HTTP fixtures; no API key or paid inference is needed.

The workspace also contains [TypeParanoid](examples/type-paranoid/README.md), a
standalone binary example with embedded Rust packet capture and optional TypeSafe
assessment of unusual outbound transfers. Build it with `cargo build -p type-paranoid`;
run all SDK and example tests with `cargo test --workspace`.
