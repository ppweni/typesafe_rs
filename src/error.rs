use std::{fmt, time::Duration};

use bytes::Bytes;
use reqwest::{StatusCode, header::HeaderMap};
use serde_json::Value;

pub type Result<T> = std::result::Result<T, Error>;

/// SDK failures; HTTP and decoding errors retain the original response.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid configuration: {0}")]
    Configuration(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("could not encode request JSON: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("request timed out: {0}")]
    Timeout(#[source] reqwest::Error),
    #[error("HTTP transport failed: {0}")]
    Transport(#[source] reqwest::Error),
    #[error(transparent)]
    Api(Box<ApiError>),
    #[error(transparent)]
    ResponseValidation(Box<ResponseValidationError>),
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        // A custom HTTP client may attach query parameters. Never expose them in errors.
        let error = error.without_url();
        if error.is_timeout() {
            Self::Timeout(error)
        } else {
            Self::Transport(error)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApiErrorKind {
    BadRequest,
    Authentication,
    PermissionDenied,
    NotFound,
    UnprocessableEntity,
    RateLimit,
    InternalServer,
    Other,
}

pub struct ApiError {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub endpoint: String,
    pub message: String,
}
impl ApiError {
    pub fn kind(&self) -> ApiErrorKind {
        match self.status.as_u16() {
            400 => ApiErrorKind::BadRequest,
            401 => ApiErrorKind::Authentication,
            403 => ApiErrorKind::PermissionDenied,
            404 => ApiErrorKind::NotFound,
            422 => ApiErrorKind::UnprocessableEntity,
            429 => ApiErrorKind::RateLimit,
            500..=599 => ApiErrorKind::InternalServer,
            _ => ApiErrorKind::Other,
        }
    }
    pub fn request_id(&self) -> Option<&str> {
        self.headers.get("x-typesafe-request-id")?.to_str().ok()
    }
    pub fn retry_after(&self) -> Option<Duration> {
        crate::retry::retry_after(&self.headers)
    }
    pub(crate) fn new(
        status: StatusCode,
        headers: HeaderMap,
        body: Bytes,
        endpoint: String,
    ) -> Self {
        let message = serde_json::from_slice::<Value>(&body)
            .ok()
            .as_ref()
            .and_then(extract_message)
            .unwrap_or_else(|| {
                if body.is_empty() {
                    return "no response body".into();
                }
                let text = String::from_utf8_lossy(&body);
                let mut shortened: String = text.chars().take(200).collect();
                if text.chars().count() > 200 {
                    shortened.push('…');
                }
                shortened
            });
        Self {
            status,
            headers,
            body,
            endpoint,
            message,
        }
    }
}
impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} {}",
            self.endpoint,
            self.status.as_u16(),
            self.message
        )?;
        if let Some(id) = self.request_id() {
            write!(f, " (request_id={id})")?;
        }
        Ok(())
    }
}
impl fmt::Debug for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for ApiError {}

/// A successful HTTP response that does not match the expected JSON schema.
pub struct ResponseValidationError {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub endpoint: String,
    pub field_path: String,
    pub source: serde_json::Error,
}
impl ResponseValidationError {
    pub fn request_id(&self) -> Option<&str> {
        self.headers.get("x-typesafe-request-id")?.to_str().ok()
    }
}
impl fmt::Display for ResponseValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: invalid response at {}: {}",
            self.endpoint, self.field_path, self.source
        )?;
        if let Some(id) = self.request_id() {
            write!(f, " (request_id={id})")?;
        }
        Ok(())
    }
}
impl fmt::Debug for ResponseValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for ResponseValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn extract_message(body: &Value) -> Option<String> {
    if let Some(text) = body.as_str().filter(|s| !s.is_empty()) {
        return Some(text.into());
    }
    for value in [
        body.get("error"),
        body.pointer("/error/message"),
        body.get("message"),
        body.get("detail"),
        body.pointer("/detail/message"),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(text) = value.as_str() {
            return Some(text.into());
        }
    }
    let details = body.get("detail")?.as_array()?;
    let parts: Vec<_> = details
        .iter()
        .filter_map(|entry| {
            let message = entry.get("msg")?.as_str()?;
            let path = entry
                .get("loc")
                .and_then(Value::as_array)
                .map(|loc| {
                    loc.iter()
                        .filter(|p| p.as_str() != Some("body"))
                        .map(|p| {
                            p.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| p.to_string())
                        })
                        .collect::<Vec<_>>()
                        .join(".")
                })
                .unwrap_or_default();
            Some(if path.is_empty() {
                message.into()
            } else {
                format!("{path}: {message}")
            })
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}
