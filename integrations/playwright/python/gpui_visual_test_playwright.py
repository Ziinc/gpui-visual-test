"""Visual checks for Playwright screenshots, judged by gpui-visual-test.

Takes the screenshot, turns locators into named regions, and runs
``cargo visual-test check --json``, so expectations, verdict reuse, agent
review and pixel diffs work the same as in Rust tests. Works with Playwright's
sync and async APIs, against gpui-playwright or any page.
"""

from __future__ import annotations

import asyncio
import json
import os
import shutil
import struct
import subprocess
import tempfile
from typing import Any

__all__ = [
    "VisualCheckError",
    "check_visual",
    "expect_visual",
    "check_visual_async",
    "expect_visual_async",
]

EXIT_PASS, EXIT_FAIL, EXIT_PENDING, EXIT_ERROR = 0, 1, 2, 3


class VisualCheckError(AssertionError):
    """A visual check that failed or still needs a decision."""

    def __init__(self, result: dict[str, Any]):
        self.result = result
        super().__init__(_describe(result))


def check_visual(target, name: str, **options) -> dict[str, Any]:
    """Checks a sync-API page or locator; returns the manifest plus ``exit_code``."""
    box = None
    if hasattr(target, "bounding_box"):
        box = target.bounding_box()
        if box is None:
            raise ValueError(f"check_visual({name!r}): the locator is not visible")
    png = target.screenshot(**options.get("screenshot", {}))
    viewport = _viewport(target)
    regions = {
        rid: area.bounding_box() if hasattr(area, "bounding_box") else area
        for rid, area in options.get("regions", {}).items()
    }
    args = _args(name, png, box, viewport, regions, options)
    return _finish(*_run_sync(_cli(options), args))


def expect_visual(target, name: str, **options) -> dict[str, Any]:
    """:func:`check_visual`, raising :class:`VisualCheckError` unless it passes."""
    return _require(check_visual(target, name, **options))


async def check_visual_async(target, name: str, **options) -> dict[str, Any]:
    """Checks an async-API page or locator."""
    box = None
    if hasattr(target, "bounding_box"):
        box = await target.bounding_box()
        if box is None:
            raise ValueError(f"check_visual({name!r}): the locator is not visible")
    png = await target.screenshot(**options.get("screenshot", {}))
    viewport = _viewport(target)
    regions = {}
    for rid, area in options.get("regions", {}).items():
        regions[rid] = await area.bounding_box() if hasattr(area, "bounding_box") else area
    args = _args(name, png, box, viewport, regions, options)
    return _finish(*await _run_async(_cli(options), args))


async def expect_visual_async(target, name: str, **options) -> dict[str, Any]:
    return _require(await check_visual_async(target, name, **options))


def _viewport(target):
    size = getattr(target, "viewport_size", None)
    return size() if callable(size) else size


def _args(name, png, box, viewport, regions, options):
    if not name:
        raise ValueError("check_visual needs a shot name, such as 'todo/empty'")
    for rid, rect in regions.items():
        if rect is None:
            raise ValueError(f"check_visual({name!r}): region {rid!r} is not visible")
    width = struct.unpack(">I", png[16:20])[0]  # IHDR width
    logical = (box or {}).get("width") or (viewport or {}).get("width")
    scale = width / logical if logical else 1.0

    fd, path = tempfile.mkstemp(prefix="gpui-visual-test-", suffix=".png")
    with os.fdopen(fd, "wb") as f:
        f.write(png)
    args = ["check", path, "--name", name, "--scale", str(scale), "--json"]
    for text in options.get("expect", []):
        args += ["--expect", text]
    for text in options.get("should", []):
        args += ["--should", text]
    for region, texts in options.get("expect_in", {}).items():
        for text in [texts] if isinstance(texts, str) else texts:
            args += ["--expect-in", f"{region}={text}"]
    for rid, rect in regions.items():
        # Locator boxes are page coordinates; a locator shot starts at its box.
        x = rect["x"] - (box or {}).get("x", 0)
        y = rect["y"] - (box or {}).get("y", 0)
        args += ["--region", f"{rid}={x},{y},{rect['width']},{rect['height']}"]
    # Baseline options mean nothing without a diff, so shared defaults can
    # carry them.
    if options.get("diff") is not None:
        args += ["--diff", str(options["diff"])]
        if options.get("baseline_dir"):
            args += ["--baseline-dir", str(options["baseline_dir"])]
        if options.get("update_baseline"):
            args.append("--update-baseline")
    if options.get("judge"):
        args += ["--judge", options["judge"]]
    if options.get("rejudge"):
        args.append("--rejudge")
    if options.get("dir"):
        args = ["--dir", str(options["dir"])] + args
    return args, path


def _cli(options):
    return options.get("cli") or os.environ.get("GPUI_VISUAL_TEST_CLI") or "cargo-visual-test"


def _missing(cli):
    return FileNotFoundError(
        f"could not run {cli}. Install it with `cargo install --git "
        "https://github.com/Ziinc/gpui-visual-test cargo-visual-test` or set GPUI_VISUAL_TEST_CLI."
    )


def _run_sync(cli, prepared):
    args, path = prepared
    try:
        if shutil.which(cli) is None and not os.path.exists(cli):
            raise _missing(cli)
        done = subprocess.run([cli, *args], capture_output=True, text=True)
        return done.returncode, done.stdout, done.stderr
    finally:
        os.unlink(path)


async def _run_async(cli, prepared):
    args, path = prepared
    try:
        if shutil.which(cli) is None and not os.path.exists(cli):
            raise _missing(cli)
        proc = await asyncio.create_subprocess_exec(
            cli, *args, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
        )
        out, err = await proc.communicate()
        return proc.returncode, out.decode(), err.decode()
    finally:
        os.unlink(path)


def _finish(code, stdout, stderr):
    if code == EXIT_ERROR or not stdout.strip():
        raise RuntimeError(f"cargo visual-test failed (exit {code}): {stderr.strip()}")
    result = json.loads(stdout)
    result["exit_code"] = code
    return result


def _require(result):
    if result["exit_code"] != EXIT_PASS:
        raise VisualCheckError(result)
    return result


def _describe(result):
    lines = [f"visual check {result['name']!r} {result['status']}: {result['image']}"]
    diff = result.get("diff")
    if diff and diff["status"] != "pass":
        lines.append(f"  [diff] {diff['status']}: {diff['reason']}")
        if diff.get("image"):
            lines.append(f"         diff image: {diff['image']}")
    for check in result["checks"]:
        if check["status"] == "pass":
            continue
        reason = f" ({check['verdict']['reason']})" if check.get("verdict") else ""
        lines.append(f"  [{check['index']}] {check['status']}: {check['text']}{reason}")
    if result["exit_code"] == EXIT_PENDING:
        lines.append(
            "Look at the image, then record decisions with `cargo visual-test verdict` "
            "(or approve a baseline with `cargo visual-test approve`), "
            "or set GPUI_VISUAL_JUDGE to a model judge."
        )
    return "\n".join(lines)
