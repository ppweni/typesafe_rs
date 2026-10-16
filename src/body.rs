use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use reqwest::header::HeaderValue;
use serde_json::{Map, Value, json};

use crate::{AssetEncoding, Assets, Error, Result};

const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct Body {
    pub bytes: Bytes,
    pub content_type: HeaderValue,
}

impl Body {
    pub(crate) async fn encode(
        mut body: Map<String, Value>,
        assets: Assets,
        encoding: AssetEncoding,
    ) -> Result<Self> {
        if let Some(state) = body.get("state") {
            validate_depth(state)?;
        }
        if let Some(questions) = body.get("questions").and_then(Value::as_object) {
            for question in questions.values() {
                for field in ["instructions", "criteria"] {
                    if let Some(value) = question.get(field) {
                        validate_depth(value)?;
                    }
                }
            }
        }
        // extra_body is a last-write-wins escape hatch. Overriding assets must not
        // read files or combine a manually supplied assets map with generated parts.
        if assets.is_empty() || body.contains_key("assets") {
            return Self::json(body);
        }
        validate_references(&body, &assets)?;
        let mut entries = Map::new();
        let mut images = Vec::with_capacity(assets.len());
        let mut total = 0;
        for (i, (reference, image)) in assets.iter().enumerate() {
            let bytes = image.bytes().await?;
            let (format, content_type) = crate::asset::format(&bytes)?;
            total += bytes.len();
            check_size(total)?;
            let entry = match encoding {
                AssetEncoding::Multipart => json!({"type": "image", "part": format!("asset-{i}")}),
                AssetEncoding::Base64 => {
                    json!({"type": "image", "format": format, "data_base64": STANDARD.encode(&bytes)})
                }
            };
            entries.insert(reference.clone(), entry);
            images.push((bytes, content_type));
        }
        body.insert("assets".into(), Value::Object(entries));
        if encoding == AssetEncoding::Base64 {
            return Self::json(body);
        }
        let boundary = format!("typesafe-{:032x}", fastrand::u128(..));
        let mut bytes = Vec::new();
        bytes.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"request\"\r\nContent-Type: application/json\r\n\r\n").as_bytes());
        serde_json::to_writer(&mut bytes, &body)?;
        for (i, (image, content_type)) in images.into_iter().enumerate() {
            bytes.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"asset-{i}\"; filename=\"asset-{i}\"\r\nContent-Type: {content_type}\r\n\r\n").as_bytes());
            bytes.extend_from_slice(&image);
        }
        bytes.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        check_size(bytes.len())?;
        Ok(Self {
            bytes: bytes.into(),
            content_type: HeaderValue::from_str(&format!(
                "multipart/form-data; boundary={boundary}"
            ))
            .expect("generated boundary is an ASCII header value"),
        })
    }

    fn json(body: Map<String, Value>) -> Result<Self> {
        let bytes = serde_json::to_vec(&body)?;
        check_size(bytes.len())?;
        Ok(Self {
            bytes: bytes.into(),
            content_type: HeaderValue::from_static("application/json"),
        })
    }
}

pub(crate) fn check_size(length: usize) -> Result<()> {
    if length > MAX_BODY_BYTES {
        return Err(Error::InvalidInput("request body exceeds 32 MiB".into()));
    }
    Ok(())
}

pub(crate) fn validate_depth(value: &Value) -> Result<()> {
    fn visit(value: &Value, depth: usize) -> Result<()> {
        match value {
            Value::Array(values) => {
                if depth == 64 {
                    return Err(Error::InvalidInput(
                        "content exceeds 64 nested levels".into(),
                    ));
                }
                for value in values {
                    visit(value, depth + 1)?;
                }
            }
            Value::Object(values) => {
                if depth == 64 {
                    return Err(Error::InvalidInput(
                        "content exceeds 64 nested levels".into(),
                    ));
                }
                for value in values.values() {
                    visit(value, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(value, 0)
}

fn validate_references(body: &Map<String, Value>, assets: &Assets) -> Result<()> {
    fn contains(value: &Value, reference: &str) -> bool {
        match value {
            Value::String(text) => text.contains(reference),
            Value::Array(values) => values.iter().any(|value| contains(value, reference)),
            Value::Object(values) => values.values().any(|value| contains(value, reference)),
            _ => false,
        }
    }
    let mut total = 0;
    for reference in assets.keys() {
        total += reference.len();
        if reference.trim().is_empty() || reference.len() > 1024 || total > 65536 {
            return Err(Error::InvalidInput("asset references must be nonblank, at most 1024 bytes each and 65536 bytes combined".into()));
        }
        let used = body
            .get("state")
            .is_some_and(|value| contains(value, reference))
            || body
                .get("questions")
                .and_then(Value::as_object)
                .is_some_and(|questions| {
                    questions.values().any(|question| {
                        ["instructions", "criteria"].iter().any(|field| {
                            question
                                .get(*field)
                                .is_some_and(|value| contains(value, reference))
                        })
                    })
                });
        if !used {
            return Err(Error::InvalidInput(format!(
                "asset {reference:?} is not referenced in state, instructions, or criteria"
            )));
        }
    }
    Ok(())
}
