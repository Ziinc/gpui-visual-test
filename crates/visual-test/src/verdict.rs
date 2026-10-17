//! A judge's answer for one expectation.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Fail,
    /// The judge could not tell, or has not looked yet (agent review pending).
    Unsure,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Unsure => "unsure",
        })
    }
}

impl std::str::FromStr for Status {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> anyhow::Result<Self> {
        match text {
            "pass" => Ok(Status::Pass),
            "fail" => Ok(Status::Fail),
            "unsure" => Ok(Status::Unsure),
            other => anyhow::bail!("unknown status `{other}`; use pass, fail or unsure"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub status: Status,
    /// Why, in a sentence, citing what is visible.
    pub reason: String,
    /// 0 to 1, when the judge reports one (Jev's probability, a model's
    /// self-rating).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// The judge that gave it, such as `openai:gpt-6-astra` or `agent`.
    pub judge: String,
}

impl Verdict {
    pub fn new(status: Status, reason: impl Into<String>, judge: impl Into<String>) -> Self {
        Self {
            status,
            reason: reason.into(),
            confidence: None,
            judge: judge.into(),
        }
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = Some(confidence.clamp(0., 1.));
        self
    }

    /// The verdict a judge gives when it has not looked.
    pub fn pending(judge: impl Into<String>) -> Self {
        Self::new(Status::Unsure, "awaiting review", judge)
    }
}
