"""Runs the helpers against the real CLI with fake pages.

Build the CLI first (`cargo build -p cargo-visual-test`) or set
GPUI_VISUAL_TEST_CLI. Run with `python3 -m unittest`.
"""

import asyncio
import os
import struct
import tempfile
import unittest
import zlib

import gpui_visual_test_playwright as gvt

CLI = os.environ.get("GPUI_VISUAL_TEST_CLI") or os.path.abspath(
    os.path.join(os.path.dirname(__file__), "../../../target/debug/cargo-visual-test")
)


def png(width, height, square=None):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    rows = b""
    for y in range(height):
        rows += b"\0"
        for x in range(width):
            dark = square and square[0] <= x < square[0] + square[2] and square[1] <= y < square[1] + square[2]
            rows += bytes([20, 20, 20, 255] if dark else [255, 255, 255, 255])
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows))
        + chunk(b"IEND", b"")
    )


class Page:
    """A sync-API page at device scale 2."""

    def __init__(self, image):
        self.image = image
        self.viewport_size = {"width": struct.unpack(">I", image[16:20])[0] / 2, "height": 1}

    def screenshot(self, **_):
        return self.image


class AsyncPage(Page):
    async def screenshot(self, **_):
        return self.image


class Locator:
    def __init__(self, box):
        self.box = box

    def bounding_box(self):
        return self.box


def options():
    root = tempfile.mkdtemp(prefix="gvt-py-")
    return {"dir": os.path.join(root, "out"), "baseline_dir": os.path.join(root, "base"), "cli": CLI}


class Helpers(unittest.TestCase):
    def test_pending_expectations_with_locator_regions(self):
        result = gvt.check_visual(
            Page(png(80, 40)),
            "todo/empty",
            expect=["The list is empty"],
            expect_in={"input": "The input is blank"},
            regions={"input": Locator({"x": 10, "y": 5, "width": 20, "height": 10})},
            **options(),
        )
        self.assertEqual(result["exit_code"], 2)
        self.assertEqual(result["scale"], 2)
        self.assertEqual(result["regions"], [{"id": "input", "x": 10, "y": 5, "width": 20, "height": 10}])
        self.assertEqual(result["checks"][1]["region"], "input")
        with self.assertRaisesRegex(gvt.VisualCheckError, "cargo visual-test verdict"):
            gvt.expect_visual(Page(png(80, 40)), "todo/empty", expect=["x"], **options())

    def test_pixel_diff_round_trip(self):
        opts = {**options(), "diff": 0}
        self.assertEqual(gvt.check_visual(Page(png(40, 40)), "box", **opts)["diff"]["status"], "unsure")
        gvt.check_visual(Page(png(40, 40)), "box", update_baseline=True, **opts)
        self.assertEqual(gvt.expect_visual(Page(png(40, 40)), "box", **opts)["status"], "pass")
        with self.assertRaisesRegex(gvt.VisualCheckError, r"36 of 1600 pixels changed"):
            gvt.expect_visual(Page(png(40, 40, (4, 4, 6))), "box", **opts)

    def test_async_api(self):
        result = asyncio.run(
            gvt.check_visual_async(AsyncPage(png(20, 20)), "async", expect=["x"], **options())
        )
        self.assertEqual(result["exit_code"], 2)

    def test_hidden_region_and_missing_cli(self):
        with self.assertRaisesRegex(ValueError, "region 'gone' is not visible"):
            gvt.check_visual(Page(png(4, 4)), "x", expect=["x"], regions={"gone": Locator(None)}, **options())
        with self.assertRaisesRegex(FileNotFoundError, "GPUI_VISUAL_TEST_CLI"):
            gvt.check_visual(Page(png(4, 4)), "x", expect=["x"], **{**options(), "cli": "/nonexistent/cvt"})


if __name__ == "__main__":
    unittest.main()
