//! Visual tests for GPUI.
//!
//! Render a view headlessly (feature `capture`) or load any PNG, list what a
//! viewer should be able to confirm by looking at it, and let a judge decide:
//! a vision model, TypeSafe Jev in front of one, or an agent or person
//! reviewing through the `cargo visual-test` CLI.
//!
//! ```no_run
//! use gpui_visual_test::{Shot, judge};
//!
//! let shot = Shot::open("toolbar.png")?;
//! shot.expect([
//!     "The Save button is greyed out",
//!     "No toolbar label is truncated",
//! ])
//! .assert(&judge::AgentJudge);
//! # anyhow::Ok(())
//! ```
//!
//! Each check writes `<name>.png` and `<name>.json` to the output directory
//! (`target/visual-test` by default). A verdict stays valid while the image
//! is unchanged, so a model is only called, and an agent only asked, when a
//! shot changes.

#[cfg(feature = "capture")]
pub mod capture;
mod check;
mod expectation;
pub mod judge;
mod report;
mod shot;
mod store;
mod verdict;

pub use check::{Check, Outcome};
pub use expectation::{Expectation, Severity};
pub use judge::Judge;
pub use report::write_index;
pub use shot::{Region, Shot, difference};
pub use store::{Entry, MANIFEST_VERSION, Manifest, Store};
pub use verdict::{Status, Verdict};
