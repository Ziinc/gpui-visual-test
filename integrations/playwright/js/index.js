// Visual checks for Playwright screenshots, judged by gpui-visual-test.
//
// Takes the screenshot, turns locators into named regions, and runs
// `cargo visual-test check --json`, so expectations, verdict reuse, agent
// review and pixel diffs work the same as in Rust tests.

"use strict";

const { execFile } = require("node:child_process");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

/** Exit codes of `cargo visual-test`. */
const EXIT = { pass: 0, fail: 1, pending: 2, error: 3 };

/**
 * Checks a page or locator screenshot and returns the CLI's JSON manifest,
 * plus `exitCode`. Never throws on a failed or pending check.
 */
async function checkVisual(target, name, options = {}) {
  if (!name) throw new Error("checkVisual needs a shot name, such as 'todo/empty'");
  const box = typeof target.boundingBox === "function" ? await target.boundingBox() : null;
  if (box === null && typeof target.boundingBox === "function") {
    throw new Error(`checkVisual('${name}'): the locator is not visible`);
  }
  const png = await target.screenshot({ ...options.screenshot, type: "png" });
  const scale = await scaleOf(target, box, png);

  const regions = [];
  for (const [id, area] of Object.entries(options.regions ?? {})) {
    const rect = typeof area.boundingBox === "function" ? await area.boundingBox() : area;
    if (!rect) throw new Error(`checkVisual('${name}'): region '${id}' is not visible`);
    // Locator boxes are page coordinates; shots of a locator start at its box.
    const x = rect.x - (box?.x ?? 0);
    const y = rect.y - (box?.y ?? 0);
    regions.push(`${id}=${x},${y},${rect.width},${rect.height}`);
  }

  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "gpui-visual-test-"));
  const file = path.join(tmp, "shot.png");
  try {
    await fs.writeFile(file, png);
    const args = ["check", file, "--name", name, "--scale", String(scale), "--json"];
    for (const text of options.expect ?? []) args.push("--expect", text);
    for (const text of options.should ?? []) args.push("--should", text);
    for (const [region, texts] of Object.entries(options.expectIn ?? {})) {
      for (const text of [].concat(texts)) args.push("--expect-in", `${region}=${text}`);
    }
    for (const region of regions) args.push("--region", region);
    // Baseline options mean nothing without a diff, so shared defaults can
    // carry them.
    if (options.diff !== undefined) {
      args.push("--diff", String(options.diff));
      if (options.baselineDir) args.push("--baseline-dir", options.baselineDir);
      if (options.updateBaseline) args.push("--update-baseline");
    }
    if (options.judge) args.push("--judge", options.judge);
    if (options.rejudge) args.push("--rejudge");
    if (options.dir) args.unshift("--dir", options.dir);
    const { code, stdout, stderr } = await run(cli(options), args);
    if (code === EXIT.error || !stdout.trim()) {
      throw new Error(`cargo visual-test failed (exit ${code}): ${stderr.trim()}`);
    }
    return { ...JSON.parse(stdout), exitCode: code };
  } finally {
    await fs.rm(tmp, { recursive: true, force: true });
  }
}

/**
 * {@link checkVisual}, throwing a readable error unless the check passes.
 * Pending checks pass only when GPUI_VISUAL_UNSURE=warn.
 */
async function expectVisual(target, name, options = {}) {
  const result = await checkVisual(target, name, options);
  if (result.exitCode === EXIT.pass) return result;
  throw new Error(describe(result));
}

function describe(result) {
  const lines = [`visual check '${result.name}' ${result.status}: ${result.image}`];
  if (result.diff && result.diff.status !== "pass") {
    lines.push(`  [diff] ${result.diff.status}: ${result.diff.reason}`);
    if (result.diff.image) lines.push(`         diff image: ${result.diff.image}`);
  }
  for (const check of result.checks) {
    if (check.status === "pass") continue;
    const reason = check.verdict ? ` (${check.verdict.reason})` : "";
    lines.push(`  [${check.index}] ${check.status}: ${check.text}${reason}`);
  }
  if (result.exitCode === EXIT.pending) {
    lines.push(
      "Look at the image, then record decisions with `cargo visual-test verdict` " +
        "(or approve a baseline with `cargo visual-test approve`), " +
        "or set GPUI_VISUAL_JUDGE to a model judge.",
    );
  }
  return lines.join("\n");
}

/** Physical pixels per logical pixel of the screenshot. */
async function scaleOf(target, box, png) {
  const width = png.readUInt32BE(16); // IHDR width
  let logical = box?.width;
  if (logical === undefined && typeof target.viewportSize === "function") {
    logical = target.viewportSize()?.width;
  }
  return logical ? width / logical : 1;
}

function cli(options) {
  return options.cli ?? process.env.GPUI_VISUAL_TEST_CLI ?? "cargo-visual-test";
}

function run(file, args) {
  return new Promise((resolve, reject) => {
    execFile(file, args, { maxBuffer: 64 * 1024 * 1024 }, (error, stdout, stderr) => {
      if (error && typeof error.code !== "number") {
        reject(new Error(`could not run ${file}: ${error.message}. Install it with ` +
          "`cargo install --git https://github.com/Ziinc/gpui-visual-test cargo-visual-test` " +
          "or set GPUI_VISUAL_TEST_CLI."));
        return;
      }
      resolve({ code: error ? error.code : 0, stdout, stderr });
    });
  });
}

module.exports = { checkVisual, expectVisual, EXIT };
