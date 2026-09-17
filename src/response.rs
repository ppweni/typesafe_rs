use std::{collections::BTreeMap, ops::Deref};

use bytes::Bytes;
use reqwest::{StatusCode, header::HeaderMap};
use serde::{Deserialize, Serialize};

use crate::Content;

/// A decoded body together with HTTP metadata and the original response bytes.
#[derive(Clone, Debug)]
pub struct Response<T> {
    pub data: T,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}
impl<T> Response<T> {
    pub fn request_id(&self) -> Option<&str> {
        self.headers.get("x-typesafe-request-id")?.to_str().ok()
    }
    pub fn into_inner(self) -> T {
        self.data
    }
}
impl<T> Deref for Response<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.data
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoulAnswer {
    pub noul: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoiceAnswer {
    pub choice: String,
    pub confidence: f64,
    pub probabilities: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoreAnswer {
    pub score: f64,
    pub confidence: f64,
    #[serde(deserialize_with = "deserialize_score_map")]
    pub legend: BTreeMap<u32, Content>,
    #[serde(deserialize_with = "deserialize_score_map")]
    pub probabilities: BTreeMap<u32, f64>,
}

// Internally tagged enums buffer their fields in Serde before decoding the chosen
// variant. JSON's usual integer-map-key coercion is lost in that buffer, so parse
// the wire string keys explicitly, directly into the destination map.
fn deserialize_score_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<u32, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct ScoreMap<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for ScoreMap<T> {
        type Value = BTreeMap<u32, T>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an object keyed by nonnegative integer score levels")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut access: M,
        ) -> Result<Self::Value, M::Error> {
            let mut map = BTreeMap::new();
            while let Some(key) = access.next_key::<String>()? {
                let key = key.parse::<u32>().map_err(serde::de::Error::custom)?;
                map.insert(key, access.next_value()?);
            }
            Ok(map)
        }
    }
    deserializer.deserialize_map(ScoreMap(std::marker::PhantomData))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Answer {
    Noul(NoulAnswer),
    Choice(ChoiceAnswer),
    Score(ScoreAnswer),
    /// Future answer kinds are tolerated; their payload remains in `Response::body`.
    #[serde(other)]
    Unknown,
}
impl Answer {
    pub fn as_noul(&self) -> Option<&NoulAnswer> {
        if let Self::Noul(answer) = self {
            Some(answer)
        } else {
            None
        }
    }
    pub fn as_choice(&self) -> Option<&ChoiceAnswer> {
        if let Self::Choice(answer) = self {
            Some(answer)
        } else {
            None
        }
    }
    pub fn as_score(&self) -> Option<&ScoreAnswer> {
        if let Self::Score(answer) = self {
            Some(answer)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemOneResponse {
    pub model: String,
    pub usage: Usage,
    pub answers: BTreeMap<String, Answer>,
}
impl SystemOneResponse {
    pub fn noul(&self, name: &str) -> Option<&NoulAnswer> {
        self.answers.get(name)?.as_noul()
    }
    pub fn choice(&self, name: &str) -> Option<&ChoiceAnswer> {
        self.answers.get(name)?.as_choice()
    }
    pub fn score(&self, name: &str) -> Option<&ScoreAnswer> {
        self.answers.get(name)?.as_score()
    }

    /// Iterate over yes/no answers without allocating a second map.
    pub fn nouls(&self) -> impl Iterator<Item = (&str, &NoulAnswer)> {
        self.answers
            .iter()
            .filter_map(|(name, answer)| Some((name.as_str(), answer.as_noul()?)))
    }
    pub fn choices(&self) -> impl Iterator<Item = (&str, &ChoiceAnswer)> {
        self.answers
            .iter()
            .filter_map(|(name, answer)| Some((name.as_str(), answer.as_choice()?)))
    }
    pub fn scores(&self) -> impl Iterator<Item = (&str, &ScoreAnswer)> {
        self.answers
            .iter()
            .filter_map(|(name, answer)| Some((name.as_str(), answer.as_score()?)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub name: String,
    pub description: String,
    pub release_date: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListModelsResponse {
    pub models: Vec<ModelMetadata>,
}
