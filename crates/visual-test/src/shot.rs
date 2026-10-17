//! A captured image plus what a judge may need to know about it.

use crate::{Check, Expectation};
use anyhow::{Context as _, Result, bail};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A named area of a shot, in logical pixels, that an expectation can refer
/// to by `id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A rendered image to be judged.
#[derive(Clone, Debug)]
pub struct Shot {
    name: String,
    image: RgbaImage,
    scale: f32,
    regions: Vec<Region>,
    state: Option<serde_json::Value>,
}

impl Shot {
    /// A shot from decoded pixels. `scale` is physical pixels per logical
    /// pixel (2.0 on a retina capture).
    pub fn new(name: impl Into<String>, image: RgbaImage, scale: f32) -> Self {
        Self {
            name: name.into(),
            image,
            scale,
            regions: Vec::new(),
            state: None,
        }
    }

    /// A shot from encoded PNG bytes, at scale 1.
    pub fn from_png(name: impl Into<String>, png: &[u8]) -> Result<Self> {
        let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .context("decoding PNG")?
            .to_rgba8();
        Ok(Self::new(name, image, 1.))
    }

    /// A shot from a PNG file. The name defaults to the file stem.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "shot".into());
        Self::from_png(name, &bytes)
    }

    /// Renames the shot. The name is its file name in the output directory,
    /// so `/` makes subdirectories.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Names an area, in logical pixels, for expectations to point at.
    pub fn with_region(
        mut self,
        id: impl Into<String>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Self {
        self.regions.push(Region {
            id: id.into(),
            x,
            y,
            width,
            height,
        });
        self
    }

    /// Attaches non-visual state (an element tree, view model) that text
    /// judges such as Jev read alongside the image description.
    pub fn with_state(mut self, state: serde_json::Value) -> Self {
        self.state = Some(state);
        self
    }

    /// Crops to an area given in logical pixels. Regions are moved into the
    /// cropped space and dropped when they fall outside it.
    pub fn clip(mut self, x: f32, y: f32, width: f32, height: f32) -> Result<Self> {
        if ![x, y, width, height].iter().all(|v| v.is_finite()) || x < 0. || y < 0. {
            bail!("clip {x},{y} {width}x{height} must be finite with a non-negative origin");
        }
        let s = self.scale;
        let (px, py) = ((x * s).round() as u32, (y * s).round() as u32);
        let (pw, ph) = ((width * s).round() as u32, (height * s).round() as u32);
        if pw == 0 || ph == 0 || px + pw > self.image.width() || py + ph > self.image.height() {
            bail!(
                "clip {x},{y} {width}x{height} is outside the {}x{} shot",
                self.image.width() as f32 / s,
                self.image.height() as f32 / s
            );
        }
        self.image = image::imageops::crop_imm(&self.image, px, py, pw, ph).to_image();
        self.regions = self
            .regions
            .into_iter()
            .filter(|r| {
                r.x >= x && r.y >= y && r.x + r.width <= x + width && r.y + r.height <= y + height
            })
            .map(|r| Region {
                x: r.x - x,
                y: r.y - y,
                ..r
            })
            .collect();
        Ok(self)
    }

    /// Starts a check of this shot against plain-English expectations.
    pub fn expect<E: Into<Expectation>>(self, expectations: impl IntoIterator<Item = E>) -> Check {
        Check::new(self, expectations.into_iter().map(Into::into).collect())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn image(&self) -> &RgbaImage {
        &self.image
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    pub fn state(&self) -> Option<&serde_json::Value> {
        self.state.as_ref()
    }

    /// Size in logical pixels.
    pub fn logical_size(&self) -> (f32, f32) {
        (
            self.image.width() as f32 / self.scale,
            self.image.height() as f32 / self.scale,
        )
    }

    /// The image encoded as PNG.
    pub fn png(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .context("encoding PNG")?;
        Ok(bytes)
    }

    /// Checks what serialization and judges rely on: a finite positive
    /// scale, a non-empty image, and finite, non-negative regions.
    pub fn validate(&self) -> Result<()> {
        if !(self.scale.is_finite() && self.scale > 0.) {
            bail!(
                "`{}` has scale {}; it must be finite and above 0",
                self.name,
                self.scale
            );
        }
        if self.image.width() == 0 || self.image.height() == 0 {
            bail!("`{}` is an empty image", self.name);
        }
        for r in &self.regions {
            let finite = [r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite());
            if !finite || r.width < 0. || r.height < 0. {
                bail!(
                    "`{}` region `{}` must have finite numbers and a non-negative size",
                    self.name,
                    r.id
                );
            }
        }
        Ok(())
    }

    /// A stable hash of the pixels, used to reuse verdicts for unchanged shots.
    pub fn hash(&self) -> String {
        let mut hash = fnv1a(0xcbf29ce484222325, &self.image.width().to_le_bytes());
        hash = fnv1a(hash, &self.image.height().to_le_bytes());
        format!("{:016x}", fnv1a(hash, self.image.as_raw()))
    }
}

fn fnv1a(seed: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(seed, |hash, byte| {
        (hash ^ *byte as u64).wrapping_mul(0x100000001b3)
    })
}

/// Share of pixels whose channels differ by more than a small anti-aliasing
/// allowance; 1.0 when the sizes differ.
pub fn difference(expected: &RgbaImage, actual: &RgbaImage) -> f64 {
    if expected.dimensions() != actual.dimensions() {
        return 1.;
    }
    let differing = expected
        .pixels()
        .zip(actual.pixels())
        .filter(|(a, b)| a.0.iter().zip(b.0.iter()).any(|(a, b)| a.abs_diff(*b) > 8))
        .count();
    differing as f64 / (expected.width() as f64 * expected.height() as f64).max(1.)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn clip_crops_pixels_and_moves_regions() {
        let shot = Shot::new(
            "s",
            RgbaImage::from_pixel(200, 100, Rgba([0, 0, 0, 255])),
            2.,
        )
        .with_region("inside", 20., 10., 10., 10.)
        .with_region("outside", 0., 0., 5., 5.)
        .clip(10., 5., 50., 20.)
        .unwrap();
        assert_eq!(shot.image().dimensions(), (100, 40));
        assert_eq!(shot.regions().len(), 1);
        assert_eq!((shot.regions()[0].x, shot.regions()[0].y), (10., 5.));
        assert!(
            Shot::new("s", RgbaImage::new(10, 10), 1.)
                .clip(5., 5., 10., 10.)
                .is_err()
        );
    }

    #[test]
    fn hash_follows_pixels_and_size() {
        let a = Shot::new("a", RgbaImage::new(4, 2), 1.);
        let b = Shot::new("b", RgbaImage::new(2, 4), 1.);
        let mut c = RgbaImage::new(4, 2);
        c.put_pixel(0, 0, Rgba([1, 0, 0, 0]));
        assert_eq!(a.hash(), Shot::new("x", RgbaImage::new(4, 2), 1.).hash());
        assert_ne!(a.hash(), b.hash());
        assert_ne!(a.hash(), Shot::new("c", c, 1.).hash());
    }

    #[test]
    fn png_round_trips() {
        let mut image = RgbaImage::new(3, 3);
        image.put_pixel(1, 1, Rgba([10, 20, 30, 255]));
        let shot = Shot::new("s", image, 1.);
        let back = Shot::from_png("s", &shot.png().unwrap()).unwrap();
        assert_eq!(back.hash(), shot.hash());
    }
}
