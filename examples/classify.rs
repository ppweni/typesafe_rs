use typesafe_rs::{Choice, Client, Noul, Score, json};

#[tokio::main]
async fn main() -> typesafe_rs::Result<()> {
    let client = Client::new()?;
    let response = client
        .system_one(json!({"document": "I was charged twice. Please fix this ASAP."}))
        .question("billing", Noul::new("Is this about billing?"))
        .question(
            "tone",
            Choice::new(["calm", "frustrated", "angry"]).instructions("What is the tone?"),
        )
        .question(
            "urgency",
            Score::new(["can wait", "this week", "today"]).instructions("How urgent is it?"),
        )
        .send()
        .await?;
    for (name, answer) in &response.answers {
        println!("{name}: {answer:?}");
    }
    println!(
        "Usage: {:?}; request ID: {:?}",
        response.usage,
        response.request_id()
    );
    Ok(())
}
