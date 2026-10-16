# typesafe_rs

An async Rust client for [TypeSafe AI's Jev](https://docs.typesafe.ai/): send state
and typed questions, then branch on structured answers. Built with Tokio, reqwest
with Rustls, and Serde. Supports text, image assets, and experimental Jev 1.14 Box
questions, based on the Python SDK 0.7.5 API contract.

- [Installation](#installation)
- [Quick start](#quick-start)
- [Question types](#question-types)
- [Images](#images)
- [Client configuration](#client-configuration)
- [Responses](#responses)
- [Retries and errors](#retries-and-errors)
- [Version compatibility](#version-compatibility)
- [Development and tests](#development-and-tests)

## Installation

Add the Git dependency and Tokio to your application's `Cargo.toml`. Cargo fetches
the repository automatically; you do not need to clone it:

```toml
[dependencies]
typesafe_rs = { git = "https://github.com/ppweni/typesafe_rs.git", branch = "main" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

For a fixed revision, replace `branch = "main"` with `rev = "<commit SHA>"` or a
published `tag`. Commit your application's `Cargo.lock` to keep that revision;
run `cargo update -p typesafe_rs` when you want to update it.

For development against a local checkout, use
`typesafe_rs = { path = "../typesafe_rs" }` instead.

## Quick start

1. Create an API key in the [TypeSafe console](https://console.typesafe.ai/).
2. Export it in the same terminal where you run your application.

   Bash or zsh:

   ```sh
   export TYPESAFE_API_KEY='your-api-key'
   ```

   Fish:

   ```fish
   set -gx TYPESAFE_API_KEY 'your-api-key'
   ```

3. Put this in `src/main.rs`, then run `cargo run`:

```no_run
use typesafe_rs::{Client, Noul};

#[tokio::main]
async fn main() -> typesafe_rs::Result<()> {
    let client = Client::new()?; // Reads TYPESAFE_API_KEY.
    let response = client
        .system_one("I was charged twice. Please fix this ASAP.")
        .question("billing", Noul::new("Is this ticket about billing?"))
        .send()
        .await?;

    if let Some(answer) = response.noul("billing") {
        println!("Probability of billing: {:.2}", answer.noul);
    }
    println!("Request ID: {:?}", response.request_id());
    Ok(())
}
```

Create a `Client` once and reuse it. Cloning it shares configuration and the HTTP
connection pool; requests take `&self` and can run concurrently. No explicit close
is needed. Request builders perform no network I/O until `.send().await`.

## Question types

| Type | Use it to | Answer |
| --- | --- | --- |
| `Noul` | Evaluate a yes/no statement | Probability of yes, from 0 to 1 |
| `Choice` | Select one named option | Selected label, confidence, probabilities |
| `Score` | Rate something against ordered levels | Expected score, confidence, legend, probabilities |
| `BoxQuestion` | Locate one target in an image | Normalized box edges, nullable confidence |

### Noul: a yes/no judgment

Use `Noul` when your code needs a probability for a statement. Choose the threshold
in your application:

```no_run
use typesafe_rs::{Client, Noul};

# async fn example() -> typesafe_rs::Result<()> {
let client = Client::new()?;
let response = client
    .system_one("Checkout is broken for every customer.")
    .question("urgent", Noul::new("Does this need immediate attention?"))
    .send()
    .await?;

if let Some(answer) = response.noul("urgent") {
    if answer.noul > 0.8 {
        println!("Escalate to the on-call team.");
    }
}
# Ok(())
# }
```

Use `NoulCriteria` to describe the yes/no outcomes. `Noul::default()` omits
instructions, so it requires at least one described outcome in `criteria`.

### Choice: choose an option

The labels in `Choice::new` become the possible answers. A Choice needs 1–255
options; descriptions can be supplied through its public `criteria` map.

```no_run
use typesafe_rs::{Choice, Client};

# async fn example() -> typesafe_rs::Result<()> {
let client = Client::new()?;
let response = client
    .system_one("I was charged twice and need a refund.")
    .question(
        "department",
        Choice::new(["billing", "technical", "sales"])
            .instructions("Which department should handle this ticket?"),
    )
    .send()
    .await?;

if let Some(answer) = response.choice("department") {
    println!("Route to: {}", answer.choice);
    println!("Confidence: {:.2}", answer.confidence);
    println!("All probabilities: {:?}", answer.probabilities);
}
# Ok(())
# }
```

### Score: rate against a rubric

Score levels are ordered from zero. A Score needs 1–10 levels; two or more make a
useful rubric. The result is a probability-weighted value and can fall between levels.

```no_run
use typesafe_rs::{Client, Score};

# async fn example() -> typesafe_rs::Result<()> {
let client = Client::new()?;
let response = client
    .system_one("The customer cannot access their account and has a deadline today.")
    .question(
        "urgency",
        Score::new(["Can wait", "Needs attention this week", "Needs attention today"])
            .instructions("How urgent is this ticket?"),
    )
    .send()
    .await?;

if let Some(answer) = response.score("urgency") {
    println!("Urgency: {:.2} out of 2", answer.score);
    println!("Confidence: {:.2}", answer.confidence);
    println!("Rubric: {:?}", answer.legend);
}
# Ok(())
# }
```

`legend` and `probabilities` use integer keys corresponding to the level indices.

### Box: locate a target in an image

Box requires Jev 1.14 and the experimental images header. **Call `.images(true)`
explicitly:** adding an image or Box question does not turn it on automatically.

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
        let edges = answer.coordinates;
        println!("Left: {}; right: {}", edges.left, edges.right);
        println!("Top: {}; bottom: {}", edges.top, edges.bottom);
        println!("Confidence: {:?}", answer.confidence);
    }
}
# Ok(())
# }
```

The coordinates are fractions of the image's width/height, measured from the
top-left corner. Multiply them by pixel dimensions to draw or crop the box.
`coordinates` maps to the wire field `box`; confidence may be `None`.

Each Box question returns one rectangle, even if the target is absent. Pair it
with a Noul asking whether the target exists, as above. Images and Box are
experimental; see [TypeSafe's image docs](https://docs.typesafe.ai/images).

### Multiple questions and structured content

Mix question types in the same request; the answers keep the names you assign:

```no_run
use typesafe_rs::{Choice, Client, Noul, Score, json};

# async fn example() -> typesafe_rs::Result<()> {
let response = Client::new()?
    .system_one(json!({"document": "I was charged twice. Please fix this ASAP."}))
    .question("billing", Noul::new("Is this about billing?"))
    .question("tone", Choice::new(["calm", "frustrated", "angry"]))
    .question("urgency", Score::new(["Can wait", "This week", "Today"]))
    .send()
    .await?;

for (name, answer) in &response.answers {
    println!("{name}: {answer:?}");
}
# Ok(())
# }
```

State can be text, JSON, or any Serde-serializable struct or array. Instructions
and criteria use `Content`: text, an object, an array, or an image. Convert JSON
with `Content::try_from(json!(...))?`. Content accepts up to 64 nested object/array
levels. `Question::Raw` supports future question kinds and additional wire fields.

## Images

### Creating and registering images

Images can come from files, bytes, or base64:

```no_run
use typesafe_rs::Image;

# fn example(png_bytes: Vec<u8>, base64_text: &str) -> typesafe_rs::Result<()> {
let from_file = Image::from_file("photo.png");
let from_bytes = Image::from_bytes(png_bytes)?;
let from_base64 = Image::from_base64(base64_text)?;
# Ok(())
# }
```

Register each image with `.asset(image.clone())` and enable `.images(true)` on the
request or client builder. Registration and the experimental opt-in are both
explicit. Use the image directly in a serializable struct or `json!`, convert it
to `Content`, or put its `reference()` / `Display` value inside a string.

Images can appear in state, instructions, and criteria. Registering the same image
again is deduplicated. `.assets(...)` accepts a map or iterator of your own reference
strings and images; every registered image must appear in content. Use long, unique
references because the API matches them inside string values, never object keys.

Files are read asynchronously on first use and cached across clones and calls.
Recreate an `Image` to send changed file contents. Failed reads can be retried on
the same image. Byte-backed images share immutable `Bytes`; debug output excludes
paths and image bytes.

### Upload formats and limits

Multipart uploads are the default. To send base64 in JSON, add
`.asset_encoding(AssetEncoding::Base64)` to the request. This changes the upload
format; `Image::from_base64` only changes how you construct an image.

| Limit | Value |
| --- | --- |
| Image formats | Still PNG or JPEG |
| Width and height | Each side between 280 and 1568 pixels |
| Image file | Up to 16 MiB |
| Request body | Up to 32 MiB, before and after compression |
| Reference strings | Up to 1024 UTF-8 bytes each; 65536 combined |

The client checks signatures, file sizes, body sizes, and references; the API
checks decoding and dimensions. Server processing and repeated references impose
further 32 MiB image/expanded-content limits. The API does not resize images or
apply EXIF orientation; prepare the pixels before sending.

## Client configuration

### Defaults and environment variables

| Setting | Environment variable | Default |
| --- | --- | --- |
| API key | `TYPESAFE_API_KEY` | Required |
| API root | `TYPESAFE_BASE_URL` | `https://api.typesafe.ai` |
| Model | `TYPESAFE_DEFAULT_MODEL` | `jev-latest` |
| Timeout per attempt | — | 10 seconds, including the response body |
| Compression | — | Zstd when the Cargo feature is enabled |

Explicit builder values override environment variables; blank environment values
are ignored. API keys are trimmed and must contain printable ASCII without internal
whitespace. Gateway path prefixes in base URLs are preserved.

### Builder and per-request options

```no_run
use std::time::Duration;
use typesafe_rs::{Client, Compression, JEV_1_14, Noul, RetryPolicy};

# async fn example() -> typesafe_rs::Result<()> {
let client = Client::builder()
    .model(JEV_1_14)
    .timeout(Duration::from_secs(30))
    .retry(RetryPolicy::default())
    .compression(Compression::Disabled) // For gateways without compression support.
    .build()?;

let response = client
    .system_one("The customer cannot sign in.")
    .question("technical", Noul::new("Is this a technical support issue?"))
    .timeout(Duration::from_secs(5))
    .send()
    .await?;
# Ok(())
# }
```

The client builder also accepts `api_key`, `base_url`, `headers`, and `http_client`.
Both inference and model-list requests accept timeout, retry, and header overrides.
Inference requests additionally accept model, compression, image assets, and question
options. A supplied reqwest client can customize proxies, TLS, pooling, and redirects;
the SDK still applies its own timeout.

### Experimental features

- `.images(true)` sends `TypeSafe-Danger-Zone-Images: true`. Required for images
  and Box; it is never enabled automatically.
- `.long_context(true)` sends `TypeSafe-Danger-Zone-Long-Context: true`, enabling
  the experimental 262k request/question context limits on Jev 1.13 and 1.14.

Both work on the client builder or per inference request. Request values override
client defaults; `false` explicitly turns a default off.

### Compression

The default `zstd` Cargo feature enables automatic compression of bodies of at
least 1024 bytes, including multipart bodies. Bodies are compressed once and
reused for retries.

| Option | Behavior |
| --- | --- |
| `Compression::Auto` | Use zstd when compiled in; otherwise send plain bodies |
| `Compression::Zstd` | Require the `zstd` Cargo feature |
| `Compression::Disabled` | Send uncompressed bodies |

Set the option on the client builder or inference request. To omit the compression
dependency entirely, add `default-features = false` to the Cargo dependency.

## Responses

### Answers and HTTP metadata

`Response<T>` dereferences to its typed data and retains status, headers, raw bytes,
and an optional request ID. Use `noul`, `choice`, `score`, and `box_answer` for borrowed
answers, or `nouls()`, `choices()`, `scores()`, and `boxes()` to iterate without
building additional maps. `into_inner()` extracts the decoded data.

Unknown response fields are ignored. Future answer kinds become `Answer::Unknown`;
their original payload is available in `Response::body`.

### Application-specific response types

Use `.send_as::<T>()` with a Serde `DeserializeOwned` type to validate your own
answer names and fields. Define the actual wire shape, including `answers`.
Add `serde = { version = "1", features = ["derive"] }` to your dependencies for
this example:

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
println!("Request ID: {:?}", response.request_id());
# Ok(())
# }
```

Custom responses retain the same HTTP metadata. Schema mismatches report the
field path and original response and are never retried.

### Listing models

```no_run
use typesafe_rs::Client;

# async fn example() -> typesafe_rs::Result<()> {
let client = Client::new()?;
for model in &client.models().list().send().await?.models {
    println!("{}: {} ({})", model.name, model.description, model.release_date);
}
# Ok(())
# }
```

## Retries and errors

### Retry behavior

The default policy retries twice for connection errors, timeouts, HTTP 408, 429,
and 5xx. Backoff starts at 500 ms, doubles up to 5 seconds, and subtracts up to 25%
jitter. `Retry-After` (seconds or HTTP date) and `retry-after-ms` override backoff.
Use `RetryPolicy::disabled()` to disable retries.

The 30-second retry budget stops new attempts; it does not interrupt an in-flight
attempt. Use `tokio::time::timeout` around `.send()` for a hard deadline. Dropping
a request future cancels pending I/O or retry sleep. Retrying an inference POST
can repeat server work if its first response was lost.

### Error details

Errors distinguish invalid configuration/input, image file reads, compression,
serialization, timeouts, transport, API failures, and malformed success responses.
API errors expose `kind()`, status, request ID, retry delay, headers, and body.
HTTP 413 and 415 have dedicated `PayloadTooLarge` and `UnsupportedMediaType` kinds.
Response validation errors include the Serde field path and original response.

### Headers and escape hatches

Authentication, accept/content type, content encoding, and SDK headers override
custom headers. A supplied HTTP client should not set encoding or content-length
defaults: the SDK computes these from the body. Retry counts start at zero. The
default client does not follow redirects; a supplied client's redirect policy
remains in effect. Client/builder debug output omits keys and custom headers;
request and response bodies are not logged.

`extra_body` is a last-write-wins, shallow-merge escape hatch for fields including
`state`, `model`, `questions`, and `assets`. A manual `assets` field replaces
registered images and prevents their files from being read.

## Version compatibility

Rust and Python SDK versions are independent. The Python version below identifies
the API contract used as the implementation baseline.

| Rust SDK | Python SDK baseline | Features |
| --- | --- | --- |
| `0.1.0` | `0.6.0` | Text: Noul, Choice, Score |
| `0.2.0` (tag pending) | `0.7.5` | Text, images, multipart/base64, Box, zstd |

| Model ID | Noul / Choice / Score | Images | Box |
| --- | --- | --- | --- |
| `jev-1.13.0` | Yes | Experimental | No |
| `jev-1.14.0` (`JEV_1_14`) | Yes | Experimental | Experimental |

Both models support the long-context header. As of October 16, 2026, the documented
`jev-latest` alias points to 1.13 and `jev-preview` to 1.14. Aliases move: pin an ID
when behavior must stay stable, especially for Box. See the
[official model list](https://docs.typesafe.ai/models).

## Development and tests

### Local checks

The default suite uses local HTTP fixtures and needs no API key:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test -p typesafe_rs --no-default-features
```

README Rust examples are also compiled by `cargo test --doc`.

### Live API tests

Six live contract tests are ignored by default. They cover model listing, text
primitives, custom response schemas, multipart/file and base64 vision/Box, and
zstd/long-context headers.

Export `TYPESAFE_API_KEY` in the terminal launching Cargo, then run:

```sh
cargo test --test live -- --ignored
```

Setting the key alone does not enable ignored tests. IDE test runners and other
already-running processes do not inherit environment changes from your terminal.
Live tests use the configured endpoint and make billable inference calls.

### Coverage and examples

With `cargo-llvm-cov` installed:

```sh
cargo llvm-cov -p typesafe_rs --all-features --summary-only
```

The workspace includes [TypeParanoid](examples/type-paranoid/README.md), a standalone
binary with embedded Rust packet capture and optional TypeSafe assessment of unusual
outbound transfers. Build it with `cargo build -p type-paranoid`. The text example
can be run with `cargo run --example classify`.
