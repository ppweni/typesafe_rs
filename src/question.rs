use std::collections::BTreeMap;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::{Error, Result};

/// Text or structured JSON content. Bare numbers, booleans, and null are not content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Object(Map<String, Value>),
    Array(Vec<Value>),
}

impl From<String> for Content {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl From<&str> for Content {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}
impl From<Map<String, Value>> for Content {
    fn from(value: Map<String, Value>) -> Self {
        Self::Object(value)
    }
}
impl From<Vec<Value>> for Content {
    fn from(value: Vec<Value>) -> Self {
        Self::Array(value)
    }
}
impl TryFrom<Value> for Content {
    type Error = Error;
    fn try_from(value: Value) -> Result<Self> {
        match value {
            Value::String(text) => Ok(Self::Text(text)),
            Value::Object(object) => Ok(Self::Object(object)),
            Value::Array(array) => Ok(Self::Array(array)),
            _ => Err(Error::InvalidInput(
                "content must be text, an object, or an array".into(),
            )),
        }
    }
}

/// Optional descriptions of the yes and no outcomes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NoulCriteria {
    #[serde(rename = "true", skip_serializing_if = "Option::is_none")]
    pub yes: Option<Content>,
    #[serde(rename = "false", skip_serializing_if = "Option::is_none")]
    pub no: Option<Content>,
}

/// A yes/no question whose answer is a probability.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Noul {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criteria: Option<NoulCriteria>,
}
impl Noul {
    pub fn new(instructions: impl Into<Content>) -> Self {
        Self {
            instructions: Some(instructions.into()),
            criteria: None,
        }
    }
    pub fn criteria(mut self, criteria: NoulCriteria) -> Self {
        self.criteria = Some(criteria);
        self
    }
}

/// A selection between named alternatives; `None` leaves a label undescribed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Content>,
    pub criteria: BTreeMap<String, Option<Content>>,
}
impl Choice {
    /// Build a choice with undescribed labels. Use `criteria` for descriptions.
    pub fn new<I, S>(labels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            instructions: None,
            criteria: labels.into_iter().map(|s| (s.into(), None)).collect(),
        }
    }
    pub fn instructions(mut self, instructions: impl Into<Content>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }
}

/// An ordered rubric, starting at zero. An empty rubric is rejected before sending.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Score {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Content>,
    pub criteria: Vec<Content>,
}
impl Score {
    pub fn new<I, C>(criteria: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<Content>,
    {
        Self {
            instructions: None,
            criteria: criteria.into_iter().map(Into::into).collect(),
        }
    }
    pub fn instructions(mut self, instructions: impl Into<Content>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Question {
    Noul(Noul),
    Choice(Choice),
    Score(Score),
    /// Escape hatch for future question kinds and additional wire fields.
    /// Must be an object with a nonempty string `type`.
    Raw(Value),
}
pub type Questions = BTreeMap<String, Question>;

impl From<Noul> for Question {
    fn from(value: Noul) -> Self {
        Self::Noul(value)
    }
}
impl From<Choice> for Question {
    fn from(value: Choice) -> Self {
        Self::Choice(value)
    }
}
impl From<Score> for Question {
    fn from(value: Score) -> Self {
        Self::Score(value)
    }
}

impl Serialize for Question {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Tagged<'a, T> {
            #[serde(rename = "type")]
            kind: &'static str,
            #[serde(flatten)]
            fields: &'a T,
        }
        match self {
            Self::Noul(fields) => Tagged {
                kind: "noul",
                fields,
            }
            .serialize(serializer),
            Self::Choice(fields) => Tagged {
                kind: "choice",
                fields,
            }
            .serialize(serializer),
            Self::Score(fields) => Tagged {
                kind: "score",
                fields,
            }
            .serialize(serializer),
            Self::Raw(value) => value.serialize(serializer),
        }
    }
}
impl Question {
    pub(crate) fn validate(&self, name: &str) -> Result<()> {
        let invalid = |message: &str| Error::InvalidInput(format!("question {name:?}: {message}"));
        match self {
            Self::Score(score) if score.criteria.is_empty() => {
                Err(invalid("at least one score criterion is required"))
            }
            Self::Raw(value) => {
                let kind = value
                    .get("type")
                    .and_then(Value::as_str)
                    .filter(|kind| !kind.is_empty())
                    .ok_or_else(|| invalid("expected an object with a nonempty string type"))?;
                if matches!(kind, "choice" | "score") && value.get("criteria").is_none() {
                    return Err(invalid("criteria are required"));
                }
                if kind == "score" && value["criteria"].as_array().is_none_or(|v| v.is_empty()) {
                    return Err(invalid("score criteria must be a nonempty array"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
