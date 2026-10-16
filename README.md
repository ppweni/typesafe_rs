# typesafe_rs

An async Rust client for the [TypeSafe AI API](https://docs.typesafe.ai/), based on
the `typesafe-sdk` Python SDK 0.7.5 API, including experimental Jev 1.14 vision.
Uses Tokio, reqwest with Rustls, and Serde.

1. Add the Git dependency and Tokio to your application's `Cargo.toml`. Cargo
   downloads the repository for you; you do not need to clone it:

   ```toml
   [dependencies]
   typesafe_rs = { git = "https://github.com/ppweni/typesafe_rs.git", branch = "main" }
   tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
   ```

   To pin a specific revision, replace `branch = "main"` with `rev = "<commit SHA>"`.
   Commit your application's `Cargo.lock` to retain the selected revision. To
   update it later, run `cargo update -p typesafe_rs`.

2. Create an API key in the [TypeSafe console](https://console.typesafe.ai/) and
   export it in the shell where you run your application:

   ```sh
   export TYPESAFE_API_KEY='your-api-key'
   ```

   For an explicit model, also set `TYPESAFE_DEFAULT_MODEL=jev-1.14.0`, or use
   `.model(JEV_1_14)` on the client builder or request. `jev-latest` is the default.

3. Put this example in `src/main.rs`, then run `cargo run`:

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

The Rust SDK and Python SDK have independent version numbers. The Python version
below identifies the API contract used as the implementation baseline:

| Rust SDK version | Python SDK baseline | Supported Jev features |
| --- | --- | --- |
| `0.1.0` | `0.6.0` | Text: Noul, Choice, Score |
| `0.2.0` (this revision) | `0.7.5` | Text, image assets, multipart/base64 uploads, Box, zstd |

| Model ID | Noul / Choice / Score | Images | Box |
| --- | --- | --- | --- |
| `jev-1.13.0` | Yes | Experimental | No |
| `jev-1.14.0` (`JEV_1_14`) | Yes | Experimental | Experimental |

Both models support the long-context header. Image/Box requests require
`.images(true)`. As of October 16, 2026, the documented `jev-latest` alias points
to 1.13 and `jev-preview` to 1.14; aliases move, so pin a model ID when behavior
must stay stable. See the [official model list](https://docs.typesafe.ai/models).

For local development against your own checkout, use
`typesafe_rs = { path = "../typesafe_rs" }` instead of the Git dependency.

Clone and reuse `Client` across tasks: clones share configuration and the connection
pool. Calls take `&self`; each call has its own retry state. Dropping a request
future cancels its pending I/O or retry sleep. No explicit close is needed.

Client configuration uses `Client::builder()` with `api_key`, `base_url`, `model`,
`timeout`, `retry`, `headers`, `http_client`, and `compression`. Explicit values
take precedence over `TYPESAFE_API_KEY`, `TYPESAFE_BASE_URL`, and `TYPESAFE_DEFAULT_MODEL`;
blank environment values are ignored. Defaults are `https://api.typesafe.ai`,
`jev-latest`, and a 10-second timeout per attempt, including the response body.
Gateway path prefixes in base URLs are preserved. API keys are trimmed and must
contain printable ASCII without internal whitespace.

Both request builders accept `timeout`, `retry`, and `headers` overrides. System One
also accepts `model`, `questions`, and `extra_body`. State can be text, JSON, or any
Serde-serializable struct or array. Question instructions and criteria use `Content`
(text, object, or array); use `Content::try_from(json!(...))?` for structured content.
`Noul::default()` omits instructions and requires a described yes/no outcome in
`criteria`. Choice requires 1–255 options; Score requires 1–10 levels. Content
accepts at most 64 nested object/array levels. Choice descriptions can be supplied
through the public `criteria` map, and `NoulCriteria` describes yes/no outcomes.
`Question::Raw` supports future question kinds and additional fields.

`RetryPolicy::default()` retries twice for connection errors, timeouts, HTTP 408,
429, and 5xx. Backoff starts at 500 ms, doubles up to 5 seconds, and subtracts up to
25% jitter. `Retry-After` (seconds or HTTP date) and `retry-after-ms` override backoff.
The 30-second retry budget stops new attempts; it does not interrupt an in-flight
attempt. Use `tokio::time::timeout` around `send()` for a hard deadline, or
`RetryPolicy::disabled()` to disable retries. Retrying an inference POST can repeat
server work if its first response was lost.

`Response<T>` dereferences to its typed data and retains status, headers, raw bytes,
and an optional request ID. `noul`, `choice`, `score`, and `box_answer` look up
borrowed answers; `nouls()`, `choices()`, `scores()`, and `boxes()` iterate without
building additional maps.
Unknown response fields are ignored. Future answer kinds become `Answer::Unknown`,
with their original payload available in the raw response body. Score legends and
probability maps use integer keys.

Errors distinguish invalid configuration/input, image file reads, compression,
serialization, timeouts, transport, API status failures, and malformed success
responses. API errors expose `kind()`,
`status`, `request_id()`, `retry_after()`, headers, and the body. Response validation
errors include the Serde field path and original response. They are never retried.
HTTP 413 and 415 have dedicated `PayloadTooLarge` and `UnsupportedMediaType` kinds.

Authentication, accept/content type, content encoding, and SDK headers override
custom headers. A supplied HTTP client should not set encoding or content-length
defaults: the SDK computes these from the encoded body.
The retry count header starts at zero. The default HTTP client does not follow
redirects; a supplied reqwest client's redirect policy remains in effect. The SDK
applies its own timeout even with a supplied client. Client/builder debug output
omits API keys and custom headers. Request and response bodies are not logged.
`extra_body` is deliberately a last-write-wins, shallow-merge escape hatch, including
for `state`, `model`, `questions`, and `assets`, as in the Python SDK. A manual
`assets` field replaces registered images and prevents their files from being read.

Images use `Image::from_bytes`, `Image::from_file`, or `Image::from_base64`. Register
each image with `.asset(image.clone())`; use its `reference()` or `Display` value
inside any string, or serialize the `Image` itself in a struct or with `json!`.
Images also convert to `Content` for question instructions and criteria. Registration
is explicit: serializing a reference alone does not upload its bytes. `.assets(...)`
accepts a map or iterator of your own reference strings and images. Use long, unique
references; the API matches them inside strings, never in object keys.

```no_run
use typesafe_rs::{BoxQuestion, Client, Image, JEV_1_14, Noul, json};

# async fn example() -> typesafe_rs::Result<()> {
let client = Client::new()?;
let photo = Image::from_file("photo.png");
let response = client
    .system_one(json!({"photo": photo}))
    .asset(photo.clone())
    .model(JEV_1_14)
    .images(true)
    .question("present", Noul::new("Is there a mug in `photo`?"))
    .question("mug", BoxQuestion::new("The mug in `photo`"))
    .send()
    .await?;

if response.noul("present").is_some_and(|answer| answer.noul > 0.8) {
    if let Some(answer) = response.box_answer("mug") {
        println!("Edges: {:?}; confidence: {:?}", answer.coordinates, answer.confidence);
    }
}
# Ok(())
# }
```

Registered images use multipart uploads by default. `.asset_encoding(AssetEncoding::Base64)`
sends base64 in JSON instead. PNG/JPEG bytes are uploaded unchanged. Files are read
asynchronously on first use and cached across clones and calls; recreate an `Image`
to send changed file contents. Byte-backed images share immutable `Bytes`. Failed
file reads can be retried on the same image. Debug output excludes file paths and bytes.

Images and Box are experimental and require `.images(true)` (the
`TypeSafe-Danger-Zone-Images: true` header). Images work on Jev 1.13 and 1.14; Box
requires `jev-1.14.0` (`JEV_1_14`) or a suitable preview alias. Pin the version when
using Box: `jev-latest` is a moving alias and may still point to 1.13.
Box returns one normalized rectangle per question, with nullable confidence.
`coordinates` maps to the wire field `box`. There is no "not found" result; pair
Box with a Noul asking whether the target is present.

Prepare still PNG/JPEG images with each side between 280 and 1568 pixels. The API
checks dimensions and image validity; the client checks signatures and the 16 MiB
file limit. The request body must fit 32 MiB before and after compression. Server
processing and repeated references impose further 32 MiB image/expanded-content
limits. The API does not resize images or apply EXIF orientation; prepare pixels
before sending. Reference keys are limited to 1024 UTF-8 bytes each and 65536 combined.
Every registered image must be referenced in state, instructions, or criteria.

Use `.long_context(true)` for the experimental 262k request/question context limit.
Both `.images(bool)` and `.long_context(bool)` work on the client builder and the
inference request; request values override client defaults. A `false` override
explicitly turns a client default off.

Zstd request compression is enabled by the default `zstd` Cargo feature. Bodies
of at least 1024 bytes are compressed once and reused for retries, including
multipart bodies. `Compression::Auto` is the default; `Compression::Disabled`
sends uncompressed data, and `Compression::Zstd` requires the Cargo feature.
Configure it on the client builder or per inference request. Use
`default-features = false` to omit zstd entirely; `Auto` then sends plain bodies.
Gateways that do not support compressed requests can use `Disabled`.

Application-specific response schemas use `.send_as::<T>()`, where `T` implements
`serde::de::DeserializeOwned`. The SDK retains the same status, headers, body and
request ID, and reports schema errors with field paths. Define the actual wire
shape, including the nested `answers` object:

```no_run
use serde::Deserialize;
use typesafe_rs::{Client, Noul, NoulAnswer};

#[derive(Deserialize)]
struct Answers { billing: NoulAnswer }
#[derive(Deserialize)]
struct Decision { model: String, answers: Answers }

# async fn example() -> typesafe_rs::Result<()> {
let response = Client::new()?
    .system_one("I was charged twice.")
    .question("billing", Noul::new("Is this about billing?"))
    .send_as::<Decision>()
    .await?;
println!("{}: {}", response.model, response.answers.billing.noul);
# Ok(())
# }
```

Run the full offline fixture suite and lint checks:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test -p typesafe_rs --no-default-features
```

Tests use local HTTP fixtures; the default suite needs no API key. Six live contract
tests are ignored by default. They check model listing, text primitives, custom
response schemas, file/multipart and base64 vision/Box, and zstd/long-context headers.
To run them, export `TYPESAFE_API_KEY` in your shell, then:

```sh
cargo test --test live -- --ignored
```

These calls use the configured API endpoint and make billable inference requests.
For a coverage report, install `cargo-llvm-cov` and run
`cargo llvm-cov -p typesafe_rs --all-features --summary-only`.

The workspace also contains [TypeParanoid](examples/type-paranoid/README.md), a
standalone binary example with embedded Rust packet capture and optional TypeSafe
assessment of unusual outbound transfers. Build it with `cargo build -p type-paranoid`;
run all SDK and example tests with `cargo test --workspace`.
