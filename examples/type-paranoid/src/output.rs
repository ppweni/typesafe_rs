use crate::{capture, engine::Window};
use serde_json::json;
use std::sync::atomic::Ordering;

pub fn capture_health(capture: &capture::Capture, json_output: bool) {
    let dropped = capture.dropped.load(Ordering::Relaxed);
    let unparsed = capture.unparsed.load(Ordering::Relaxed);
    if json_output {
        println!(
            "{}",
            json!({"event": "capture_health", "queue_dropped_packets": dropped,
            "unparsed_or_unsupported_packets": unparsed, "kernel_drop_count": null})
        );
    } else if dropped > 0 || unparsed > 0 {
        eprintln!(
            "Capture totals: {dropped} queue drops; {unparsed} unparsed/unsupported packets; kernel losses unknown."
        );
    }
}

pub fn window(window: &Window, json_output: bool) {
    if json_output {
        println!("{}", json!({"event": "window", "data": window}));
    } else {
        println!(
            "Window {} ({:.1}s): {} peers, {} sent / {} received payload bytes; {} candidates{}",
            window.number,
            window.seconds,
            window.peers,
            window.sent_bytes,
            window.received_bytes,
            window.candidates.len(),
            if window.omitted_candidates > 0 {
                " (additional candidates omitted)"
            } else {
                ""
            }
        );
        for candidate in &window.candidates {
            println!(
                "  WATCH {} {:?}: {} bytes out; {}; process=unknown",
                candidate.peer.remote,
                candidate.peer.protocol,
                candidate.sent_bytes,
                candidate.signals.join(", ")
            );
        }
        if window.untracked_packets > 0 {
            eprintln!(
                "Flow limit: {} packets were not attributed to peers.",
                window.untracked_packets
            );
        }
    }
}

pub fn assessment(batch: &crate::assess::Batch, json_output: bool) {
    match &batch.result {
        Ok((assessments, request_id)) => {
            if json_output {
                println!(
                    "{}",
                    json!({"event": "assessment", "window": batch.window,
                    "request_id": request_id, "assessments": assessments})
                );
            } else {
                for assessment in assessments {
                    println!(
                        "  TYPESAFE {}: {} (review priority {:.2}/3, confidence {:.2})",
                        assessment.peer.remote,
                        assessment.classification,
                        assessment.review_priority,
                        assessment.classification_confidence
                    );
                }
            }
        }
        Err(error) => {
            if json_output {
                println!(
                    "{}",
                    json!({"event": "assessment_error", "window": batch.window, "error": error.to_string()})
                );
            } else {
                eprintln!(
                    "TypeSafe assessment failed: {error:#}; local findings remain available."
                );
            }
        }
    }
}
