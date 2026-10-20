use std::{collections::BTreeMap, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use typesafers::{
    Answer, AssetEncoding, BoxQuestion, Choice, Client, Compression, Content, Error, IMAGES_HEADER,
    Image, JEV_1_14, LONG_CONTEXT_HEADER, Noul, NoulCriteria, Question, RetryPolicy, Score, Value,
    header::HeaderMap, json,
};

mod common;
use common::{Captured, Reply, Server};

const PNG: &[u8] = include_bytes!("fixtures/red-square.png");

fn image() -> Image {
    Image::from_bytes(PNG).unwrap()
}

fn inference() -> Value {
    json!({"model": JEV_1_14, "usage": {"input_tokens": 120, "output_tokens": 20}, "answers": {
        "present": {"type": "noul", "noul": 0.99},
        "square": {"type": "box", "box": {"top": 0.25, "bottom": 0.75, "left": 0.25, "right": 0.75}, "confidence": null},
        "other": {"type": "box", "box": {"top": 0.1, "bottom": 0.9, "left": 0.2, "right": 0.8}, "confidence": 0.9}
    }})
}

fn decoded(request: &Captured) -> Vec<u8> {
    match request.headers.get("content-encoding").map(String::as_str) {
        None => request.body.clone(),
        #[cfg(feature = "zstd")]
        Some("zstd") => zstd::stream::decode_all(request.body.as_slice()).unwrap(),
        encoding => panic!("unexpected encoding {encoding:?}"),
    }
}

/// Parse actual wire framing independently of the production encoder.
fn multipart(request: &Captured) -> Vec<(String, Vec<u8>)> {
    let body = decoded(request);
    let boundary = request.headers["content-type"]
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap();
    let opening = format!("--{boundary}\r\n");
    assert!(body.starts_with(opening.as_bytes()));
    let delimiter = format!("\r\n--{boundary}");
    let mut rest = &body[opening.len()..];
    let mut parts = vec![];
    loop {
        let end = rest.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let headers = String::from_utf8(rest[..end].to_vec()).unwrap();
        rest = &rest[end + 4..];
        let end = rest
            .windows(delimiter.len())
            .position(|w| w == delimiter.as_bytes())
            .unwrap();
        parts.push((headers, rest[..end].to_vec()));
        rest = &rest[end + delimiter.len()..];
        if rest == b"--\r\n" {
            break;
        }
        assert!(rest.starts_with(b"\r\n"));
        rest = &rest[2..];
    }
    parts
}

#[tokio::test]
async fn multipart_images_in_state_instructions_and_criteria_and_typed_boxes() {
    let mut server = Server::start(vec![Reply::json(200, inference())]).await;
    let image = image();
    let client = server.client();
    let response = client
        .system_one(json!({"photo": image, "frames": [image]}))
        .asset(image.clone())
        .asset(image.clone())
        .model(JEV_1_14)
        .images(true)
        .question("present", Noul::new(format!("Is this present: {image}?")))
        .question("match", Noul::new(image.clone()))
        .question(
            "choice",
            Choice {
                instructions: None,
                criteria: BTreeMap::from([("example".into(), Some(image.clone().into()))]),
            },
        )
        .question(
            "score",
            Score::new([Content::from("absent"), image.clone().into()]),
        )
        .question("square", BoxQuestion::new("The red square in `photo`"))
        .send()
        .await
        .unwrap();
    let answer = response.box_answer("square").unwrap();
    assert_eq!(answer.coordinates.left, 0.25);
    assert_eq!(answer.confidence, None);
    assert_eq!(response.box_answer("other").unwrap().confidence, Some(0.9));
    assert_eq!(response.boxes().count(), 2);
    assert!(response.box_answer("present").is_none());
    assert!(response.box_answer("missing").is_none());
    assert!(response.answers["square"].as_noul().is_none());
    assert!(response.answers["square"].as_choice().is_none());
    assert!(response.answers["square"].as_score().is_none());
    assert!(matches!(response.answers["square"], Answer::Box(_)));
    let request = server.request().await;
    assert_eq!(request.line, "POST /v1/systemone HTTP/1.1");
    assert_eq!(request.headers[IMAGES_HEADER], "true");
    let parts = multipart(&request);
    assert_eq!(parts.len(), 2); // One upload despite repeated registration/references.
    assert!(parts[0].0.contains("name=\"request\""));
    assert!(parts[0].0.contains("Content-Type: application/json"));
    assert!(parts[1].0.contains("name=\"asset-0\""));
    assert!(parts[1].0.contains("Content-Type: image/png"));
    assert_eq!(parts[1].1, PNG);
    let body: Value = serde_json::from_slice(&parts[0].1).unwrap();
    assert_eq!(body["state"]["photo"], image.reference());
    assert_eq!(
        body["assets"][image.reference()],
        json!({"type": "image", "part": "asset-0"})
    );
    assert_eq!(
        body["questions"]["match"]["instructions"],
        image.reference()
    );
    assert_eq!(
        body["questions"]["choice"]["criteria"]["example"],
        image.reference()
    );
    assert_eq!(body["questions"]["score"]["criteria"][1], image.reference());
    assert_eq!(
        body["questions"]["square"],
        json!({"type": "box", "instructions": "The red square in `photo`"})
    );
}

#[tokio::test]
async fn base64_upload_with_explicit_aliases_and_jpeg_detection() {
    let mut server = Server::start(vec![Reply::json(200, inference())]).await;
    let jpeg = Image::from_bytes(b"\xff\xd8\xfftest".as_slice()).unwrap();
    server
        .client()
        .system_one(json!({"photo": "opaque-photo", "second": "opaque-jpeg"}))
        .assets([
            (
                "opaque-photo",
                Image::from_base64(STANDARD.encode(PNG)).unwrap(),
            ),
            ("opaque-jpeg", jpeg),
        ])
        .asset_encoding(AssetEncoding::Base64)
        .images(true)
        .question("present", Noul::new("Is there a square?"))
        .send()
        .await
        .unwrap();
    let request = server.request().await;
    assert_eq!(request.headers["content-type"], "application/json");
    let body: Value = serde_json::from_slice(&decoded(&request)).unwrap();
    assert_eq!(
        body["assets"]["opaque-photo"],
        json!({"type": "image", "format": "png", "data_base64": STANDARD.encode(PNG)})
    );
    assert_eq!(body["assets"]["opaque-jpeg"]["format"], "jpg");
}

#[tokio::test]
async fn multiple_multipart_files_use_distinct_parts_and_types() {
    let mut server = Server::start(vec![Reply::json(200, inference())]).await;
    let png = image();
    let jpeg = Image::from_bytes(b"\xff\xd8\xffbinary\r\nfile\0".as_slice()).unwrap();
    server
        .client()
        .system_one(json!([png, jpeg]))
        .asset(png.clone())
        .asset(jpeg.clone())
        .images(true)
        .question("present", Noul::new("Are they the same?"))
        .send()
        .await
        .unwrap();
    let parts = multipart(&server.request().await);
    assert_eq!(parts.len(), 3);
    let body: Value = serde_json::from_slice(&parts[0].1).unwrap();
    for (reference, expected, mime) in [
        (png.reference(), PNG, "image/png"),
        (
            jpeg.reference(),
            b"\xff\xd8\xffbinary\r\nfile\0".as_slice(),
            "image/jpeg",
        ),
    ] {
        let part = body["assets"][reference]["part"].as_str().unwrap();
        let found = parts
            .iter()
            .find(|(headers, _)| headers.contains(&format!("name=\"{part}\"")))
            .unwrap();
        assert!(found.0.contains(mime));
        assert_eq!(found.1, expected);
    }
}

#[tokio::test]
async fn retries_reuse_identical_multipart_bytes_and_compression() {
    let mut server = Server::start(vec![
        Reply::json(503, json!({})),
        Reply::json(200, inference()),
    ])
    .await;
    let image = image();
    server
        .client()
        .system_one(&image)
        .asset(image.clone())
        .images(true)
        .question("present", Noul::new("Square?"))
        .retry(RetryPolicy {
            backoff_initial: Duration::ZERO,
            ..RetryPolicy::default()
        })
        .send()
        .await
        .unwrap();
    let first = server.request().await;
    let second = server.request().await;
    assert_eq!(first.body, second.body);
    assert_eq!(
        first.headers["content-type"],
        second.headers["content-type"]
    );
    assert_eq!(
        first.headers.get("content-encoding"),
        second.headers.get("content-encoding")
    );
    assert_eq!(second.headers["x-typesafe-retry-count"], "1");
    assert_eq!(multipart(&first)[1].1, PNG);
}

#[tokio::test]
async fn image_files_are_cached_across_clones_and_concurrent_requests() {
    let mut server = Server::start(vec![Reply::json(200, inference())]).await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("photo.png");
    std::fs::write(&path, PNG).unwrap();
    let image = Image::from_file(&path);
    let client = server.client();
    let send = || {
        client
            .system_one(&image)
            .asset(image.clone())
            .question("present", Noul::new("Square?"))
    };
    let (one, two) = tokio::join!(send().send(), send().send());
    one.unwrap();
    two.unwrap();
    std::fs::remove_file(path).unwrap();
    send().send().await.unwrap();
    for _ in 0..3 {
        assert_eq!(multipart(&server.request().await)[1].1, PNG);
    }
}

#[tokio::test]
async fn file_failures_are_recoverable_and_reported_before_network_io() {
    let server = Server::start(vec![Reply::json(200, inference())]).await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("photo.png");
    let image = Image::from_file(&path);
    let client = server.client();
    let request = || {
        client
            .system_one(&image)
            .asset(image.clone())
            .question("present", Noul::new("Square?"))
    };
    assert!(matches!(request().send().await, Err(Error::AssetRead(_))));
    assert_eq!(server.count(), 0);
    std::fs::write(&path, []).unwrap();
    assert!(matches!(
        request().send().await,
        Err(Error::InvalidInput(_))
    ));
    std::fs::write(&path, PNG).unwrap();
    request().send().await.unwrap();
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn invalid_assets_do_not_reach_the_network_and_extra_body_wins() {
    let mut server = Server::start(vec![Reply::json(200, inference())]).await;
    let client = server.client();
    for name in ["", "  ", &"a".repeat(1025), "unused"] {
        let error = client
            .system_one("state")
            .assets([(name, image())])
            .question("present", Noul::new("Square?"))
            .send()
            .await
            .unwrap_err();
        assert!(matches!(error, Error::InvalidInput(_)));
    }
    let bytes = Image::from_bytes(b"not an image".as_slice()).unwrap();
    assert!(matches!(
        client
            .system_one(&bytes)
            .asset(bytes.clone())
            .question("present", Noul::new("Square?"))
            .send()
            .await,
        Err(Error::InvalidInput(_))
    ));
    // Object keys are never image references.
    assert!(
        client
            .system_one(json!({"key-only": "text"}))
            .assets([("key-only", image())])
            .question("present", Noul::new("Square?"))
            .send()
            .await
            .is_err()
    );
    assert_eq!(server.count(), 0);
    client
        .system_one("state")
        .asset(Image::from_file("/nonexistent/not-read.png"))
        .question("present", Noul::new("Square?"))
        .extra_body(
            json!({"assets": {"manual": {"format": "png", "data_base64": "manually-encoded"}}})
                .as_object()
                .unwrap()
                .clone(),
        )
        .send()
        .await
        .unwrap();
    let request = server.request().await;
    assert_eq!(request.headers["content-type"], "application/json");
    let body: Value = serde_json::from_slice(&decoded(&request)).unwrap();
    assert_eq!(body["assets"]["manual"]["data_base64"], "manually-encoded");
}

#[test]
fn image_constructors_references_and_debug_output() {
    assert!(Image::from_bytes(Vec::new()).is_err());
    assert!(Image::from_bytes(vec![0; 16 * 1024 * 1024 + 1]).is_err());
    for data in ["", " ", "!!!!", "data:image/png;base64,AAAA", "a", "é"] {
        assert!(Image::from_base64(data).is_err(), "{data}");
    }
    assert!(Image::from_base64("a".repeat((16_usize * 1024 * 1024).div_ceil(3) * 4 + 4)).is_err());
    let image = image();
    assert_eq!(image, image.clone());
    assert_ne!(image, Image::from_bytes(PNG).unwrap());
    assert_eq!(image.to_string(), image.reference());
    assert_eq!(
        serde_json::to_value(&image).unwrap(),
        json!(image.reference())
    );
    assert!(!format!("{image:?}").contains("PNG"));
    Image::from_base64(format!(" \n{}\r\n", STANDARD.encode(PNG))).unwrap();
}

#[tokio::test]
async fn feature_headers_and_api_keys_support_client_defaults_and_request_overrides() {
    let mut server = Server::start(vec![
        Reply::json(200, inference()).header("x-typesafe-request-id", "req-features"),
    ])
    .await;
    let client = Client::builder()
        .api_key(" \ttrimmed-key\r\n")
        .base_url(&server.url)
        .images(true)
        .long_context(true)
        .build()
        .unwrap();
    client
        .system_one("state")
        .question("present", Noul::new("True?"))
        .images(false)
        .long_context(false)
        .send()
        .await
        .unwrap();
    let request = server.request().await;
    assert_eq!(request.headers["authorization"], "Bearer trimmed-key");
    assert_eq!(request.headers[IMAGES_HEADER], "false");
    assert_eq!(request.headers[LONG_CONTEXT_HEADER], "false");
    client
        .system_one("state")
        .question("present", Noul::new("True?"))
        .send()
        .await
        .unwrap();
    let request = server.request().await;
    assert_eq!(request.headers[IMAGES_HEADER], "true");
    assert_eq!(request.headers[LONG_CONTEXT_HEADER], "true");
    for key in ["a b", "a\tb", "a\nb", "a\0b", "a\x7fb", "sécret"] {
        let error = Client::builder().api_key(key).build().unwrap_err();
        assert!(matches!(error, Error::Configuration(_)));
        assert!(!error.to_string().contains(key));
    }
}

#[tokio::test]
async fn custom_response_types_keep_metadata_and_schema_errors_are_not_retried() {
    #[derive(Debug, Deserialize)]
    struct Answers {
        present: typesafers::NoulAnswer,
    }
    #[derive(Debug, Deserialize)]
    struct Typed {
        model: String,
        answers: Answers,
    }
    let mut server = Server::start(vec![
        Reply::json(200, inference()).header("x-typesafe-request-id", "req-custom"),
    ])
    .await;
    let client = server.client();
    let response = client
        .system_one("state")
        .question("present", Noul::new("Square?"))
        .send_as::<Typed>()
        .await
        .unwrap();
    assert_eq!(response.model, JEV_1_14);
    assert_eq!(response.answers.present.noul, 0.99);
    assert_eq!(response.request_id(), Some("req-custom"));
    assert_eq!(response.status.as_u16(), 200);
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap(),
        inference()
    );
    #[derive(Debug, Deserialize)]
    struct Invalid {
        #[serde(rename = "model")]
        _model: u64,
    }
    let Error::ResponseValidation(error) = client
        .system_one("state")
        .question("present", Noul::new("Square?"))
        .retry(RetryPolicy::default())
        .send_as::<Invalid>()
        .await
        .unwrap_err()
    else {
        panic!("expected schema error")
    };
    assert_eq!(error.field_path, "model");
    assert_eq!(error.request_id(), Some("req-custom"));
    assert_eq!(server.count(), 2);
    server.request().await;
    server.request().await;
}

#[tokio::test]
async fn compression_threshold_defaults_and_request_overrides() {
    let mut server = Server::start(vec![Reply::json(200, inference())]).await;
    let client = Client::builder()
        .api_key("key")
        .base_url(&server.url)
        .model("jev-latest")
        .compression(Compression::Disabled)
        .build()
        .unwrap();
    // The complete payload is exactly 1023 or 1024 bytes.
    let template = json!({"model": "jev-latest", "state": "", "questions": {"present": {"type": "noul", "instructions": "True?"}}});
    for size in [1023, 1024] {
        let state = "a".repeat(size - serde_json::to_vec(&template).unwrap().len());
        client
            .system_one(&state)
            .question("present", Noul::new("True?"))
            .compression(Compression::Auto)
            .send()
            .await
            .unwrap();
        let request = server.request().await;
        assert_eq!(decoded(&request).len(), size);
        assert_eq!(
            request.headers.contains_key("content-encoding"),
            cfg!(feature = "zstd") && size >= 1024
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&decoded(&request)).unwrap()["state"],
            state
        );
    }
    let mut headers = HeaderMap::new();
    headers.insert("content-encoding", "gzip".parse().unwrap());
    headers.insert("content-type", "wrong".parse().unwrap());
    client
        .system_one("a".repeat(4000))
        .question("present", Noul::new("True?"))
        .headers(headers)
        .compression(Compression::Disabled)
        .send()
        .await
        .unwrap();
    let request = server.request().await;
    assert!(!request.headers.contains_key("content-encoding"));
    assert_eq!(request.headers["content-type"], "application/json");
    assert!(serde_json::from_slice::<Value>(&request.body).is_ok());
    #[cfg(feature = "zstd")]
    {
        client
            .system_one("a".repeat(4000))
            .question("present", Noul::new("True?"))
            .compression(Compression::Zstd)
            .send()
            .await
            .unwrap();
        assert_eq!(server.request().await.headers["content-encoding"], "zstd");
    }
    #[cfg(not(feature = "zstd"))]
    {
        assert!(
            Client::builder()
                .api_key("key")
                .compression(Compression::Zstd)
                .build()
                .is_err()
        );
        assert!(matches!(
            client
                .system_one("state")
                .question("present", Noul::new("True?"))
                .compression(Compression::Zstd)
                .send()
                .await,
            Err(Error::Configuration(_))
        ));
    }
}

#[tokio::test]
async fn question_limits_and_noul_validation_are_consistent_for_typed_and_raw_inputs() {
    let server = Server::start(vec![Reply::json(200, inference())]).await;
    let client = server.client();
    let invalid: Vec<Question> = vec![
        Choice::new(Vec::<String>::new()).into(),
        Choice::new((0..256).map(|i| i.to_string())).into(),
        Score::new(Vec::<String>::new()).into(),
        Score::new(["level"; 11]).into(),
        Noul::default().into(),
        Noul::default().criteria(NoulCriteria::default()).into(),
        Question::Raw(json!({"type": "noul", "criteria": []})),
        Question::Raw(json!({"type": "noul", "instructions": 42})),
        Question::Raw(json!({"type": "choice", "criteria": {"bad": 42}})),
        Question::Raw(json!({"type": "score", "criteria": [null]})),
        Question::Raw(json!({"type": "noul", "criteria": {"true": 42}})),
        Question::Raw(json!({"type": "noul", "criteria": {"true": null, "false": null}})),
        Question::Raw(json!({"type": "noul", "criteria": {"unknown": "not an outcome"}})),
        Question::Raw(json!({"type": "choice", "criteria": []})),
        Question::Raw(json!({"type": "score", "criteria": {"0": "level"}})),
    ];
    for question in invalid {
        assert!(matches!(
            client
                .system_one("state")
                .question("invalid", question)
                .send()
                .await,
            Err(Error::InvalidInput(_))
        ));
    }
    assert_eq!(server.count(), 0);
    for question in [
        Question::from(Choice::new((0..255).map(|i| i.to_string()))),
        Score::new(["level"; 10]).into(),
        Score::new(["level"]).into(),
        Noul::default()
            .criteria(NoulCriteria {
                yes: Some("yes".into()),
                no: None,
            })
            .into(),
        Noul::default()
            .criteria(NoulCriteria {
                yes: None,
                no: Some("no".into()),
            })
            .into(),
        BoxQuestion::default().into(),
    ] {
        client
            .system_one("state")
            .question("valid", question)
            .send()
            .await
            .unwrap();
    }
    assert_eq!(server.count(), 6);
}

#[tokio::test]
async fn nesting_and_body_size_limits_are_enforced_before_io() {
    let server = Server::start(vec![Reply::json(200, inference())]).await;
    let client = server.client();
    let mut nested = json!("leaf");
    for _ in 0..64 {
        nested = json!([nested]);
    }
    client
        .system_one(nested.clone())
        .question("present", Noul::new("True?"))
        .send()
        .await
        .unwrap();
    nested = json!([nested]);
    for (state, instructions) in [
        (nested.clone(), Content::from("True?")),
        (json!("state"), Content::try_from(nested).unwrap()),
    ] {
        assert!(matches!(
            client
                .system_one(state)
                .question("present", Noul::new(instructions))
                .send()
                .await,
            Err(Error::InvalidInput(_))
        ));
    }
    assert!(matches!(
        client
            .system_one("a".repeat(32 * 1024 * 1024))
            .question("present", Noul::new("True?"))
            .send()
            .await,
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn oversized_file_is_rejected_without_reading_it_into_memory() {
    let server = Server::start(vec![Reply::json(200, inference())]).await;
    let temp = tempfile::NamedTempFile::new().unwrap();
    temp.as_file().set_len(16 * 1024 * 1024 + 1).unwrap();
    let image = Image::from_file(temp.path());
    assert!(matches!(
        server
            .client()
            .system_one(&image)
            .asset(image.clone())
            .question("present", Noul::new("True?"))
            .send()
            .await,
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn image_references_in_question_content_and_raw_future_questions() {
    #[derive(serde::Serialize)]
    struct State<'a> {
        label: &'a str,
        nested: Vec<&'a Image>,
    }
    let mut server = Server::start(vec![Reply::json(200, inference())]).await;
    let image = image();
    server
        .client()
        .system_one(State {
            label: "example",
            nested: vec![&image],
        })
        .asset(image.clone())
        .images(true)
        .questions([
            (
                "criteria",
                Question::from(Noul::default().criteria(NoulCriteria {
                    yes: Some(image.clone().into()),
                    no: None,
                })),
            ),
            (
                "future",
                Question::Raw(json!({"type": "future", "instructions": 42, "extension": true})),
            ),
        ])
        .send()
        .await
        .unwrap();
    let parts = multipart(&server.request().await);
    let body: Value = serde_json::from_slice(&parts[0].1).unwrap();
    assert_eq!(body["state"]["nested"][0], image.reference());
    assert_eq!(body["questions"]["future"]["instructions"], 42);
    // An image may be used only by a question, with no image in state.
    server
        .client()
        .system_one("state")
        .asset(image.clone())
        .question("present", Noul::new(format!("Is this square red? {image}")))
        .send()
        .await
        .unwrap();
    assert_eq!(multipart(&server.request().await)[1].1, PNG);
    server
        .client()
        .system_one("state")
        .asset(image.clone())
        .question(
            "present",
            Noul::default().criteria(NoulCriteria {
                yes: Some(image.into()),
                no: None,
            }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(multipart(&server.request().await)[1].1, PNG);
}

#[tokio::test]
async fn reference_totals_and_complete_upload_sizes_are_bounded() {
    let server = Server::start(vec![Reply::json(200, inference())]).await;
    let client = server.client();
    let image = image();
    let references: Vec<_> = (0..65)
        .map(|i| format!("{i:04}{}", "x".repeat(1020)))
        .collect();
    let assets = references
        .iter()
        .map(|reference| (reference.clone(), image.clone()));
    assert!(matches!(
        client
            .system_one(&references)
            .assets(assets)
            .question("present", Noul::new("True?"))
            .send()
            .await,
        Err(Error::InvalidInput(_))
    ));
    // The two files individually fit, but multipart framing makes the body exceed 32 MiB.
    let mut bytes = vec![0; 16 * 1024 * 1024];
    bytes[..8].copy_from_slice(&PNG[..8]);
    let large = Image::from_bytes(bytes).unwrap();
    for (names, encoding) in [
        (vec!["one", "two"], AssetEncoding::Multipart),
        (vec!["one", "two"], AssetEncoding::Base64),
        (vec!["one", "two", "three"], AssetEncoding::Multipart),
    ] {
        assert!(matches!(
            client
                .system_one(&names)
                .assets(names.iter().map(|name| (*name, large.clone())))
                .asset_encoding(encoding)
                .question("present", Noul::new("True?"))
                .send()
                .await,
            Err(Error::InvalidInput(_))
        ));
    }
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn final_extra_body_content_still_obeys_nesting_limits() {
    let server = Server::start(vec![Reply::json(200, inference())]).await;
    let mut nested = json!("leaf");
    for _ in 0..65 {
        nested = json!({"next": nested});
    }
    for extra in [
        json!({"state": nested}),
        json!({"questions": {"present": {"type": "noul", "instructions": nested}}}),
    ] {
        assert!(matches!(
            server
                .client()
                .system_one("state")
                .question("present", Noul::new("True?"))
                .extra_body(extra.as_object().unwrap().clone())
                .send()
                .await,
            Err(Error::InvalidInput(_))
        ));
    }
    assert_eq!(server.count(), 0);
}
