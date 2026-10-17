//! TypeSafe Jev as a fast decision stage.
//!
//! Jev is a "System One" model: it answers typed questions about text state
//! with calibrated probabilities, in one pass, and accepts no images. So a
//! vision model first describes the shot as JSON, Jev answers each
//! expectation as a yes/no ("noul") question over that description plus the
//! shot's attached state, and expectations Jev is unsure of go back to the
//! vision model as a full judgement.

use super::{Judge, VisionModel, http};
use crate::{Expectation, Shot, Status, Verdict};
use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

pub struct JevJudge {
    pub model: String,
    pub base_url: String,
    api_key: String,
    pub vision: VisionModel,
    /// `p(true)` at or above this passes.
    pub pass_at: f32,
    /// `p(true)` at or below this fails; anything between is escalated.
    pub fail_at: f32,
    /// Send `Unsure` answers to the vision model (default true).
    pub escalate: bool,
}

impl JevJudge {
    /// `TYPESAFE_API_KEY`, and `TYPESAFE_BASE_URL` to override the endpoint.
    pub fn from_env(model: &str, vision: VisionModel) -> Result<Self> {
        Ok(Self {
            model: model.into(),
            base_url: std::env::var("TYPESAFE_BASE_URL")
                .unwrap_or_else(|_| "https://api.typesafe.ai/v1".into())
                .trim_end_matches('/')
                .into(),
            api_key: http::env_key("TYPESAFE_API_KEY")?,
            vision,
            pass_at: 0.9,
            fail_at: 0.1,
            escalate: true,
        })
    }

    fn jev_id(&self) -> String {
        format!("jev:{}+{}", self.model, self.vision.id())
    }
}

/// The request body: the description and any app state as `state`, one
/// noul question per expectation keyed `e<index>`.
pub(crate) fn request(
    model: &str,
    shot: &Shot,
    description: &str,
    expectations: &[Expectation],
) -> Value {
    let description: Value =
        serde_json::from_str(description).unwrap_or_else(|_| Value::String(description.into()));
    let (width, height) = shot.logical_size();
    let state = json!({
        "screenshot": {
            "width": width,
            "height": height,
            "description": description,
            "regions": shot.regions(),
        },
        "app_state": shot.state(),
    });
    let questions: Map<String, Value> = expectations
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let claim = match &e.region {
                Some(region) => format!("In region `{region}`: {}", e.text),
                None => e.text.clone(),
            };
            (
                format!("e{i}"),
                json!({
                    "type": "noul",
                    "instructions": format!("Going by the screenshot description and app state, is this true? {claim}"),
                    "criteria": {
                        "true": "The description shows this clearly holds",
                        "false": "The description contradicts it or shows it does not hold"
                    }
                }),
            )
        })
        .collect();
    json!({ "model": model, "state": state.to_string(), "questions": questions })
}

/// Maps each answer's `p(true)` to a verdict by threshold.
pub(crate) fn verdicts(
    response: &Value,
    count: usize,
    pass_at: f32,
    fail_at: f32,
    judge: &str,
) -> Result<Vec<Verdict>> {
    (0..count)
        .map(|i| {
            let p = response["answers"][format!("e{i}")]["noul"]
                .as_f64()
                .ok_or_else(|| anyhow!("{judge} gave no answer for expectation {i}"))?
                as f32;
            let status = if p >= pass_at {
                Status::Pass
            } else if p <= fail_at {
                Status::Fail
            } else {
                Status::Unsure
            };
            Ok(Verdict::new(
                status,
                format!("Jev p(true) = {p:.2} over the screenshot description"),
                judge,
            )
            .with_confidence(p.max(1. - p)))
        })
        .collect()
}

impl Judge for JevJudge {
    fn id(&self) -> String {
        self.jev_id()
    }

    fn judge(&self, shot: &Shot, expectations: &[Expectation]) -> Result<Vec<Verdict>> {
        let description = self.vision.describe(shot)?;
        let body = request(&self.model, shot, &description, expectations);
        let auth = format!("Bearer {}", self.api_key);
        let response = http::post_json(
            &format!("{}/systemone", self.base_url),
            &[("authorization", &auth)],
            &body,
        )?;
        let mut verdicts = verdicts(
            &response,
            expectations.len(),
            self.pass_at,
            self.fail_at,
            &self.jev_id(),
        )?;

        let unsure: Vec<usize> = (0..verdicts.len())
            .filter(|&i| verdicts[i].status == Status::Unsure)
            .collect();
        if self.escalate && !unsure.is_empty() {
            let asked: Vec<Expectation> = unsure.iter().map(|&i| expectations[i].clone()).collect();
            for (i, verdict) in unsure.into_iter().zip(self.vision.verdicts(shot, &asked)?) {
                verdicts[i] = verdict;
            }
        }
        Ok(verdicts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    #[test]
    fn builds_one_noul_question_per_expectation() {
        let shot = Shot::new("s", RgbaImage::new(100, 50), 1.).with_state(json!({"saved": false}));
        let body = request(
            "jev-1.13.0",
            &shot,
            r#"{"text": [{"value": "Save"}]}"#,
            &[
                "Save is disabled".into(),
                Expectation::new("No text is clipped").in_region("toolbar"),
            ],
        );
        assert_eq!(body["model"], "jev-1.13.0");
        assert_eq!(body["questions"]["e0"]["type"], "noul");
        assert!(
            body["questions"]["e1"]["instructions"]
                .as_str()
                .unwrap()
                .contains("In region `toolbar`")
        );
        let state: Value = serde_json::from_str(body["state"].as_str().unwrap()).unwrap();
        assert_eq!(
            state["screenshot"]["description"]["text"][0]["value"],
            "Save"
        );
        assert_eq!(state["app_state"]["saved"], false);
    }

    #[test]
    fn thresholds_split_pass_fail_unsure() {
        let response = json!({"model": "jev-1.13.0", "answers": {
            "e0": {"type": "noul", "noul": 0.95},
            "e1": {"type": "noul", "noul": 0.04},
            "e2": {"type": "noul", "noul": 0.5}
        }});
        let v = verdicts(&response, 3, 0.9, 0.1, "jev").unwrap();
        assert_eq!(
            v.iter().map(|v| v.status).collect::<Vec<_>>(),
            [Status::Pass, Status::Fail, Status::Unsure]
        );
        assert!((v[1].confidence.unwrap() - 0.96).abs() < 1e-5);
        assert!(verdicts(&response, 4, 0.9, 0.1, "jev").is_err());
    }
}
