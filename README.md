# gpui-visual-test

Visual tests for [GPUI](https://www.gpui.rs) judged in plain English. Render a
view headlessly (or load any PNG), list what a viewer should be able to confirm
by looking at it, and let a judge decide:

- **an agent or a person**, through the `cargo visual-test` CLI (the default,
  no API key);
- **a vision model**: OpenAI, Gemini, Anthropic, or any OpenAI-compatible
  local server (Ollama, vLLM);
- **[TypeSafe Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev)
  in front of a vision model**: the vision model describes the shot once, Jev
  answers each expectation with a probability, and only what Jev is unsure of
  goes back to the vision model.

The [documentation site](website/) (`cd website && npm install && npm start`) covers everything on one page.

Verdicts are stored next to each image and reused while the image is
unchanged, so a model is called (or an agent asked) only when a shot changes.

## In a test

```toml
[dev-dependencies]
gpui-visual-test = { version = "0.1", features = ["capture", "judges"] }
```

```rust
use gpui_visual_test::{Expectation, capture};

#[test]
fn toolbar() {
    let mut cx = capture::headless_app(None).unwrap();
    cx.update(|cx| gpui_kit::init(cx));
    let shot = capture::render_view(&mut cx, "editor/toolbar", 800., 120., |window, cx| {
        cx.new(|cx| Toolbar::new(window, cx))
    })
    .unwrap()
    .with_region("actions", 600., 0., 200., 40.);

    shot.expect([
        Expectation::new("The Save button is greyed out").in_region("actions"),
        Expectation::new("No toolbar label is truncated"),
        Expectation::new("Icons are vertically centred").should(),
    ])
    .assert_env(); // judge from GPUI_VISUAL_JUDGE, default `agent`
}
```

For windows opened another way (such as `gpui_kit::open_window`, which wraps
content in its Root), open the window inside `cx.update` and call
`capture::capture_window(&mut cx, handle.into(), "name")`.

`Shot::open("file.png")` checks any image, GPUI or not. `.with_state(json)`
attaches non-visual state (an element tree, a view model) for Jev to read.

Each check writes `<name>.png` and `<name>.json` to `target/visual-test/`
(override with `GPUI_VISUAL_DIR`) and refreshes `index.html` there, failing
and pending shots first.

## Judges

Set `GPUI_VISUAL_JUDGE` (tests) or `--judge` (CLI):

| Spec | Needs |
| --- | --- |
| `agent` (default) | Nothing. Expectations stay pending until recorded with the CLI. |
| `openai:<model>`, e.g. `openai:gpt-6-astra` | `OPENAI_API_KEY` (`OPENAI_BASE_URL` to override) |
| `gemini:<model>` | `GEMINI_API_KEY` |
| `anthropic:<model>`, e.g. `anthropic:claude-opus-5-5` | `ANTHROPIC_API_KEY` |
| `local:<model>`, e.g. `local:qwen3-vl:8b` | An OpenAI-compatible server at `GPUI_VISUAL_LOCAL_URL` (default `http://localhost:11434/v1`) |
| `jev:<model>+<vision spec>`, e.g. `jev:jev-1.13.0+openai:gpt-6-astra` | `TYPESAFE_API_KEY`, plus the vision model's key |

Model judges answer in a fixed JSON schema (Structured Outputs, or a forced
tool call on Anthropic); an answer outside it is an error, never a silent pass.
Jev's `p(true)` passes at 0.9 or above and fails at 0.1 or below.

Other knobs: `GPUI_VISUAL_REJUDGE=1` ignores stored verdicts;
`GPUI_VISUAL_UNSURE=warn` lets undecided expectations pass with a warning.

## The CLI, for agents and people

```sh
cargo install --path crates/cli   # provides `cargo visual-test`
```

| Command | Does | Exit |
| --- | --- | --- |
| `cargo visual-test check shot.png -e "…" [-e …] [--should "…"] [--region id=x,y,w,h] [--judge spec]` | Checks any PNG | 0 pass, 1 fail, 2 pending |
| `cargo visual-test pending [--json]` | Lists undecided expectations, the absolute image path, and the command to record each | 2 if any |
| `cargo visual-test verdict <name> <index\|all> pass\|fail --reason "…"` | Records a decision after looking at the image | 0 |
| `cargo visual-test judge [names…] --judge spec [--rejudge]` | Runs a model over stored shots | 0/1/2 |
| `cargo visual-test status [--json]` | Overall result | 0/1/2 |
| `cargo visual-test list [--status fail] [--json]`, `show <name>`, `report` | Browse; `report` prints the `index.html` path | 0 |

The agent loop: run the tests (or `check`), then `pending --json`, open each
`image`, decide each expectation, `verdict` it with the evidence, and run
`status`. Recorded verdicts are reused while the image is unchanged, so the
next test run passes without asking again. See
[`skills/gpui-visual-test/SKILL.md`](skills/gpui-visual-test/SKILL.md).

## Headless rendering

`capture` uses GPUI's offscreen renderer: Metal on macOS, Vulkan on Linux. On
Linux CI without a GPU, install lavapipe:

```sh
sudo apt-get install -y mesa-vulkan-drivers libvulkan-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev
```

The crate pins `gpui-pre =0.3.9`, the snapshot gpui-kit 0.7.3 and
gpui-styleguidist build against.
