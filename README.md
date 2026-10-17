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

An optional [pixel diff](#pixel-diff) layer compares each shot with an
approved baseline, to catch any before/after change the expectations do not
mention.

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

## Pixel diff

```rust
shot.expect(["No toolbar label is truncated"])
    .with_diff(0.001) // fail when more than 0.1% of pixels changed
    .assert_env();
```

`with_diff(max)` compares the shot with its approved baseline,
`visual-baselines/<name>.png` at the workspace root (override with
`with_baseline_dir` or `GPUI_VISUAL_BASELINE_DIR`); commit that directory.
A pixel counts as changed when any channel differs by more than 8 of 255
(`with_diff_threshold`). A size change always fails. On a change it writes
`<name>.diff.png`, the new shot faded with changed pixels in magenta, and
names the changed area. With no baseline yet the layer is pending.

Accept an intended change with `cargo visual-test approve <name>` (or
`--all`), or rerun with `GPUI_VISUAL_UPDATE=1`. With a diff layer, a check
needs no expectations.

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
`GPUI_VISUAL_UNSURE=warn` lets undecided expectations (and missing
baselines) pass with a warning, in tests and the CLI alike. CLI flags such as
`--rejudge` only switch things on, so these variables still apply when the
flag is left out. Relative directories in these variables are from the
workspace root.

## The CLI, for agents and people

```sh
cargo install --path crates/cli   # provides `cargo visual-test`
```

| Command | Does | Exit |
| --- | --- | --- |
| `cargo visual-test check shot.png -e "…" [-e …] [--should "…"] [--region id=x,y,w,h] [--expect-in id=…] [--judge spec] [--diff max]` | Checks any PNG, against expectations, its baseline, or both | 0 pass, 1 fail, 2 pending |
| `cargo visual-test diff before.png after.png [--out diff.png] [--max share] [--json]` | Pixel-diffs two PNGs | 0 within `--max`, 1 otherwise |
| `cargo visual-test approve <name…\|--all>` | Accepts the current images as pixel-diff baselines | 0 |
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

## Integrations

Outside Rust tests, everything goes through `cargo visual-test check --json`,
so verdict reuse, agent review and pixel diffs behave the same. Each helper
finds the CLI at `$GPUI_VISUAL_TEST_CLI`, else `cargo-visual-test` on `PATH`.

### Playwright and gpui-playwright

[`integrations/playwright`](integrations/playwright) screenshots a Page or
Locator, turns locators into named regions, and checks the shot. It works
against a [gpui-playwright](https://github.com/Ziinc/gpui-playwright) app or
any web page.

```js
const { expectVisual } = require("gpui-visual-test-playwright");

test("todo list", async ({ page }) => {
  await expectVisual(page, "todo/seeded", {
    expect: ["Two todos are listed, neither checked"],
    expectIn: { add: "The Add button is enabled" },
    regions: { add: page.getByRole("button", { name: "Add" }) },
    diff: 0.001,
  });
});
```

Python (`from gpui_visual_test_playwright import expect_visual`) takes the
same options in snake_case, with `expect_visual_async` for the async API. A
failure raises `VisualCheckError`, an `AssertionError`.

### Elixir and baguette

[`integrations/elixir`](integrations/elixir) (`gpui_visual_test`) checks PNG
bytes or `{:file, path}` from ExUnit:

```elixir
import GpuiVisualTest.Assertions

assert_visual png, "counter/initial",
  expect: ["The count reads 0"],
  regions: [toolbar: {0, 0, 400, 40}],
  expect_in: [toolbar: "Save is greyed out"],
  diff: 0.001
```

`GpuiVisualTest.check/3` returns `{:ok | :fail | :pending, result}` instead of
raising.

### A running GPUI app (tauri-gpui)

`capture::window_shot(window, name)` reads back the last frame a live window
drew, for a test hook or debug command inside the app. It needs a platform
that implements `PlatformWindow::render_to_image`, as GPUI's headless
renderer does.

## Headless rendering

`capture` uses GPUI's offscreen renderer: Metal on macOS, Vulkan on Linux. On
Linux CI without a GPU, install lavapipe:

```sh
sudo apt-get install -y mesa-vulkan-drivers libvulkan-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev
```

The crate pins `gpui-pre =0.3.9`, the snapshot gpui-kit 0.7.3 and
gpui-styleguidist build against.
