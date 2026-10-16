//! Opt-in contract checks against the real API. Run with TYPESAFE_API_KEY set:
//! cargo test --test live -- --ignored

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use typesafe_rs::{
    AssetEncoding, BoxQuestion, Choice, Client, Image, JEV_1_14, Noul, NoulAnswer, Score,
    SystemOneResponse, Value,
};

#[cfg(feature = "zstd")]
use typesafe_rs::Compression;

fn client() -> Client {
    Client::builder()
        .model(JEV_1_14)
        .timeout(Duration::from_secs(30))
        .build()
        .expect("set TYPESAFE_API_KEY to run ignored live tests")
}

fn assert_usage(response: &SystemOneResponse) {
    assert_eq!(response.model, JEV_1_14);
    assert!(response.usage.input_tokens.is_some_and(|tokens| tokens > 0));
    assert!(
        response
            .usage
            .output_tokens
            .is_some_and(|tokens| tokens > 0)
    );
}

#[tokio::test]
#[ignore = "requires TYPESAFE_API_KEY; calls the live API"]
async fn live_models() {
    let response = client().models().list().send().await.unwrap();
    assert!(!response.models.is_empty());
    for model in &response.models {
        assert!(!model.name.is_empty());
        assert!(!model.description.is_empty());
        assert!(!model.release_date.is_empty());
    }
}

#[tokio::test]
#[ignore = "requires TYPESAFE_API_KEY; makes billable inference calls"]
async fn live_text_primitives() {
    let response = client()
        .system_one("The customer was charged twice and wants a refund today.")
        .question("billing", Noul::new("Is this about billing?"))
        .question(
            "category",
            Choice::new(["billing", "weather"]).instructions("What is this about?"),
        )
        .question(
            "urgency",
            Score::new(["Can wait", "Needs attention today"]).instructions("How urgent is this?"),
        )
        .send()
        .await
        .unwrap();
    assert_usage(&response);
    assert!(response.request_id().is_some());
    assert!(response.noul("billing").unwrap().noul > 0.5);
    let choice = response.choice("category").unwrap();
    assert_eq!(choice.choice, "billing");
    assert!((choice.probabilities.values().sum::<f64>() - 1.0).abs() < 0.01);
    assert!((0.0..=1.0).contains(&choice.confidence));
    let score = response.score("urgency").unwrap();
    assert!((0.0..=1.0).contains(&score.score));
    assert_eq!(score.legend.len(), 2);
    assert!((score.probabilities.values().sum::<f64>() - 1.0).abs() < 0.01);
}

#[tokio::test]
#[ignore = "requires TYPESAFE_API_KEY; makes billable inference calls"]
async fn live_custom_response() {
    #[derive(Deserialize)]
    struct Answers {
        red: NoulAnswer,
    }
    #[derive(Deserialize)]
    struct Decision {
        model: String,
        answers: Answers,
    }
    let response = client()
        .system_one("The apple is red.")
        .question("red", Noul::new("Is the apple red?"))
        .send_as::<Decision>()
        .await
        .unwrap();
    assert_eq!(response.model, JEV_1_14);
    assert!(response.answers.red.noul > 0.5);
    assert!(response.request_id().is_some());
    assert!(serde_json::from_slice::<Value>(&response.body).is_ok());
}

async fn vision(encoding: AssetEncoding, from_file: bool) {
    let temp = tempfile::NamedTempFile::new().unwrap();
    let bytes = include_bytes!("fixtures/red-square.png");
    let image = if from_file {
        std::fs::write(temp.path(), bytes).unwrap();
        Image::from_file(temp.path())
    } else {
        Image::from_base64(STANDARD.encode(bytes)).unwrap()
    };
    let response = client()
        .system_one(&image)
        .asset(image.clone())
        .asset_encoding(encoding)
        .images(true)
        .question(
            "present",
            Noul::new("Is there a solid red square on a white background?"),
        )
        .question("square", BoxQuestion::new("The red square"))
        .send()
        .await
        .unwrap();
    assert_usage(&response);
    assert!(response.noul("present").unwrap().noul > 0.5);
    let answer = response.box_answer("square").unwrap();
    let edges = answer.coordinates;
    for edge in [edges.left, edges.right, edges.top, edges.bottom] {
        assert!((0.0..=1.0).contains(&edge), "invalid coordinate: {edge}");
    }
    assert!(edges.left < edges.right && edges.top < edges.bottom);
    assert!(edges.left < 0.5 && edges.right > 0.5 && edges.top < 0.5 && edges.bottom > 0.5);
    if let Some(confidence) = answer.confidence {
        assert!((0.0..=1.0).contains(&confidence));
    }
    assert_eq!(response.boxes().count(), 1);
}

#[tokio::test]
#[ignore = "requires TYPESAFE_API_KEY; makes billable experimental vision calls"]
async fn live_multipart_image_and_box_from_file() {
    vision(AssetEncoding::Multipart, true).await;
}

#[tokio::test]
#[ignore = "requires TYPESAFE_API_KEY; makes billable experimental vision calls"]
async fn live_base64_image_and_box() {
    vision(AssetEncoding::Base64, false).await;
}

#[cfg(feature = "zstd")]
#[tokio::test]
#[ignore = "requires TYPESAFE_API_KEY; makes billable inference calls"]
async fn live_zstd_and_long_context_header() {
    let state = format!(
        "{}\nThe apple is red.",
        "Background: the sky is blue. ".repeat(100)
    );
    let response = client()
        .system_one(state)
        .compression(Compression::Zstd)
        .long_context(true)
        .question("red", Noul::new("Is the apple red?"))
        .send()
        .await
        .unwrap();
    assert_usage(&response);
    assert!(response.noul("red").unwrap().noul > 0.5);
}
