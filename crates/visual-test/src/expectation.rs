//! What a shot should show, in plain English.

use serde::{Deserialize, Serialize};

/// How a failed expectation affects the check.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// A failure fails the check.
    #[default]
    Must,
    /// A failure is reported but does not fail the check.
    Should,
}

/// A claim a viewer could confirm by looking at the shot, such as "The Save
/// button is greyed out" or "No toolbar label is truncated".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Expectation {
    pub text: String,
    /// A region id of the shot this claim is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default)]
    pub severity: Severity,
}

impl Expectation {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            region: None,
            severity: Severity::Must,
        }
    }

    /// Points the claim at a named region of the shot.
    pub fn in_region(mut self, id: impl Into<String>) -> Self {
        self.region = Some(id.into());
        self
    }

    /// Reports a failure without failing the check.
    pub fn should(mut self) -> Self {
        self.severity = Severity::Should;
        self
    }
}

impl From<&str> for Expectation {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for Expectation {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}
