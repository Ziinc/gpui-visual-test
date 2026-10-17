//! Judges decide whether each expectation holds for a shot.
//!
//! Every judge returns one [`Verdict`] per expectation, in order. Model judges
//! are asked for a fixed JSON shape and a malformed answer is an error, never
//! a silent pass.

use crate::{Expectation, Shot, Status, Verdict};
use anyhow::Result;

#[cfg(feature = "judges")]
mod http;
#[cfg(feature = "judges")]
mod jev;
#[cfg(feature = "judges")]
mod vision;

#[cfg(feature = "judges")]
pub use jev::JevJudge;
#[cfg(feature = "judges")]
pub use vision::{Provider, VisionJudge, VisionModel};

pub trait Judge: Send + Sync {
    /// Recorded on each verdict, such as `openai:gpt-6-astra`.
    fn id(&self) -> String;

    /// One verdict per expectation, in the same order.
    fn judge(&self, shot: &Shot, expectations: &[Expectation]) -> Result<Vec<Verdict>>;
}

impl<J: Judge + ?Sized> Judge for Box<J> {
    fn id(&self) -> String {
        (**self).id()
    }

    fn judge(&self, shot: &Shot, expectations: &[Expectation]) -> Result<Vec<Verdict>> {
        (**self).judge(shot, expectations)
    }
}

/// Leaves every expectation pending for an agent or person to review with
/// `cargo visual-test pending` and `cargo visual-test verdict`. Needs no API
/// key, so it is the default.
pub struct AgentJudge;

impl Judge for AgentJudge {
    fn id(&self) -> String {
        "agent".into()
    }

    fn judge(&self, _: &Shot, expectations: &[Expectation]) -> Result<Vec<Verdict>> {
        Ok(expectations
            .iter()
            .map(|_| Verdict::pending("agent"))
            .collect())
    }
}

/// A judge from a closure, for tests and custom rules.
pub struct FnJudge<F> {
    id: String,
    judge: F,
}

impl<F> FnJudge<F>
where
    F: Fn(&Shot, &[Expectation]) -> Result<Vec<Verdict>> + Send + Sync,
{
    pub fn new(id: impl Into<String>, judge: F) -> Self {
        Self {
            id: id.into(),
            judge,
        }
    }
}

impl<F> Judge for FnJudge<F>
where
    F: Fn(&Shot, &[Expectation]) -> Result<Vec<Verdict>> + Send + Sync,
{
    fn id(&self) -> String {
        self.id.clone()
    }

    fn judge(&self, shot: &Shot, expectations: &[Expectation]) -> Result<Vec<Verdict>> {
        (self.judge)(shot, expectations)
    }
}

/// Asks `first`, then sends only its `Unsure` answers to `then`. Puts a cheap
/// judge in front of a strong one.
pub struct Cascade<A, B> {
    pub first: A,
    pub then: B,
}

impl<A: Judge, B: Judge> Judge for Cascade<A, B> {
    fn id(&self) -> String {
        format!("{}>{}", self.first.id(), self.then.id())
    }

    fn judge(&self, shot: &Shot, expectations: &[Expectation]) -> Result<Vec<Verdict>> {
        let mut verdicts = self.first.judge(shot, expectations)?;
        let unsure: Vec<usize> = (0..verdicts.len())
            .filter(|&i| verdicts[i].status == Status::Unsure)
            .collect();
        if unsure.is_empty() {
            return Ok(verdicts);
        }
        let asked: Vec<Expectation> = unsure.iter().map(|&i| expectations[i].clone()).collect();
        let answers = self.then.judge(shot, &asked)?;
        anyhow::ensure!(
            answers.len() == asked.len(),
            "{} returned {} verdicts for {} expectations",
            self.then.id(),
            answers.len(),
            asked.len()
        );
        for (i, verdict) in unsure.into_iter().zip(answers) {
            verdicts[i] = verdict;
        }
        Ok(verdicts)
    }
}

/// A judge from a spec string:
///
/// - `agent`: pending until an agent or person records verdicts (default)
/// - `openai:<model>`: OpenAI, `OPENAI_API_KEY` (`OPENAI_BASE_URL` to override)
/// - `gemini:<model>`: Gemini's OpenAI-compatible API, `GEMINI_API_KEY`
/// - `local:<model>`: any OpenAI-compatible server (Ollama, vLLM),
///   `GPUI_VISUAL_LOCAL_URL` (default `http://localhost:11434/v1`)
/// - `anthropic:<model>`: Anthropic, `ANTHROPIC_API_KEY`
/// - `jev:<model>+<vision spec>`: the vision model describes the shot once,
///   TypeSafe Jev answers each expectation over that description
///   (`TYPESAFE_API_KEY`), and the vision model rejudges what Jev is unsure of
pub fn from_spec(spec: &str) -> Result<Box<dyn Judge>> {
    let spec = spec.trim();
    if spec == "agent" {
        return Ok(Box::new(AgentJudge));
    }
    model_judge(spec)
}

#[cfg(feature = "judges")]
fn model_judge(spec: &str) -> Result<Box<dyn Judge>> {
    if let Some(rest) = spec.strip_prefix("jev:") {
        let (model, vision) = rest.split_once('+').ok_or_else(|| {
            anyhow::anyhow!(
                "Jev reads text, not images; name the vision model that describes the shot: jev:{rest}+openai:<model>"
            )
        })?;
        let vision = VisionModel::from_spec(vision)?;
        return Ok(Box::new(JevJudge::from_env(model, vision)?));
    }
    Ok(Box::new(VisionJudge::new(VisionModel::from_spec(spec)?)))
}

#[cfg(not(feature = "judges"))]
fn model_judge(spec: &str) -> Result<Box<dyn Judge>> {
    anyhow::bail!("judge `{spec}` needs the `judges` feature of gpui-visual-test")
}

/// The judge named by `GPUI_VISUAL_JUDGE`, or [`AgentJudge`].
pub fn from_env() -> Result<Box<dyn Judge>> {
    match std::env::var("GPUI_VISUAL_JUDGE") {
        Ok(spec) if !spec.trim().is_empty() => from_spec(&spec),
        _ => Ok(Box::new(AgentJudge)),
    }
}

/// The instructions every vision judge gets, with the expectations numbered.
pub fn prompt(shot: &Shot, expectations: &[Expectation]) -> String {
    let (width, height) = shot.logical_size();
    let mut text = format!(
        "You are a visual QA judge for a screenshot of a desktop application, \
         {width:.0}x{height:.0} logical pixels at scale {}.\n\
         For each numbered expectation, decide from the image alone whether it holds:\n\
         - pass: clearly true in the image\n\
         - fail: clearly false, or contradicted by the image\n\
         - unsure: the image cannot settle it (too small, ambiguous, out of frame)\n\
         Give a one-sentence reason that cites what is visible, and a confidence from 0 to 1. \
         Judge only what is drawn; do not assume behaviour you cannot see.\n",
        shot.scale()
    );
    if !shot.regions().is_empty() {
        text.push_str("\nNamed regions (logical pixels, origin top-left):\n");
        for r in shot.regions() {
            text.push_str(&format!(
                "- {}: x={} y={} width={} height={}\n",
                r.id, r.x, r.y, r.width, r.height
            ));
        }
    }
    text.push_str("\nExpectations:\n");
    for (i, e) in expectations.iter().enumerate() {
        match &e.region {
            Some(region) => text.push_str(&format!("{i}. [in region {region}] {}\n", e.text)),
            None => text.push_str(&format!("{i}. {}\n", e.text)),
        }
    }
    text
}

/// JSON Schema of a vision judge's answer.
pub fn verdicts_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["verdicts"],
        "properties": {
            "verdicts": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["index", "status", "reason", "confidence"],
                    "properties": {
                        "index": { "type": "integer" },
                        "status": { "type": "string", "enum": ["pass", "fail", "unsure"] },
                        "reason": { "type": "string" },
                        "confidence": { "type": "number" }
                    }
                }
            }
        }
    })
}

/// Reads an answer shaped like [`verdicts_schema`]. An expectation the model
/// skipped comes back `Unsure`; anything malformed is an error.
pub fn parse_verdicts(
    answer: &serde_json::Value,
    count: usize,
    judge: &str,
) -> Result<Vec<Verdict>> {
    #[derive(serde::Deserialize)]
    struct Answer {
        verdicts: Vec<Item>,
    }
    #[derive(serde::Deserialize)]
    struct Item {
        index: usize,
        status: Status,
        reason: String,
        #[serde(default)]
        confidence: Option<f32>,
    }
    let answer: Answer = serde_json::from_value(answer.clone())
        .map_err(|e| anyhow::anyhow!("{judge} answered outside the verdict schema: {e}"))?;
    let mut verdicts: Vec<Verdict> = (0..count)
        .map(|_| Verdict::new(Status::Unsure, "the judge gave no answer", judge))
        .collect();
    for item in answer.verdicts {
        let Some(slot) = verdicts.get_mut(item.index) else {
            anyhow::bail!("{judge} answered expectation {} of {count}", item.index);
        };
        let mut verdict = Verdict::new(item.status, item.reason, judge);
        if let Some(confidence) = item.confidence {
            verdict = verdict.with_confidence(confidence);
        }
        *slot = verdict;
    }
    Ok(verdicts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;
    use serde_json::json;

    #[test]
    fn parses_verdicts_and_marks_skipped_unsure() {
        let answer = json!({"verdicts": [
            {"index": 1, "status": "fail", "reason": "label is cut off", "confidence": 0.9}
        ]});
        let verdicts = parse_verdicts(&answer, 2, "m").unwrap();
        assert_eq!(verdicts[0].status, Status::Unsure);
        assert_eq!(verdicts[1].status, Status::Fail);
        assert_eq!(verdicts[1].confidence, Some(0.9));
        assert!(parse_verdicts(&json!({"verdicts": [{"index": 5, "status": "pass", "reason": "", "confidence": 1}]}), 2, "m").is_err());
        assert!(parse_verdicts(&json!({"verdicts": [{"index": 0, "status": "maybe", "reason": "", "confidence": 1}]}), 1, "m").is_err());
    }

    #[test]
    fn cascade_escalates_only_unsure() {
        let shot = Shot::new("s", RgbaImage::new(2, 2), 1.);
        let cascade = Cascade {
            first: FnJudge::new("fast", |_, _| {
                Ok(vec![
                    Verdict::new(Status::Pass, "yes", "fast"),
                    Verdict::new(Status::Unsure, "?", "fast"),
                ])
            }),
            then: FnJudge::new("slow", |_, asked: &[Expectation]| {
                assert_eq!(asked.len(), 1);
                assert_eq!(asked[0].text, "b");
                Ok(vec![Verdict::new(Status::Fail, "no", "slow")])
            }),
        };
        let verdicts = cascade.judge(&shot, &["a".into(), "b".into()]).unwrap();
        assert_eq!(verdicts[0].judge, "fast");
        assert_eq!(verdicts[1].status, Status::Fail);
        assert_eq!(cascade.id(), "fast>slow");
    }

    #[test]
    fn prompt_numbers_expectations_and_regions() {
        let shot =
            Shot::new("s", RgbaImage::new(200, 100), 2.).with_region("toolbar", 0., 0., 100., 20.);
        let text = prompt(
            &shot,
            &[Expectation::new("Save is disabled").in_region("toolbar")],
        );
        assert!(text.contains("100x50 logical pixels"));
        assert!(text.contains("- toolbar: x=0 y=0 width=100 height=20"));
        assert!(text.contains("0. [in region toolbar] Save is disabled"));
    }

    #[test]
    fn agent_is_the_default_judge() {
        assert_eq!(from_spec("agent").unwrap().id(), "agent");
    }
}
