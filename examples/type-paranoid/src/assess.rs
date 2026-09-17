use crate::engine::{Peer, Window};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::json;
use typesafe_rs::{Choice, Client, Score};

#[derive(Debug, Serialize)]
pub struct Assessment {
    pub peer: Peer,
    pub classification: String,
    pub review_priority: f64,
    pub classification_confidence: f64,
}

pub struct Batch {
    pub window: u64,
    pub result: Result<(Vec<Assessment>, Option<String>)>,
}

async fn assess(client: &Client, window: &Window) -> Result<(Vec<Assessment>, Option<String>)> {
    let state = json!({
        "context": "Passive outbound traffic observations. Byte counts are observed TCP/UDP payload bytes, including retransmissions and encryption overhead. Process identity and payload contents are unknown. A new peer is only new within bounded in-memory history; first-window novelty is not proof of malicious behavior. Local signals are heuristics, not verified attacks. Expected peers were filtered locally. Evaluate the evidence without inventing destination reputation, process details, or intent.",
        "window": window,
    });
    let mut request = client.system_one(state);
    for index in 0..window.candidates.len() {
        request = request
            .question(format!("behavior_{index}"), Choice::new(["routine", "unusual", "insufficient_evidence"])
                .instructions(format!("Classify only window.candidates[{index}] using the supplied observations. Distinguish unusual transfer behavior from evidence of malicious intent. Select insufficient_evidence when context is inadequate.")))
            .question(format!("priority_{index}"), Score::new([
                "No investigation indicated by available evidence",
                "Low priority: gather more context",
                "Medium priority: investigate the unusual transfer",
                "High priority: promptly investigate multiple strong anomaly signals",
            ]).instructions(format!("Rate the investigation priority of window.candidates[{index}]. This is a review-priority rubric, not a probability of exfiltration.")));
    }
    let response = request.send().await?;
    let mut assessments = Vec::new();
    for (index, candidate) in window.candidates.iter().enumerate() {
        let choice = response
            .choice(&format!("behavior_{index}"))
            .context("missing classification answer")?;
        let score = response
            .score(&format!("priority_{index}"))
            .context("missing priority answer")?;
        if !matches!(
            choice.choice.as_str(),
            "routine" | "unusual" | "insufficient_evidence"
        ) || !(0.0..=3.0).contains(&score.score)
            || !(0.0..=1.0).contains(&choice.confidence)
        {
            bail!("TypeSafe returned an answer outside the requested classification/rubric");
        }
        assessments.push(Assessment {
            peer: candidate.peer,
            classification: choice.choice.clone(),
            review_priority: score.score,
            classification_confidence: choice.confidence,
        });
    }
    Ok((assessments, response.request_id().map(str::to_owned)))
}

pub async fn run(client: &Client, window: Window) -> Batch {
    Batch {
        window: window.number,
        result: assess(client, &window).await,
    }
}

#[cfg(test)]
#[path = "assess_tests.rs"]
mod tests;
