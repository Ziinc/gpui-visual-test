//! `cargo visual-test`: review and assert visual checks from the command line.
//!
//! Built for agents as much as people: every listing has `--json`, exit codes
//! say pass (0), fail (1) or needs a decision (2), and `pending` prints the
//! image to look at and the command that records each verdict.

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use gpui_visual_test::{
    Expectation, Manifest, Shot, Status, Store, Verdict, diff::DEFAULT_THRESHOLD, image, judge,
    shell_word, write_index,
};
use serde_json::{Value, json};
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(name = "cargo visual-test", version, about)]
struct Cli {
    /// Output directory of shots [default: $GPUI_VISUAL_DIR or target/visual-test]
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List shots and their verdicts.
    List {
        /// Only shots whose overall status is this (pass, fail, unsure).
        #[arg(long)]
        status: Option<Status>,
        #[arg(long)]
        json: bool,
    },
    /// Expectations that need a decision, with the image to look at and the
    /// command that records each verdict. Exits 2 when any are pending.
    Pending {
        #[arg(long)]
        json: bool,
    },
    /// One shot in full.
    Show {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Record a verdict after looking at a shot's image.
    Verdict {
        name: String,
        /// Expectation index from `pending`/`show`, or `all`.
        index: String,
        status: Status,
        /// What in the image settles it, in a sentence.
        #[arg(long)]
        reason: String,
        /// Who decided [default: agent].
        #[arg(long, default_value = "agent")]
        judge: String,
        /// The image hash you looked at (`hash` in `pending --json`). Refuses
        /// to record if the image has changed since.
        #[arg(long)]
        hash: Option<String>,
    },
    /// Check any PNG against expectations, a baseline, or both. Exits 0
    /// pass, 1 fail, 2 pending.
    Check {
        image: PathBuf,
        /// An expectation that must hold (repeatable).
        #[arg(short, long = "expect")]
        expect: Vec<String>,
        /// An expectation reported but never failing (repeatable).
        #[arg(long)]
        should: Vec<String>,
        /// An expectation about one region: region=text (repeatable).
        #[arg(long, value_name = "REGION=TEXT")]
        expect_in: Vec<String>,
        /// A named area for expectations: id=x,y,width,height in logical px.
        #[arg(long)]
        region: Vec<String>,
        /// Name in the output directory [default: file stem].
        #[arg(long)]
        name: Option<String>,
        /// Physical pixels per logical pixel of the image.
        #[arg(long, default_value_t = 1.)]
        scale: f32,
        /// Judge spec [default: $GPUI_VISUAL_JUDGE or agent].
        #[arg(long)]
        judge: Option<String>,
        #[arg(long)]
        rejudge: bool,
        /// Also compare with the approved baseline, failing when more than
        /// this share (0 to 1) of pixels changed.
        #[arg(long, value_name = "MAX")]
        diff: Option<f64>,
        /// Approved baselines [default: $GPUI_VISUAL_BASELINE_DIR or visual-baselines]
        #[arg(long)]
        baseline_dir: Option<PathBuf>,
        /// Accept the image as the new baseline.
        #[arg(long)]
        update_baseline: bool,
        #[arg(long)]
        json: bool,
    },
    /// Pixel-diff two PNGs. Exits 0 when at most --max of pixels changed
    /// and the sizes match, else 1.
    Diff {
        before: PathBuf,
        after: PathBuf,
        /// Write the diff image here: the after image faded, changes in
        /// magenta.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Share of pixels (0 to 1) allowed to change.
        #[arg(long, default_value_t = 0.)]
        max: f64,
        /// Channel difference (0 to 255) before a pixel counts as changed.
        #[arg(long, default_value_t = DEFAULT_THRESHOLD)]
        threshold: u8,
        #[arg(long)]
        json: bool,
    },
    /// Accept shots' current images as their pixel-diff baselines.
    Approve {
        names: Vec<String>,
        /// Every shot whose pixel diff failed or has no baseline yet.
        #[arg(long, conflicts_with = "names")]
        all: bool,
        /// Approved baselines [default: where the last diff looked]
        #[arg(long)]
        baseline_dir: Option<PathBuf>,
    },
    /// Judge stored shots with a model (all, or the names given).
    Judge {
        names: Vec<String>,
        /// Judge spec [default: $GPUI_VISUAL_JUDGE or agent].
        #[arg(long)]
        judge: Option<String>,
        /// Judge again even where a verdict exists.
        #[arg(long)]
        rejudge: bool,
    },
    /// Overall result. Exits 0 when all pass, 1 on any failure, 2 when any
    /// expectation still needs a decision.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Rewrite index.html and print its path.
    Report,
}

fn main() -> ExitCode {
    // `cargo visual-test …` runs us as `cargo-visual-test visual-test …`.
    let args = std::env::args()
        .enumerate()
        .filter(|(i, a)| !(*i == 1 && a == "visual-test"))
        .map(|(_, a)| a);
    // Usage errors exit 3 like any other error: clap's own 2 would read as
    // "pending" to an agent.
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) if !error.use_stderr() => error.exit(),
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(3);
        }
    };
    match run(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(3)
        }
    }
}

fn exit_for(status: Status) -> ExitCode {
    match status {
        Status::Pass => ExitCode::SUCCESS,
        Status::Fail => ExitCode::from(1),
        Status::Unsure => ExitCode::from(2),
    }
}

/// [`exit_for`], except that undecided checks pass when the run allows them
/// (`GPUI_VISUAL_UNSURE=warn`).
fn exit_for_outcome(outcome: &gpui_visual_test::Outcome) -> ExitCode {
    if outcome.is_ok() {
        ExitCode::SUCCESS
    } else {
        exit_for(outcome.manifest.status())
    }
}

fn worst(statuses: impl IntoIterator<Item = Status>) -> Status {
    statuses
        .into_iter()
        .fold(Status::Pass, |worst, s| match (worst, s) {
            (Status::Fail, _) | (_, Status::Fail) => Status::Fail,
            (Status::Unsure, _) | (_, Status::Unsure) => Status::Unsure,
            _ => Status::Pass,
        })
}

fn run(cli: Cli) -> Result<ExitCode> {
    let store = Store::new(cli.dir.unwrap_or_else(Store::default_dir));
    match cli.command {
        Command::List { status, json } => {
            let manifests: Vec<Manifest> = store
                .list()?
                .into_iter()
                .filter(|m| status.is_none_or(|s| m.status() == s))
                .collect();
            if json {
                print_json(&Value::Array(
                    manifests.iter().map(|m| manifest_json(&store, m)).collect(),
                ))?;
            } else {
                if manifests.is_empty() {
                    println!("no shots in {}", store.dir().display());
                }
                for m in &manifests {
                    println!(
                        "{:6} {}  ({} expectations)",
                        m.status().to_string(),
                        m.name,
                        m.checks.len()
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Pending { json } => {
            let pending: Vec<Manifest> = store
                .list()?
                .into_iter()
                .filter(|m| {
                    m.checks.iter().any(|e| e.status() == Status::Unsure)
                        || m.diff.as_ref().is_some_and(|d| d.status == Status::Unsure)
                })
                .collect();
            if json {
                print_json(&Value::Array(
                    pending.iter().map(|m| manifest_json(&store, m)).collect(),
                ))?;
            } else if pending.is_empty() {
                println!("nothing pending in {}", store.dir().display());
            } else {
                for m in &pending {
                    println!(
                        "{}\n  image: {}",
                        m.name,
                        abs(store.image_path(&m.name)).display()
                    );
                    if m.diff.as_ref().is_some_and(|d| d.status == Status::Unsure) {
                        println!("  [diff] no baseline yet");
                        println!("      cargo visual-test approve {}", shell_word(&m.name));
                    }
                    for (i, e) in m
                        .checks
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| e.status() == Status::Unsure)
                    {
                        println!("  [{i}] {}", describe(&e.expectation));
                        println!(
                            "      cargo visual-test verdict {} {i} pass|fail --reason \"...\"",
                            shell_word(&m.name)
                        );
                    }
                }
            }
            Ok(if pending.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        Command::Show { name, json } => {
            let m = store.load(&name)?;
            if json {
                print_json(&manifest_json(&store, &m))?;
            } else {
                print_manifest(&store, &m);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Verdict {
            name,
            index,
            status,
            reason,
            judge,
            hash,
        } => {
            if let Some(hash) = hash {
                let current = store.load(&name)?.hash;
                if current != hash {
                    bail!(
                        "`{name}` changed since you looked (hash {current}, not {hash}); look at the image again"
                    );
                }
            }
            let index = match index.as_str() {
                "all" => None,
                n => Some(
                    n.parse::<usize>()
                        .with_context(|| format!("index `{n}` is not a number or `all`"))?,
                ),
            };
            let m = store.record(&name, index, Verdict::new(status, reason, judge))?;
            write_index(&store)?;
            print_manifest(&store, &m);
            Ok(ExitCode::SUCCESS)
        }
        Command::Check {
            image,
            expect,
            should,
            expect_in,
            region,
            name,
            scale,
            judge,
            rejudge,
            diff,
            baseline_dir,
            update_baseline,
            json,
        } => {
            if expect.is_empty() && should.is_empty() && expect_in.is_empty() && diff.is_none() {
                bail!(
                    "give at least one --expect (what a viewer should be able to confirm by looking at the image) or --diff"
                );
            }
            if diff.is_none() && (update_baseline || baseline_dir.is_some()) {
                bail!("--update-baseline and --baseline-dir need --diff");
            }
            let mut shot = Shot::open(&image)?;
            shot = Shot::new(
                name.unwrap_or_else(|| shot.name().to_string()),
                shot.image().clone(),
                scale,
            );
            for spec in &region {
                let (id, x, y, w, h) = parse_region(spec)?;
                shot = shot.with_region(id, x, y, w, h);
            }
            let mut expectations: Vec<Expectation> = expect
                .into_iter()
                .map(Expectation::new)
                .chain(should.into_iter().map(|t| Expectation::new(t).should()))
                .collect();
            for spec in &expect_in {
                let (region, text) = spec
                    .split_once('=')
                    .filter(|(region, text)| !region.is_empty() && !text.is_empty())
                    .with_context(|| {
                        format!("--expect-in `{spec}` should look like toolbar=Save is greyed out")
                    })?;
                expectations.push(Expectation::new(text).in_region(region));
            }
            let judge = pick_judge(judge)?;
            // Flags only switch things on, so GPUI_VISUAL_REJUDGE and
            // GPUI_VISUAL_UPDATE still work when they are not given.
            let mut check = shot.expect(expectations).with_dir(store.dir());
            if rejudge {
                check = check.rejudge(true);
            }
            if update_baseline {
                check = check.update_baseline(true);
            }
            if let Some(max) = diff {
                check = check.with_diff(max);
            }
            if let Some(dir) = baseline_dir {
                check = check.with_baseline_dir(dir);
            }
            let outcome = check.run(judge.as_ref())?;
            if json {
                print_json(&manifest_json(&store, &outcome.manifest))?;
            } else {
                print!("{outcome}");
            }
            Ok(exit_for_outcome(&outcome))
        }
        Command::Judge {
            names,
            judge,
            rejudge,
        } => {
            let judge = pick_judge(judge)?;
            let manifests: Vec<Manifest> = store
                .list()?
                .into_iter()
                .filter(|m| names.is_empty() || names.contains(&m.name))
                .collect();
            if let Some(missing) = names
                .iter()
                .find(|n| !manifests.iter().any(|m| &m.name == *n))
            {
                bail!("no shot `{missing}` in {}", store.dir().display());
            }
            if manifests.is_empty() {
                bail!("no shots in {}", store.dir().display());
            }
            let mut statuses = Vec::new();
            for m in manifests {
                let mut shot = Shot::open(store.image_path(&m.name))?;
                shot = Shot::new(m.name.clone(), shot.image().clone(), m.scale);
                for r in &m.regions {
                    shot = shot.with_region(r.id.clone(), r.x, r.y, r.width, r.height);
                }
                if let Some(state) = m.state.clone() {
                    shot = shot.with_state(state);
                }
                let mut check = shot
                    .expect(m.checks.iter().map(|e| e.expectation.clone()))
                    .with_dir(store.dir());
                if rejudge {
                    check = check.rejudge(true);
                }
                if let Some(diff) = &m.diff {
                    check = check
                        .with_diff(diff.max)
                        .with_diff_threshold(diff.threshold);
                    // `<dir>/<name>.png`, with one level per name segment.
                    if let Some(dir) = diff.baseline.ancestors().nth(m.name.split('/').count()) {
                        check = check.with_baseline_dir(dir);
                    }
                }
                let outcome = check.run(judge.as_ref())?;
                print!("{outcome}");
                statuses.push(if outcome.is_ok() {
                    Status::Pass
                } else {
                    outcome.manifest.status()
                });
            }
            Ok(exit_for(worst(statuses)))
        }
        Command::Status { json } => {
            let manifests = store.list()?;
            let count = |s: Status| manifests.iter().filter(|m| m.status() == s).count();
            let overall = worst(manifests.iter().map(Manifest::status));
            if json {
                print_json(&json!({
                    "status": overall,
                    "pass": count(Status::Pass),
                    "fail": count(Status::Fail),
                    "unsure": count(Status::Unsure),
                    "dir": abs(store.dir().to_path_buf()),
                }))?;
            } else {
                println!(
                    "{overall}: {} pass, {} fail, {} need a decision ({})",
                    count(Status::Pass),
                    count(Status::Fail),
                    count(Status::Unsure),
                    store.dir().display()
                );
            }
            Ok(exit_for(overall))
        }
        Command::Diff {
            before,
            after,
            out,
            max,
            threshold,
            json,
        } => {
            if !(0. ..=1.).contains(&max) {
                bail!("--max {max} must be between 0 and 1");
            }
            // By content, not extension, as `check` reads them.
            let open = |path: &PathBuf| -> Result<image::RgbaImage> {
                Ok(image::ImageReader::open(path)
                    .and_then(|r| r.with_guessed_format())
                    .with_context(|| format!("reading {}", path.display()))?
                    .decode()
                    .with_context(|| format!("decoding {}", path.display()))?
                    .to_rgba8())
            };
            let d = gpui_visual_test::diff(&open(&before)?, &open(&after)?, threshold);
            let pass = !d.resized && d.ratio() <= max;
            if let Some(out) = &out {
                if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                d.image
                    .save(out)
                    .with_context(|| format!("writing {}", out.display()))?;
            }
            if json {
                print_json(&json!({
                    "status": if pass { Status::Pass } else { Status::Fail },
                    "changed": d.ratio(),
                    "pixels": d.changed,
                    "total": d.total,
                    "max": max,
                    "threshold": threshold,
                    "resized": d.resized,
                    "bounds": d.bounds,
                    "image": out.map(abs),
                }))?;
            } else {
                let bounds = d.bounds.map_or(String::new(), |b| {
                    format!(" within {},{} {}x{} px", b.x, b.y, b.width, b.height)
                });
                println!(
                    "{}: {} of {} pixels changed ({:.3}%, max {:.3}%){bounds}{}",
                    if pass { "pass" } else { "fail" },
                    d.changed,
                    d.total,
                    d.ratio() * 100.,
                    max * 100.,
                    if d.resized { ", sizes differ" } else { "" }
                );
            }
            Ok(if pass {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Command::Approve {
            names,
            all,
            baseline_dir,
        } => {
            let names: Vec<String> = if all {
                store
                    .list()?
                    .into_iter()
                    .filter(|m| m.diff.as_ref().is_some_and(|d| d.status != Status::Pass))
                    .map(|m| m.name)
                    .collect()
            } else if names.is_empty() {
                bail!("name the shots to approve, or pass --all");
            } else {
                // Every name must exist before any baseline is written.
                for name in &names {
                    store.load(name)?;
                }
                names
            };
            for name in &names {
                let baseline = store.approve(name, baseline_dir.as_deref())?;
                println!("approved {name} -> {}", baseline.display());
            }
            if names.is_empty() {
                println!("nothing to approve in {}", store.dir().display());
            } else {
                write_index(&store)?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Report => {
            println!("{}", abs(write_index(&store)?).display());
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn pick_judge(spec: Option<String>) -> Result<Box<dyn gpui_visual_test::Judge>> {
    match spec {
        Some(spec) => judge::from_spec(&spec),
        None => judge::from_env(),
    }
}

fn parse_region(spec: &str) -> Result<(String, f32, f32, f32, f32)> {
    let (id, rect) = spec
        .split_once('=')
        .with_context(|| format!("region `{spec}` should look like toolbar=0,0,800,40"))?;
    let numbers: Vec<f32> = rect
        .split(',')
        .map(|n| n.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .with_context(|| format!("region `{spec}` should look like toolbar=0,0,800,40"))?;
    let [x, y, w, h] = numbers[..] else {
        bail!("region `{spec}` needs four numbers: x,y,width,height");
    };
    Ok((id.to_string(), x, y, w, h))
}

fn abs(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

fn describe(e: &Expectation) -> String {
    let mut text = e.text.clone();
    if let Some(region) = &e.region {
        text = format!("[in region {region}] {text}");
    }
    if e.severity == gpui_visual_test::Severity::Should {
        text.push_str(" (should)");
    }
    text
}

fn print_manifest(store: &Store, m: &Manifest) {
    println!(
        "{} ({})\n  image: {}",
        m.name,
        m.status(),
        abs(store.image_path(&m.name)).display()
    );
    if let Some(d) = &m.diff {
        println!("  [diff] {:6} {}", d.status.to_string(), d.reason);
        println!("         baseline: {}", d.baseline.display());
        if let Some(image) = &d.image {
            println!(
                "         diff: {}",
                abs(store.manifest_path(&m.name).with_file_name(image)).display()
            );
        }
    }
    for r in &m.regions {
        println!(
            "  region {}: x={} y={} width={} height={}",
            r.id, r.x, r.y, r.width, r.height
        );
    }
    for (i, e) in m.checks.iter().enumerate() {
        println!(
            "  [{i}] {:6} {}",
            e.status().to_string(),
            describe(&e.expectation)
        );
        if let Some(v) = &e.verdict {
            println!("         {} ({})", v.reason, v.judge);
        }
    }
}

/// The manifest plus absolute paths and indices, for agents.
fn manifest_json(store: &Store, m: &Manifest) -> Value {
    json!({
        "name": m.name,
        "status": m.status(),
        "image": abs(store.image_path(&m.name)),
        "manifest": abs(store.manifest_path(&m.name)),
        "hash": m.hash,
        "width": m.width,
        "height": m.height,
        "scale": m.scale,
        "regions": m.regions,
        "diff": m.diff.as_ref().map(|d| json!({
            "status": d.status,
            "reason": d.reason,
            "changed": d.changed,
            "max": d.max,
            "threshold": d.threshold,
            "bounds": d.bounds,
            "baseline": d.baseline,
            "image": d.image.as_ref().map(|i| abs(store.manifest_path(&m.name).with_file_name(i))),
        })),
        "checks": m.checks.iter().enumerate().map(|(i, e)| json!({
            "index": i,
            "text": e.expectation.text,
            "region": e.expectation.region,
            "severity": e.expectation.severity,
            "status": e.status(),
            "verdict": e.verdict,
        })).collect::<Vec<_>>(),
    })
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
