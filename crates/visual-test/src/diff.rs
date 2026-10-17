//! Pixel diffs: how much of a shot changed since an approved baseline.
//!
//! A cheap, deterministic layer under the judged expectations. Baselines are
//! approved PNGs kept outside the output directory (committed with the
//! tests), so a run compares what it renders now against what someone
//! accepted before.

use crate::Status;
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Channel difference (0 to 255) a pixel may have before it counts as
/// changed. Absorbs anti-aliasing and rounding noise between renderers.
pub const DEFAULT_THRESHOLD: u8 = 8;

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The pixel difference between a before and an after image.
#[derive(Clone, Debug)]
pub struct Diff {
    /// Pixels that differ, counting every pixel outside the overlap when the
    /// sizes differ.
    pub changed: u64,
    /// Pixels compared: the area covering both images.
    pub total: u64,
    /// The smallest rectangle holding every changed pixel.
    pub bounds: Option<Bounds>,
    /// The images have different sizes.
    pub resized: bool,
    /// The after image faded, with changed pixels in magenta.
    pub image: RgbaImage,
}

impl Diff {
    /// Share of pixels (0 to 1) that changed.
    pub fn ratio(&self) -> f64 {
        self.changed as f64 / (self.total as f64).max(1.)
    }

    pub fn is_identical(&self) -> bool {
        self.changed == 0
    }
}

const MARK: Rgba<u8> = Rgba([255, 0, 170, 255]);

/// Compares two images pixel by pixel. A pixel changed when any channel
/// differs by more than `threshold`.
pub fn diff(before: &RgbaImage, after: &RgbaImage, threshold: u8) -> Diff {
    let (width, height) = (
        before.width().max(after.width()),
        before.height().max(after.height()),
    );
    let mut image = RgbaImage::new(width, height);
    let mut changed = 0;
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, out) in image.enumerate_pixels_mut() {
        let pixels = (
            before.get_pixel_checked(x, y),
            after.get_pixel_checked(x, y),
        );
        let differs = match pixels {
            (Some(a), Some(b)) => differs(a, b, threshold),
            _ => true,
        };
        *out = if differs {
            changed += 1;
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            MARK
        } else {
            fade(pixels.1.or(pixels.0).copied().unwrap_or(Rgba([0; 4])))
        };
    }
    Diff {
        changed,
        total: width as u64 * height as u64,
        bounds: (changed > 0).then(|| Bounds {
            x: x0,
            y: y0,
            width: x1 - x0 + 1,
            height: y1 - y0 + 1,
        }),
        resized: before.dimensions() != after.dimensions(),
        image,
    }
}

pub(crate) fn differs(a: &Rgba<u8>, b: &Rgba<u8>, threshold: u8) -> bool {
    a.0.iter()
        .zip(b.0.iter())
        .any(|(a, b)| a.abs_diff(*b) > threshold)
}

/// Grey, light and opaque, so magenta marks stand out on any content.
fn fade(Rgba([r, g, b, a]): Rgba<u8>) -> Rgba<u8> {
    let luma = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;
    let luma = luma * a as u32 / 255 + (255 - a as u32);
    let v = (180 + luma * 75 / 255) as u8;
    Rgba([v, v, v, 255])
}

/// The pixel-diff layer of a manifest: the shot compared with its approved
/// baseline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaselineDiff {
    /// The approved image, `<baseline dir>/<name>.png`.
    pub baseline: PathBuf,
    /// `<name>.diff.png` beside the manifest, when any pixel changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Share of pixels that changed (0 to 1).
    pub changed: f64,
    /// Share allowed to change before the layer fails.
    pub max: f64,
    pub threshold: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
    pub status: Status,
    pub reason: String,
}

/// `$GPUI_VISUAL_BASELINE_DIR`, else `visual-baselines/` at the workspace
/// root. Unlike the output directory, it belongs in version control.
pub fn default_baseline_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GPUI_VISUAL_BASELINE_DIR") {
        return dir.into();
    }
    crate::store::workspace_root().join("visual-baselines")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_changed_pixels_and_bounds() {
        let before = RgbaImage::from_pixel(10, 10, Rgba([255, 255, 255, 255]));
        let mut after = before.clone();
        after.put_pixel(2, 3, Rgba([0, 0, 0, 255]));
        after.put_pixel(5, 7, Rgba([0, 0, 0, 255]));
        after.put_pixel(9, 9, Rgba([250, 250, 250, 255]));
        let d = diff(&before, &after, DEFAULT_THRESHOLD);
        assert_eq!((d.changed, d.total), (2, 100));
        assert_eq!(
            d.bounds,
            Some(Bounds {
                x: 2,
                y: 3,
                width: 4,
                height: 5
            })
        );
        assert_eq!(*d.image.get_pixel(2, 3), MARK);
        assert_ne!(*d.image.get_pixel(0, 0), MARK);
        assert!(!d.resized);
        assert!(diff(&before, &before, 0).is_identical());
    }

    #[test]
    fn size_changes_count_the_uncovered_area() {
        let before = RgbaImage::new(4, 4);
        let after = RgbaImage::new(4, 6);
        let d = diff(&before, &after, DEFAULT_THRESHOLD);
        assert!(d.resized);
        assert_eq!((d.changed, d.total), (8, 24));
        assert_eq!(d.image.dimensions(), (4, 6));
    }
}
