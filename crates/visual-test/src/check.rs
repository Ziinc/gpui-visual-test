//! Running a shot's expectations past a judge, reusing earlier verdicts for
//! unchanged images.

use crate::{
    BaselineDiff, Entry, Expectation, Judge, Manifest, Severity, Shot, Status, Store, Verdict,
    diff::{DEFAULT_THRESHOLD, default_baseline_dir, diff},
    shot::difference,
    store::{MANIFEST_VERSION, write_atomic},
};
use anyhow::{Context as _, Result, bail};
use std::{fmt, path::PathBuf};

/// A shot and its expectations, ready to judge. Build one with
/// [`Shot::expect`].
pub struct Check {
    shot: Shot,
    expectations: Vec<Expectation>,
    store: Store,
    tolerance: f64,
    rejudge: bool,
    allow_unsure: bool,
    max_diff: Option<f64>,
    baseline_dir: Option<PathBuf>,
    threshold: u8,
    update_baseline: bool,
}

impl Check {
    pub(crate) fn new(shot: Shot, expectations: Vec<Expectation>) -> Self {
        Self {
            shot,
            expectations,
            store: Store::new(Store::default_dir()),
            tolerance: 0.,
            rejudge: env_flag("GPUI_VISUAL_REJUDGE"),
            allow_unsure: std::env::var("GPUI_VISUAL_UNSURE").is_ok_and(|v| v == "warn"),
            max_diff: None,
            baseline_dir: None,
            threshold: DEFAULT_THRESHOLD,
            update_baseline: env_flag("GPUI_VISUAL_UPDATE"),
        }
    }

    /// Writes the shot and its manifest here instead of the default
    /// directory.
    pub fn with_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.store = Store::new(dir);
        self
    }

    /// Share of pixels (0 to 1) that may change while earlier verdicts are
    /// still reused. Defaults to 0: any pixel change re-judges.
    pub fn with_tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Ignores earlier verdicts and judges every expectation again. Also set
    /// by `GPUI_VISUAL_REJUDGE=1`.
    pub fn rejudge(mut self, rejudge: bool) -> Self {
        self.rejudge = rejudge;
        self
    }

    /// Lets `Unsure` verdicts (including pending agent reviews) pass with a
    /// warning. Also set by `GPUI_VISUAL_UNSURE=warn`.
    pub fn allow_unsure(mut self, allow: bool) -> Self {
        self.allow_unsure = allow;
        self
    }

    /// Adds a pixel-diff layer: compares the shot with its approved baseline
    /// and fails when more than `max` (0 to 1) of its pixels changed. With no
    /// baseline yet, the layer is pending until one is approved. With this
    /// layer, a check needs no expectations.
    pub fn with_diff(mut self, max: f64) -> Self {
        self.max_diff = Some(max);
        self
    }

    /// Where approved baselines live. Defaults to
    /// `$GPUI_VISUAL_BASELINE_DIR`, else `visual-baselines/` at the
    /// workspace root.
    pub fn with_baseline_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.baseline_dir = Some(dir.into());
        self
    }

    /// Channel difference (0 to 255) a pixel may have before it counts as
    /// changed. Defaults to [`DEFAULT_THRESHOLD`].
    pub fn with_diff_threshold(mut self, threshold: u8) -> Self {
        self.threshold = threshold;
        self
    }

    /// Accepts the shot as the new baseline instead of failing on a
    /// difference. Also set by `GPUI_VISUAL_UPDATE=1`.
    pub fn update_baseline(mut self, update: bool) -> Self {
        self.update_baseline = update;
        self
    }

    /// Judges the expectations, writes `<name>.png` and `<name>.json`, and
    /// returns the verdicts. Never panics on a failed expectation.
    pub fn run(self, judge: &dyn Judge) -> Result<Outcome> {
        if let Some(max) = self.max_diff
            && !(0. ..=1.).contains(&max)
        {
            bail!("pixel diff max {max} must be between 0 and 1");
        }
        if self.expectations.is_empty() && self.max_diff.is_none() {
            bail!(
                "`{}` has no expectations: list what a viewer should be able to confirm by looking at it",
                self.shot.name()
            );
        }
        Store::validate_name(self.shot.name())?;
        self.shot.validate()?;
        let name = self.shot.name().to_string();
        let manifest_path = self.store.manifest_path(&name);
        let image_path = self.store.image_path(&name);
        let hash = self.shot.hash();

        let previous = manifest_path
            .exists()
            .then(|| Manifest::load(&manifest_path))
            .transpose()?;
        let unchanged = previous.as_ref().is_some_and(|previous| {
            previous.hash == hash
                || (self.tolerance > 0.
                    && image::open(&image_path).is_ok_and(|old| {
                        difference(&old.to_rgba8(), self.shot.image()) <= self.tolerance
                    }))
        });

        // Match earlier verdicts by text and occurrence, so two identical
        // expectations keep their own verdicts.
        let mut verdicts: Vec<Option<Verdict>> = self
            .expectations
            .iter()
            .enumerate()
            .map(|(i, expectation)| {
                if self.rejudge || !unchanged {
                    return None;
                }
                let nth = self.expectations[..i]
                    .iter()
                    .filter(|e| e.text == expectation.text)
                    .count();
                previous
                    .as_ref()?
                    .checks
                    .iter()
                    .filter(|entry| entry.expectation.text == expectation.text)
                    .nth(nth)
                    .and_then(|entry| entry.verdict.clone())
                    .filter(|verdict| verdict.status != Status::Unsure)
            })
            .collect();

        let missing: Vec<usize> = (0..verdicts.len())
            .filter(|&i| verdicts[i].is_none())
            .collect();
        let judged = missing.len();
        if !missing.is_empty() {
            let asked: Vec<Expectation> = missing
                .iter()
                .map(|&i| self.expectations[i].clone())
                .collect();
            let answers = judge
                .judge(&self.shot, &asked)
                .with_context(|| format!("judging `{name}` with {}", judge.id()))?;
            if answers.len() != asked.len() {
                bail!(
                    "{} returned {} verdicts for {} expectations",
                    judge.id(),
                    answers.len(),
                    asked.len()
                );
            }
            for (i, verdict) in missing.into_iter().zip(answers) {
                verdicts[i] = Some(verdict);
            }
        }

        let diff = self
            .max_diff
            .map(|max| self.compare(&name, max))
            .transpose()?;

        // When every verdict was reused within tolerance, keep the judged
        // image so small changes cannot drift unnoticed. A pixel diff guards
        // against drift itself, so with one the new image is always kept.
        let keep_baseline =
            diff.is_none() && judged == 0 && previous.as_ref().is_some_and(|p| p.hash != hash);
        let (width, height) = self.shot.image().dimensions();
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            name: name.clone(),
            image: image_path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
            hash: if keep_baseline {
                previous.as_ref().map(|p| p.hash.clone()).unwrap_or(hash)
            } else {
                hash
            },
            width,
            height,
            scale: self.shot.scale(),
            regions: self.shot.regions().to_vec(),
            state: self.shot.state().cloned(),
            checks: self
                .expectations
                .iter()
                .cloned()
                .zip(verdicts)
                .map(|(expectation, verdict)| Entry {
                    expectation,
                    verdict,
                })
                .collect(),
            diff,
        };
        if !keep_baseline {
            write_atomic(&image_path, &self.shot.png()?)?;
        }
        manifest.save(&manifest_path)?;
        crate::report::write_index(&self.store)?;

        Ok(Outcome {
            manifest,
            image: image_path,
            manifest_path,
            judged,
            allow_unsure: self.allow_unsure,
        })
    }

    /// The pixel-diff layer against `<baseline dir>/<name>.png`. Writes
    /// `<name>.diff.png` when pixels changed, and the baseline when updating.
    fn compare(&self, name: &str, max: f64) -> Result<BaselineDiff> {
        let dir = self
            .baseline_dir
            .clone()
            .unwrap_or_else(default_baseline_dir);
        let baseline = std::path::absolute(dir.join(format!("{name}.png")))?;
        let diff_path = self.store.diff_path(name);
        let mut layer = BaselineDiff {
            baseline: baseline.clone(),
            image: None,
            changed: 0.,
            max,
            threshold: self.threshold,
            bounds: None,
            status: Status::Pass,
            reason: String::new(),
        };
        if !baseline.exists() {
            if self.update_baseline {
                write_atomic(&baseline, &self.shot.png()?)?;
                layer.reason = "baseline created".into();
            } else {
                layer.status = Status::Unsure;
                layer.reason = format!(
                    "no baseline yet; look at the image, then approve it with `cargo visual-test approve {name}`"
                );
            }
            remove_stale(&diff_path)?;
            return Ok(layer);
        }
        let before = image::open(&baseline)
            .with_context(|| format!("reading baseline {}", baseline.display()))?
            .to_rgba8();
        let d = diff(&before, self.shot.image(), self.threshold);
        layer.changed = d.ratio();
        layer.bounds = d.bounds;
        let percent = |r: f64| format!("{:.2}%", r * 100.);
        layer.reason = match d.bounds {
            None => "identical to the baseline".into(),
            Some(b) => {
                let resized = if d.resized {
                    format!(
                        ", size {}x{} -> {}x{}",
                        before.width(),
                        before.height(),
                        self.shot.image().width(),
                        self.shot.image().height()
                    )
                } else {
                    String::new()
                };
                format!(
                    "{} of pixels changed (max {}), within {},{} {}x{} px{resized}",
                    percent(layer.changed),
                    percent(max),
                    b.x,
                    b.y,
                    b.width,
                    b.height
                )
            }
        };
        if d.is_identical() {
            remove_stale(&diff_path)?;
        } else {
            let mut png = Vec::new();
            d.image
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .context("encoding diff PNG")?;
            write_atomic(&diff_path, &png)?;
            layer.image = diff_path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned());
            if self.update_baseline {
                write_atomic(&baseline, &self.shot.png()?)?;
                layer.reason = format!("baseline updated: {}", layer.reason);
            } else if d.resized || layer.changed > max {
                layer.status = Status::Fail;
            }
        }
        Ok(layer)
    }

    /// Runs and panics with a readable report unless every `Must`
    /// expectation passed.
    #[track_caller]
    pub fn assert(self, judge: &dyn Judge) -> Outcome {
        let outcome = match self.run(judge) {
            Ok(outcome) => outcome,
            Err(error) => panic!("visual check failed to run: {error:#}"),
        };
        if !outcome.is_ok() {
            panic!("{outcome}");
        }
        if outcome.has_warnings() {
            eprintln!("{outcome}");
        }
        outcome
    }

    /// [`Check::assert`] with the judge named by `GPUI_VISUAL_JUDGE`
    /// (default `agent`).
    #[track_caller]
    pub fn assert_env(self) -> Outcome {
        let judge = match crate::judge::from_env() {
            Ok(judge) => judge,
            Err(error) => panic!("GPUI_VISUAL_JUDGE: {error:#}"),
        };
        self.assert(judge.as_ref())
    }
}

/// The verdicts of one check.
#[derive(Debug)]
pub struct Outcome {
    pub manifest: Manifest,
    pub image: PathBuf,
    pub manifest_path: PathBuf,
    /// How many expectations the judge was asked about (the rest reused
    /// earlier verdicts).
    pub judged: usize,
    allow_unsure: bool,
}

impl Outcome {
    fn with_status(&self, severity: Severity, status: Status) -> impl Iterator<Item = &Entry> {
        self.manifest
            .checks
            .iter()
            .filter(move |e| e.expectation.severity == severity && e.status() == status)
    }

    /// `Must` expectations judged false.
    pub fn failures(&self) -> Vec<&Entry> {
        self.with_status(Severity::Must, Status::Fail).collect()
    }

    /// `Must` expectations with no decision yet.
    pub fn unsure(&self) -> Vec<&Entry> {
        self.with_status(Severity::Must, Status::Unsure).collect()
    }

    /// The pixel-diff layer's status, when the check has one.
    pub fn diff_status(&self) -> Option<Status> {
        self.manifest.diff.as_ref().map(|d| d.status)
    }

    pub fn is_ok(&self) -> bool {
        let diff = self.diff_status().unwrap_or(Status::Pass);
        self.failures().is_empty()
            && diff != Status::Fail
            && (self.allow_unsure || (self.unsure().is_empty() && diff != Status::Unsure))
    }

    fn has_warnings(&self) -> bool {
        self.diff_status().is_some_and(|s| s != Status::Pass)
            || self
                .manifest
                .checks
                .iter()
                .any(|e| e.status() != Status::Pass)
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = &self.manifest.name;
        writeln!(f, "visual check `{name}`: {}", self.image.display())?;
        if let Some(diff) = &self.manifest.diff {
            writeln!(
                f,
                "  [diff] {} {}\n        baseline: {}",
                mark(diff.status),
                diff.reason,
                diff.baseline.display()
            )?;
            if let Some(image) = &diff.image {
                let path = self.manifest_path.with_file_name(image);
                writeln!(f, "        diff: {}", path.display())?;
            }
            if diff.status == Status::Fail {
                writeln!(
                    f,
                    "        if the change is intended, accept it with\n          cargo visual-test approve {name}\n        or rerun with GPUI_VISUAL_UPDATE=1."
                )?;
            }
        }
        for (i, entry) in self.manifest.checks.iter().enumerate() {
            let mark = mark(entry.status());
            let should = if entry.expectation.severity == Severity::Should {
                " (should)"
            } else {
                ""
            };
            write!(f, "  [{i}] {mark}{should} {}", entry.expectation.text)?;
            if let Some(verdict) = &entry.verdict {
                write!(f, "\n        {} ({})", verdict.reason, verdict.judge)?;
            }
            writeln!(f)?;
        }
        let pending = self.unsure().len();
        if pending > 0 && !self.allow_unsure {
            writeln!(
                f,
                "{pending} expectation(s) need a decision. Look at the image, then record each with\n  \
                 cargo visual-test verdict {name} <index> pass|fail --reason \"...\"\n\
                 or set GPUI_VISUAL_JUDGE to a model judge (openai:<model>, anthropic:<model>, jev:<model>+<vision judge>)."
            )?;
        }
        Ok(())
    }
}

fn mark(status: Status) -> &'static str {
    match status {
        Status::Pass => "pass",
        Status::Fail => "FAIL",
        Status::Unsure => "????",
    }
}

fn remove_stale(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0" && v != "false")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judge::{AgentJudge, FnJudge};
    use image::{Rgba, RgbaImage};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn shot(color: u8) -> Shot {
        Shot::new(
            "panel/save",
            RgbaImage::from_pixel(8, 8, Rgba([color, 0, 0, 255])),
            1.,
        )
    }

    #[test]
    fn verdicts_are_reused_until_the_image_changes() {
        let dir = tempfile::tempdir().unwrap();
        let calls = AtomicUsize::new(0);
        let judge = FnJudge::new("fake", |_, expectations| {
            calls.fetch_add(expectations.len(), Ordering::SeqCst);
            Ok(expectations
                .iter()
                .map(|_| Verdict::new(Status::Pass, "looks right", "fake"))
                .collect())
        });

        let first = shot(1)
            .expect(["A", "B"])
            .with_dir(dir.path())
            .run(&judge)
            .unwrap();
        assert!(first.is_ok());
        assert_eq!(first.judged, 2);

        let again = shot(1)
            .expect(["A", "B", "C"])
            .with_dir(dir.path())
            .run(&judge)
            .unwrap();
        assert_eq!(again.judged, 1, "only the new expectation is judged");

        let changed = shot(200)
            .expect(["A"])
            .with_dir(dir.path())
            .run(&judge)
            .unwrap();
        assert_eq!(changed.judged, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert!(dir.path().join("panel/save.png").exists());
        assert!(dir.path().join("index.html").exists());
    }

    #[test]
    fn agent_verdicts_unblock_the_next_run() {
        let dir = tempfile::tempdir().unwrap();
        let pending = shot(1)
            .expect(["The panel is red"])
            .with_dir(dir.path())
            .run(&AgentJudge)
            .unwrap();
        assert!(!pending.is_ok());
        assert_eq!(pending.unsure().len(), 1);
        assert!(
            pending
                .to_string()
                .contains("cargo visual-test verdict panel/save <index>")
        );

        Store::new(dir.path())
            .record(
                "panel/save",
                Some(0),
                Verdict::new(Status::Pass, "solid red fill", "agent"),
            )
            .unwrap();
        let reviewed = shot(1)
            .expect(["The panel is red"])
            .with_dir(dir.path())
            .run(&AgentJudge)
            .unwrap();
        assert!(reviewed.is_ok());
        assert_eq!(reviewed.judged, 0);

        let failing = Store::new(dir.path())
            .record(
                "panel/save",
                None,
                Verdict::new(Status::Fail, "it is blue", "agent"),
            )
            .unwrap();
        assert_eq!(failing.status(), Status::Fail);
    }

    #[test]
    fn tolerance_keeps_the_judged_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let judge = FnJudge::new("fake", |_, e| {
            Ok(e.iter()
                .map(|_| Verdict::new(Status::Pass, "ok", "fake"))
                .collect())
        });
        let first = shot(1)
            .expect(["A"])
            .with_dir(dir.path())
            .run(&judge)
            .unwrap();
        let mut image = RgbaImage::from_pixel(8, 8, Rgba([1, 0, 0, 255]));
        image.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        let nudged = Shot::new("panel/save", image, 1.)
            .expect(["A"])
            .with_dir(dir.path())
            .with_tolerance(0.05)
            .run(&judge)
            .unwrap();
        assert_eq!(nudged.judged, 0);
        assert_eq!(nudged.manifest.hash, first.manifest.hash);
    }

    #[test]
    fn should_failures_do_not_fail() {
        let dir = tempfile::tempdir().unwrap();
        let judge = FnJudge::new("fake", |_, e| {
            Ok(e.iter()
                .map(|_| Verdict::new(Status::Fail, "no", "fake"))
                .collect())
        });
        let outcome = shot(1)
            .expect([Expectation::new("nice to have").should()])
            .with_dir(dir.path())
            .run(&judge)
            .unwrap();
        assert!(outcome.is_ok());
        assert!(
            shot(1)
                .expect(Vec::<Expectation>::new())
                .with_dir(dir.path())
                .run(&judge)
                .is_err()
        );
    }

    #[test]
    fn pixel_diff_against_an_approved_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let baselines = tempfile::tempdir().unwrap();
        let check = |shot: Shot, max: f64, update: bool| {
            shot.expect(Vec::<Expectation>::new())
                .with_dir(dir.path())
                .with_baseline_dir(baselines.path())
                .with_diff(max)
                .update_baseline(update)
                .run(&AgentJudge)
                .unwrap()
        };

        let new = check(shot(1), 0., false);
        assert_eq!(new.diff_status(), Some(Status::Unsure));
        assert!(!new.is_ok());

        let created = check(shot(1), 0., true);
        assert!(created.is_ok());
        assert!(baselines.path().join("panel/save.png").exists());

        let same = check(shot(1), 0., false);
        assert!(same.is_ok());
        assert!(same.manifest.diff.as_ref().unwrap().image.is_none());

        let mut image = RgbaImage::from_pixel(8, 8, Rgba([1, 0, 0, 255]));
        image.put_pixel(3, 4, Rgba([255, 255, 255, 255]));
        let changed = check(Shot::new("panel/save", image.clone(), 1.), 0., false);
        assert_eq!(changed.diff_status(), Some(Status::Fail));
        let layer = changed.manifest.diff.as_ref().unwrap();
        assert_eq!(layer.changed, 1. / 64.);
        assert!(dir.path().join("panel/save.diff.png").exists());
        assert!(
            changed
                .to_string()
                .contains("cargo visual-test approve panel/save")
        );

        let tolerated = check(Shot::new("panel/save", image, 1.), 0.05, false);
        assert!(tolerated.is_ok());

        assert!(check(shot(1), 0., false).is_ok());
        assert!(!dir.path().join("panel/save.diff.png").exists());

        let resized = Shot::new("panel/save", RgbaImage::new(8, 9), 1.);
        assert_eq!(check(resized, 1., false).diff_status(), Some(Status::Fail));
    }
}
