//! The output directory: one PNG and one JSON manifest per shot.
//!
//! The manifest is the contract between test runs, model judges, the CLI and
//! agents. A verdict recorded for an image stays valid while the image is
//! unchanged, so an agent can review a shot once and later runs reuse it.

use crate::{Expectation, Region, Status, Verdict};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const MANIFEST_VERSION: u32 = 1;

/// One expectation and its latest verdict, if any.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(flatten)]
    pub expectation: Expectation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
}

impl Entry {
    /// The verdict's status, with no verdict counting as `Unsure`.
    pub fn status(&self) -> Status {
        self.verdict.as_ref().map_or(Status::Unsure, |v| v.status)
    }
}

/// `<name>.json` next to `<name>.png`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub name: String,
    /// The image file, relative to the manifest.
    pub image: String,
    /// Pixel hash of the image the verdicts were given for.
    pub hash: String,
    /// Physical pixels.
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<Region>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<serde_json::Value>,
    pub checks: Vec<Entry>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Writes atomically, so parallel tests and the CLI never see half a file.
    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, serde_json::to_string_pretty(self)?.as_bytes())
    }

    /// Worst status over expectations that can fail the check.
    pub fn status(&self) -> Status {
        let statuses = self
            .checks
            .iter()
            .filter(|e| e.expectation.severity == crate::Severity::Must)
            .map(Entry::status);
        let mut worst = Status::Pass;
        for status in statuses {
            match status {
                Status::Fail => return Status::Fail,
                Status::Unsure => worst = Status::Unsure,
                Status::Pass => {}
            }
        }
        worst
    }
}

/// An output directory of shots.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `$GPUI_VISUAL_DIR`, else `visual-test/` under `$CARGO_TARGET_DIR` or
    /// the workspace's `target/`. Tests and the CLI resolve the same place
    /// from anywhere in the workspace.
    pub fn default_dir() -> PathBuf {
        if let Some(dir) = std::env::var_os("GPUI_VISUAL_DIR") {
            return dir.into();
        }
        if let Some(target) = std::env::var_os("CARGO_TARGET_DIR") {
            return PathBuf::from(target).join("visual-test");
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        let root = cwd
            .ancestors()
            .find(|dir| dir.join("Cargo.lock").exists())
            .unwrap_or(&cwd);
        root.join("target").join("visual-test")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Rejects names that would land outside the directory or collide:
    /// empty, absolute, `.`/`..` or empty segments, or backslashes.
    pub fn validate_name(name: &str) -> Result<()> {
        let bad = name.is_empty()
            || name.starts_with('/')
            || name.contains('\\')
            || name.contains(':')
            || name
                .split('/')
                .any(|seg| seg.is_empty() || seg == "." || seg == "..");
        if bad {
            anyhow::bail!(
                "shot name {name:?} must be a relative path of non-empty segments, without `.`, `..`, `\\` or `:`"
            );
        }
        Ok(())
    }

    pub fn manifest_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    pub fn image_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.png"))
    }

    pub fn load(&self, name: &str) -> Result<Manifest> {
        Self::validate_name(name)?;
        Manifest::load(&self.manifest_path(name))
    }

    /// Every manifest under the directory, by name.
    pub fn list(&self) -> Result<Vec<Manifest>> {
        let mut found = Vec::new();
        if self.dir.exists() {
            collect(&self.dir, &mut found)?;
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(found)
    }

    /// Records a verdict for one expectation (by index), or for all when
    /// `index` is `None`.
    pub fn record(&self, name: &str, index: Option<usize>, verdict: Verdict) -> Result<Manifest> {
        Self::validate_name(name)?;
        let path = self.manifest_path(name);
        let mut manifest = Manifest::load(&path)?;
        let count = manifest.checks.len();
        match index {
            Some(i) if i >= count => {
                anyhow::bail!("`{name}` has {count} expectations; index {i} is out of range")
            }
            Some(i) => manifest.checks[i].verdict = Some(verdict),
            None => {
                for entry in &mut manifest.checks {
                    entry.verdict = Some(verdict.clone());
                }
            }
        }
        manifest.save(&path)?;
        Ok(manifest)
    }
}

fn collect(dir: &Path, found: &mut Vec<Manifest>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        // `file_type` does not follow symlinks, so a link back up the tree
        // cannot loop.
        if entry.file_type()?.is_dir() {
            collect(&path, found)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            // A JSON file beside a PNG of the same stem is a manifest, and a
            // broken one is an error: skipping it would hide a shot from
            // `status`. Other JSON files may share the directory.
            match Manifest::load(&path) {
                Ok(manifest) => found.push(manifest),
                Err(error) if path.with_extension("png").exists() => return Err(error),
                Err(_) => {}
            }
        }
    }
    Ok(())
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    // Unique per process and per write, so parallel tests (threads in one
    // process) never share a temporary file.
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{file}.{}.{n}.tmp", std::process::id()));
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}
