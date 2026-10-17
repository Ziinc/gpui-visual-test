/** A Playwright Page or Locator, or anything with the same screenshot API. */
export interface Target {
  screenshot(options?: object): Promise<Buffer>;
  boundingBox?(): Promise<Rect | null>;
  viewportSize?(): { width: number; height: number } | null;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface VisualOptions {
  /** Expectations that must hold. */
  expect?: string[];
  /** Expectations reported but never failing. */
  should?: string[];
  /** Expectations about one region, keyed by region id. */
  expectIn?: Record<string, string | string[]>;
  /** Named areas: a Locator (its bounding box) or a rect in CSS pixels. */
  regions?: Record<string, { boundingBox(): Promise<Rect | null> } | Rect>;
  /** Pixel-diff the shot against its approved baseline, allowing this share (0 to 1) to change. */
  diff?: number;
  baselineDir?: string;
  updateBaseline?: boolean;
  /** Judge spec, such as `openai:<model>`. Default: $GPUI_VISUAL_JUDGE or `agent`. */
  judge?: string;
  rejudge?: boolean;
  /** Output directory. Default: $GPUI_VISUAL_DIR or target/visual-test. */
  dir?: string;
  /** The CLI to run. Default: $GPUI_VISUAL_TEST_CLI or `cargo-visual-test`. */
  cli?: string;
  /** Passed to `screenshot()`, such as `{ mask: [...] }`. */
  screenshot?: object;
}

export interface Check {
  index: number;
  text: string;
  region: string | null;
  severity: "must" | "should";
  status: "pass" | "fail" | "unsure";
  verdict: { status: string; reason: string; judge: string; confidence?: number } | null;
}

export interface VisualResult {
  name: string;
  status: "pass" | "fail" | "unsure";
  image: string;
  manifest: string;
  hash: string;
  width: number;
  height: number;
  scale: number;
  regions: Array<{ id: string } & Rect>;
  diff: null | {
    status: "pass" | "fail" | "unsure";
    reason: string;
    changed: number;
    max: number;
    threshold: number;
    bounds: Rect | null;
    baseline: string;
    image: string | null;
  };
  checks: Check[];
  /** 0 pass, 1 fail, 2 needs a decision. */
  exitCode: number;
}

export function checkVisual(target: Target, name: string, options?: VisualOptions): Promise<VisualResult>;
export function expectVisual(target: Target, name: string, options?: VisualOptions): Promise<VisualResult>;
export const EXIT: { pass: 0; fail: 1; pending: 2; error: 3 };
