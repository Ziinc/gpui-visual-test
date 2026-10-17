//! Vision-model judges: OpenAI-compatible chat completions (OpenAI, Gemini,
//! local servers) and Anthropic messages.

use super::{Judge, http, parse_verdicts, prompt, verdicts_schema};
use crate::{Expectation, Shot, Verdict};
use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use serde_json::{Value, json};

/// The API a vision model is reached through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Provider {
    /// `/chat/completions` with `response_format: json_schema`.
    OpenAiCompatible,
    /// `/v1/messages` with a forced tool call.
    Anthropic,
}

/// A vision model and how to call it.
#[derive(Clone, Debug)]
pub struct VisionModel {
    /// The spec prefix (`openai`, `gemini`, `local`, `anthropic`), for ids.
    pub vendor: String,
    pub model: String,
    pub provider: Provider,
    pub base_url: String,
    pub api_key: Option<String>,
}

const DESCRIBE: &str = "Describe this desktop UI screenshot as a JSON object, literally and \
without guessing behaviour. Include: \"text\": every visible string with its approximate position \
(x, y in logical pixels) and colour; \"controls\": buttons, inputs, checkboxes, tabs and menus with \
their label, position and visible state (enabled or greyed out, selected, checked, focused, hovered); \
\"layout\": panels, toolbars and how they are arranged; \"theme\": light or dark and the main colours; \
\"defects\": any clipped, truncated, overlapping, misaligned or unreadable element.";

impl VisionModel {
    /// `openai:<model>`, `gemini:<model>`, `local:<model>` or
    /// `anthropic:<model>`, with keys and URLs from the environment.
    pub fn from_spec(spec: &str) -> Result<Self> {
        let (vendor, model) = spec
            .split_once(':')
            .ok_or_else(|| anyhow!("judge `{spec}` should look like openai:<model>"))?;
        let var =
            |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
        let (provider, base_url, api_key) = match vendor {
            "openai" => (
                Provider::OpenAiCompatible,
                var("OPENAI_BASE_URL", "https://api.openai.com/v1"),
                Some(http::env_key("OPENAI_API_KEY")?),
            ),
            "gemini" => (
                Provider::OpenAiCompatible,
                var(
                    "GEMINI_BASE_URL",
                    "https://generativelanguage.googleapis.com/v1beta/openai",
                ),
                Some(http::env_key("GEMINI_API_KEY")?),
            ),
            "local" => (
                Provider::OpenAiCompatible,
                var("GPUI_VISUAL_LOCAL_URL", "http://localhost:11434/v1"),
                std::env::var("GPUI_VISUAL_LOCAL_KEY").ok(),
            ),
            "anthropic" => (
                Provider::Anthropic,
                var("ANTHROPIC_BASE_URL", "https://api.anthropic.com"),
                Some(http::env_key("ANTHROPIC_API_KEY")?),
            ),
            other => {
                bail!("unknown judge `{other}`; use agent, openai, gemini, local, anthropic or jev")
            }
        };
        if model.is_empty() {
            bail!("judge `{spec}` names no model");
        }
        Ok(Self {
            vendor: vendor.into(),
            model: model.into(),
            provider,
            base_url: base_url.trim_end_matches('/').into(),
            api_key,
        })
    }

    pub fn id(&self) -> String {
        format!("{}:{}", self.vendor, self.model)
    }

    /// Verdicts for `expectations`, in order.
    pub fn verdicts(&self, shot: &Shot, expectations: &[Expectation]) -> Result<Vec<Verdict>> {
        let instructions = prompt(shot, expectations);
        let answer = match self.provider {
            Provider::OpenAiCompatible => {
                let response = self.chat(
                    shot,
                    &instructions,
                    json!({
                        "type": "json_schema",
                        "json_schema": { "name": "verdicts", "strict": true, "schema": verdicts_schema() }
                    }),
                )?;
                serde_json::from_str(&response)
                    .with_context(|| format!("{} answered non-JSON", self.id()))?
            }
            Provider::Anthropic => {
                let response = self.messages(
                    shot,
                    &instructions,
                    Some(json!({
                        "name": "record_verdicts",
                        "description": "Record one verdict per expectation.",
                        "input_schema": verdicts_schema()
                    })),
                )?;
                tool_input(&response)
                    .ok_or_else(|| anyhow!("{} did not call record_verdicts", self.id()))?
            }
        };
        parse_verdicts(&answer, expectations.len(), &self.id())
    }

    /// A literal JSON description of the shot, for text judges such as Jev.
    pub fn describe(&self, shot: &Shot) -> Result<String> {
        let (width, height) = shot.logical_size();
        let instructions =
            format!("{DESCRIBE}\nThe screenshot is {width:.0}x{height:.0} logical pixels.");
        match self.provider {
            Provider::OpenAiCompatible => {
                self.chat(shot, &instructions, json!({ "type": "json_object" }))
            }
            Provider::Anthropic => {
                let response = self.messages(shot, &instructions, None)?;
                text_of(&response).ok_or_else(|| anyhow!("{} returned no description", self.id()))
            }
        }
    }

    fn chat(&self, shot: &Shot, instructions: &str, response_format: Value) -> Result<String> {
        let body = json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": instructions },
                    { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{}", png_base64(shot)?) } }
                ]
            }],
            "response_format": response_format
        });
        let auth = self.api_key.as_ref().map(|key| format!("Bearer {key}"));
        let headers: Vec<(&str, &str)> =
            auth.iter().map(|a| ("authorization", a.as_str())).collect();
        let response = http::post_json(
            &format!("{}/chat/completions", self.base_url),
            &headers,
            &body,
        )?;
        response["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("{} returned no message content", self.id()))
    }

    fn messages(&self, shot: &Shot, instructions: &str, tool: Option<Value>) -> Result<Value> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": png_base64(shot)? } },
                    { "type": "text", "text": instructions }
                ]
            }]
        });
        if let Some(tool) = tool {
            body["tool_choice"] = json!({ "type": "tool", "name": tool["name"] });
            body["tools"] = json!([tool]);
        }
        let key = self.api_key.clone().unwrap_or_default();
        http::post_json(
            &format!("{}/v1/messages", self.base_url),
            &[("x-api-key", &key), ("anthropic-version", "2023-06-01")],
            &body,
        )
    }
}

/// Longest edge, in pixels, sent to a model. Providers reject or silently
/// downscale larger images (Anthropic caps at 8000 px and 5 MB; OpenAI tiles
/// to 2048), and a retina capture of a large window easily exceeds that.
const MAX_MODEL_EDGE: u32 = 2048;

fn png_base64(shot: &Shot) -> Result<String> {
    let png = model_png(shot)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(png))
}

/// The shot as PNG, downscaled so its longest edge fits [`MAX_MODEL_EDGE`].
/// Prompts give sizes in logical pixels, so the scale change is invisible to
/// the model.
fn model_png(shot: &Shot) -> Result<Vec<u8>> {
    let (w, h) = shot.image().dimensions();
    let longest = w.max(h);
    if longest <= MAX_MODEL_EDGE {
        return shot.png();
    }
    let ratio = MAX_MODEL_EDGE as f64 / longest as f64;
    let (nw, nh) = (
        ((w as f64 * ratio).round() as u32).max(1),
        ((h as f64 * ratio).round() as u32).max(1),
    );
    let small =
        image::imageops::resize(shot.image(), nw, nh, image::imageops::FilterType::Triangle);
    Shot::new(shot.name(), small, shot.scale() * ratio as f32).png()
}

fn tool_input(response: &Value) -> Option<Value> {
    response["content"]
        .as_array()?
        .iter()
        .find(|block| block["type"] == "tool_use")
        .map(|block| block["input"].clone())
}

fn text_of(response: &Value) -> Option<String> {
    let text: String = response["content"]
        .as_array()?
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect();
    (!text.is_empty()).then_some(text)
}

/// Judges with one vision model.
pub struct VisionJudge(pub VisionModel);

impl VisionJudge {
    pub fn new(model: VisionModel) -> Self {
        Self(model)
    }
}

impl Judge for VisionJudge {
    fn id(&self) -> String {
        self.0.id()
    }

    fn judge(&self, shot: &Shot, expectations: &[Expectation]) -> Result<Vec<Verdict>> {
        self.0.verdicts(shot, expectations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_anthropic_tool_input_and_text() {
        let response = json!({"content": [
            {"type": "text", "text": "Here you go. "},
            {"type": "tool_use", "name": "record_verdicts", "input": {"verdicts": []}}
        ]});
        assert_eq!(tool_input(&response), Some(json!({"verdicts": []})));
        assert_eq!(text_of(&response).as_deref(), Some("Here you go. "));
    }

    #[test]
    fn large_shots_are_downscaled_for_models() {
        let big = Shot::new("big", image::RgbaImage::new(5000, 1000), 2.);
        let png = model_png(&big).unwrap();
        let back = image::load_from_memory(&png).unwrap();
        assert_eq!((back.width(), back.height()), (2048, 410));
        let small = Shot::new("small", image::RgbaImage::new(300, 200), 1.);
        assert_eq!(model_png(&small).unwrap(), small.png().unwrap());
    }

    #[test]
    fn local_needs_no_key() {
        let model = VisionModel::from_spec("local:qwen3-vl:8b").unwrap();
        assert_eq!(model.model, "qwen3-vl:8b");
        assert_eq!(model.id(), "local:qwen3-vl:8b");
        assert!(VisionModel::from_spec("nope:x").is_err());
        assert!(VisionModel::from_spec("local:").is_err());
    }
}
