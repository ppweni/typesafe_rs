#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod asset;
mod body;
mod client;
mod compression;
mod error;
mod question;
mod response;
mod retry;

pub use asset::{Asset, AssetEncoding, Assets, Image};
pub use client::{Client, ClientBuilder, Models, ModelsRequest, SystemOneRequest};
pub use compression::Compression;
pub use error::{ApiError, ApiErrorKind, Error, ResponseValidationError, Result};
pub use question::{BoxQuestion, Choice, Content, Noul, NoulCriteria, Question, Questions, Score};
pub use reqwest::{StatusCode, header};
pub use response::{
    Answer, BoxAnswer, BoxCoordinates, ChoiceAnswer, ListModelsResponse, ModelMetadata, NoulAnswer,
    Response, ScoreAnswer, SystemOneResponse, Usage,
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
/// Pinned model supporting Box questions.
pub const JEV_1_14: &str = "jev-1.14.0";
/// Experimental image/Box opt-in header.
pub const IMAGES_HEADER: &str = "typesafe-danger-zone-images";
/// Experimental 262k context opt-in header.
pub const LONG_CONTEXT_HEADER: &str = "typesafe-danger-zone-long-context";
/// Per-attempt timeout, including reading the response body.
pub const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
