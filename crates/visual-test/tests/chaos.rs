//! Adversarial inputs and concurrency.

use gpui_visual_test::{Shot, Status, Verdict, judge::FnJudge};
use image::{Rgba, RgbaImage};

fn pass_judge() -> FnJudge<
    impl Fn(&Shot, &[gpui_visual_test::Expectation]) -> anyhow::Result<Vec<Verdict>> + Send + Sync,
> {
    FnJudge::new("fake", |_, e| {
        Ok(e.iter()
            .map(|_| Verdict::new(Status::Pass, "ok", "fake"))
            .collect())
    })
}

#[test]
fn parallel_checks_share_one_directory() {
    let dir = tempfile::tempdir().unwrap();
    let judge = pass_judge();
    std::thread::scope(|s| {
        for i in 0..32 {
            let dir = dir.path();
            let judge = &judge;
            s.spawn(move || {
                Shot::new(
                    format!("shot{i}"),
                    RgbaImage::from_pixel(64, 64, Rgba([i as u8, 0, 0, 255])),
                    1.,
                )
                .expect(["a"])
                .with_dir(dir)
                .run(judge)
                .unwrap();
            });
        }
    });
}

#[test]
fn negative_clip_is_rejected() {
    let shot = Shot::new("s", RgbaImage::new(100, 100), 1.);
    assert!(
        shot.clip(-10., 0., 20., 20.).is_err(),
        "negative origin must not silently shift"
    );
}

#[test]
fn non_finite_regions_do_not_poison_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let judge = pass_judge();
    let run = || {
        Shot::new("s", RgbaImage::new(10, 10), 1.)
            .with_region("r", f32::NAN, 0., 1., 1.)
            .expect(["a"])
            .with_dir(dir.path())
            .run(&judge)
    };
    assert!(
        run().is_err(),
        "a NaN region is rejected before anything is written"
    );
    assert!(!dir.path().join("s.json").exists());
}

#[test]
fn names_cannot_escape_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let judge = pass_judge();
    for name in ["../escape", "/tmp/escape-abs", "", "a/", "a//b", "./a"] {
        let result = Shot::new(name, RgbaImage::new(4, 4), 1.)
            .expect(["a"])
            .with_dir(&store)
            .run(&judge);
        assert!(result.is_err(), "name {name:?} should be rejected");
    }
}

#[test]
fn huge_and_tiny_images() {
    let dir = tempfile::tempdir().unwrap();
    let judge = pass_judge();
    let big = Shot::new(
        "big",
        RgbaImage::from_pixel(4000, 3000, Rgba([255, 255, 255, 255])),
        2.,
    );
    let start = std::time::Instant::now();
    big.expect(["a"]).with_dir(dir.path()).run(&judge).unwrap();
    eprintln!("4000x3000 check took {:?}", start.elapsed());
    Shot::new("one", RgbaImage::new(1, 1), 1.)
        .expect(["a"])
        .with_dir(dir.path())
        .run(&judge)
        .unwrap();
}

#[test]
fn identical_expectations_keep_their_own_verdicts() {
    let dir = tempfile::tempdir().unwrap();
    let shot = || {
        Shot::new("dup", RgbaImage::new(4, 4), 1.)
            .expect(["same", "same"])
            .with_dir(dir.path())
    };
    shot().run(&gpui_visual_test::judge::AgentJudge).unwrap();
    let store = gpui_visual_test::Store::new(dir.path());
    store
        .record("dup", Some(0), Verdict::new(Status::Pass, "first", "agent"))
        .unwrap();
    store
        .record(
            "dup",
            Some(1),
            Verdict::new(Status::Fail, "second", "agent"),
        )
        .unwrap();
    let outcome = shot().run(&gpui_visual_test::judge::AgentJudge).unwrap();
    let statuses: Vec<_> = outcome.manifest.checks.iter().map(|e| e.status()).collect();
    assert_eq!(statuses, [Status::Pass, Status::Fail]);
}

#[test]
fn a_broken_manifest_is_reported_not_hidden() {
    let dir = tempfile::tempdir().unwrap();
    Shot::new("ok", RgbaImage::new(4, 4), 1.)
        .expect(["a"])
        .with_dir(dir.path())
        .run(&pass_judge())
        .unwrap();
    std::fs::write(dir.path().join("ok.json"), "{\"version\": 1,").unwrap();
    std::fs::write(dir.path().join("unrelated.json"), "not a manifest").unwrap();
    assert!(gpui_visual_test::Store::new(dir.path()).list().is_err());
}

#[test]
fn bad_scales_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    for scale in [0., -1., f32::NAN, f32::INFINITY] {
        let result = Shot::new("s", RgbaImage::new(4, 4), scale)
            .expect(["a"])
            .with_dir(dir.path())
            .run(&pass_judge());
        assert!(result.is_err(), "scale {scale}");
    }
}
