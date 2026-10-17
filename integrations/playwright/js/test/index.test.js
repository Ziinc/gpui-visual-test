// Runs the helpers against the real CLI with a fake page. Build the CLI
// first (`cargo build -p cargo-visual-test`) or set GPUI_VISUAL_TEST_CLI.

"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const zlib = require("node:zlib");
const { checkVisual, expectVisual } = require("..");

const cli =
  process.env.GPUI_VISUAL_TEST_CLI ??
  path.resolve(__dirname, "../../../../target/debug/cargo-visual-test");

/** A solid PNG with an optional dark square. */
function png(width, height, square) {
  const crc = (buf) => {
    let c = ~0;
    for (const byte of buf) {
      c ^= byte;
      for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
    }
    return ~c >>> 0;
  };
  const chunk = (type, data) => {
    const out = Buffer.alloc(12 + data.length);
    out.writeUInt32BE(data.length, 0);
    out.write(type, 4, "ascii");
    data.copy(out, 8);
    out.writeUInt32BE(crc(out.subarray(4, 8 + data.length)), 8 + data.length);
    return out;
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr.set([8, 6, 0, 0, 0], 8);
  const rows = [];
  for (let y = 0; y < height; y++) {
    const row = Buffer.alloc(1 + width * 4);
    for (let x = 0; x < width; x++) {
      const dark = square && x >= square.x && x < square.x + square.size && y >= square.y && y < square.y + square.size;
      row.set(dark ? [20, 20, 20, 255] : [255, 255, 255, 255], 1 + x * 4);
    }
    rows.push(row);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(Buffer.concat(rows))),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/** A page at device scale 2 whose screenshot is `image`. */
function page(image) {
  return {
    screenshot: async () => image,
    viewportSize: () => ({ width: image.readUInt32BE(16) / 2, height: image.readUInt32BE(20) / 2 }),
  };
}

function dirs() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "gvt-js-"));
  return { dir: path.join(root, "out"), baselineDir: path.join(root, "baselines"), cli };
}

test("expectations stay pending for an agent, with regions from locators", async () => {
  const opts = dirs();
  const locator = { boundingBox: async () => ({ x: 10, y: 5, width: 20, height: 10 }) };
  const result = await checkVisual(page(png(80, 40)), "todo/empty", {
    ...opts,
    expect: ["The list is empty"],
    expectIn: { input: "The input is blank" },
    regions: { input: locator },
  });
  assert.equal(result.exitCode, 2);
  assert.equal(result.scale, 2);
  assert.deepEqual(result.regions, [{ id: "input", x: 10, y: 5, width: 20, height: 10 }]);
  assert.equal(result.checks[1].region, "input");
  await assert.rejects(
    expectVisual(page(png(80, 40)), "todo/empty", { ...opts, expect: ["The list is empty"] }),
    /cargo visual-test verdict/,
  );
});

test("pixel diff: pending, approved, then failing on a change", async () => {
  const opts = { ...dirs(), diff: 0 };
  const first = await checkVisual(page(png(40, 40)), "box", opts);
  assert.equal(first.diff.status, "unsure");
  await checkVisual(page(png(40, 40)), "box", { ...opts, updateBaseline: true });
  assert.equal((await expectVisual(page(png(40, 40)), "box", opts)).status, "pass");
  await assert.rejects(
    expectVisual(page(png(40, 40, { x: 4, y: 4, size: 6 })), "box", opts),
    /\[diff\] fail: 36 of 1600 pixels changed/,
  );
});

test("a locator shot offsets regions into its own box", async () => {
  const target = {
    screenshot: async () => png(40, 20),
    boundingBox: async () => ({ x: 100, y: 50, width: 20, height: 10 }),
  };
  const result = await checkVisual(target, "card", {
    ...dirs(),
    expect: ["x"],
    regions: { icon: { x: 104, y: 52, width: 4, height: 4 } },
  });
  assert.equal(result.scale, 2);
  assert.deepEqual(result.regions[0], { id: "icon", x: 4, y: 2, width: 4, height: 4 });
});

test("a missing CLI is a clear error", async () => {
  await assert.rejects(
    checkVisual(page(png(4, 4)), "x", { ...dirs(), cli: "/nonexistent/cargo-visual-test", expect: ["x"] }),
    /could not run .*GPUI_VISUAL_TEST_CLI/s,
  );
});
