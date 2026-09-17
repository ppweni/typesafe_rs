#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod client;
mod error;
mod question;
mod response;
mod retry;

pub use client::{Client, ClientBuilder, Models, ModelsRequest, SystemOneRequest};
pub use error::{ApiError, ApiErrorKind, Error, ResponseValidationError, Result};
pub use question::{Choice, Content, Noul, NoulCriteria, Question, Questions, Score};
pub use reqwest::{StatusCode, header};
pub use response::{
    Answer, ChoiceAnswer, ListModelsResponse, ModelMetadata, NoulAnswer, Response, ScoreAnswer,
    SystemOneResponse, Usage,
};
pub use retry::RetryPolicy;
pub use serde_json::{Value, json};

/// Environment variable used when no explicit API key is supplied.
pub const API_KEY_ENV: &str = "TYPESAFE_API_KEY";
/// Environment variable used when no explicit base URL is supplied.
pub const BASE_URL_ENV: &str = "TYPESAFE_BASE_URL";
/// Environment variable used when no explicit default model is supplied.
pub const DEFAULT_MODEL_ENV: &str = "TYPESAFE_DEFAULT_MODEL";
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
pub const DEFAULT_MODEL: &str = "jev-latest";
/// Per-attempt timeout, including reading the response body.
pub const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
