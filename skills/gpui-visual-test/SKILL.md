---
name: gpui-visual-test
description: Review and assert gpui-visual-test screenshot checks from the command line - list pending expectations, look at each image, record pass/fail verdicts with evidence, or check any PNG against plain-English expectations.
---

# Visual checks with `cargo visual-test`

Shots live in `target/visual-test/` (or `$GPUI_VISUAL_DIR`, or `--dir`) as
`<name>.png` plus `<name>.json`. Each expectation is a claim a viewer can
confirm by looking. Exit codes everywhere: 0 pass, 1 fail, 2 needs a decision,
3 error.

## Review pending checks

1. Run the visual tests (`cargo test …`); with no model judge set they stop
   with expectations pending.
2. `cargo visual-test pending --json` lists each shot's absolute `image` path
   and its `checks` with `index`, `text`, `region` and `status`.
3. Open the image and judge each pending check from what is drawn, not from
   the code or what the UI is supposed to do. Use `regions` (logical pixels,
   origin top-left; multiply by `scale` for image pixels) when a check names
   one.
4. Record each decision with the evidence:
   `cargo visual-test verdict <name> <index> pass --reason "Save button is grey with muted text"`.
   Add `--hash <hash from pending --json>` so the verdict is refused if the
   image was re-rendered while you looked. Use `unsure` only when the image cannot settle it (too small, out of frame),
   and say why.
5. `cargo visual-test status` must exit 0. Re-run the tests: unchanged images
   reuse your verdicts.

A failing check is a real finding: report it, or fix the UI and re-run.
Never flip a verdict to pass without looking at the current image.

## Pixel diffs

A shot checked with a diff layer (`with_diff(max)`, or `check --diff max`)
carries a `diff` object in `--json` output: `status`, `reason` (share and
area changed), `baseline` (the approved PNG) and `image` (`<name>.diff.png`,
changes in magenta).

- `unsure` means no baseline yet: look at the image, and if it is right,
  `cargo visual-test approve <name>`.
- `fail` means pixels changed. Open the diff image and the shot. Approve only
  when the change is what the task intended; otherwise it is a regression to
  report or fix. Never approve to make a run green.
- `cargo visual-test diff before.png after.png --out diff.png --json`
  compares any two images.

## Check any image

```sh
cargo visual-test check out.png -e "The dialog title reads Export" \
  -e "No text overlaps the buttons" --region footer=0,520,800,80
```

Add `--judge openai:<model>` (or `anthropic:`, `gemini:`, `local:`,
`jev:<model>+<vision spec>`) to have a model decide instead; otherwise record
verdicts as above.

## From Playwright or Elixir

`expectVisual` / `expect_visual` (`integrations/playwright`) and
`assert_visual` (`integrations/elixir`) run `cargo visual-test check --json`
under the hood, so their shots land in the same directory and the review loop
above applies unchanged. They need the CLI installed, or
`$GPUI_VISUAL_TEST_CLI` pointing at it.
