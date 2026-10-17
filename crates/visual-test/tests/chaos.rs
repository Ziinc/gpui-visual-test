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

// Found by the second chaos pass and audit.

#[test]
fn huge_clip_origin_is_an_error_not_a_panic() {
    let shot = Shot::new("s", RgbaImage::new(10, 10), 1.);
    assert!(shot.clip(1e10, 0., 5., 5.).is_err());
}

#[test]
fn a_moved_region_is_judged_again() {
    use gpui_visual_test::Expectation;
    let dir = tempfile::tempdir().unwrap();
    let judge = pass_judge();
    let run = |id: &str, x: f32| {
        Shot::new("s", RgbaImage::new(10, 10), 1.)
            .with_region(id, x, 0., 5., 5.)
            .expect([Expectation::new("x").in_region(id)])
            .with_dir(dir.path())
            .run(&judge)
            .unwrap()
            .judged
    };
    assert_eq!(run("r", 0.), 1);
    assert_eq!(run("r", 0.), 0);
    assert_eq!(
        run("q", 0.),
        1,
        "a different region is a different question"
    );
    assert_eq!(run("q", 5.), 1, "a moved region is a different area");
}

#[test]
fn unknown_and_duplicate_regions_are_rejected() {
    use gpui_visual_test::Expectation;
    let dir = tempfile::tempdir().unwrap();
    let judge = pass_judge();
    let unknown = Shot::new("s", RgbaImage::new(10, 10), 1.)
        .with_region("toolbar", 0., 0., 5., 5.)
        .expect([Expectation::new("x").in_region("tolbar")])
        .with_dir(dir.path())
        .run(&judge);
    assert!(unknown.unwrap_err().to_string().contains("toolbar"));
    let duplicate = Shot::new("s", RgbaImage::new(10, 10), 1.)
        .with_region("a", 0., 0., 5., 5.)
        .with_region("a", 5., 5., 5., 5.)
        .expect(["x"])
        .with_dir(dir.path())
        .run(&judge);
    assert!(duplicate.is_err());
}

#[test]
fn tolerance_never_spans_a_size_change() {
    let dir = tempfile::tempdir().unwrap();
    let judge = pass_judge();
    let run = |w: u32, h: u32| {
        Shot::new("s", RgbaImage::new(w, h), 1.)
            .expect(["x"])
            .with_dir(dir.path())
            .with_tolerance(1.)
            .run(&judge)
            .unwrap()
    };
    run(10, 10);
    let resized = run(500, 20);
    assert_eq!(resized.judged, 1);
    let stored = image::open(dir.path().join("s.png")).unwrap();
    assert_eq!(
        (stored.width(), stored.height()),
        (resized.manifest.width, resized.manifest.height)
    );
}

#[test]
fn out_of_range_tolerance_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    for tolerance in [f64::NAN, -0.1, 1.5] {
        let run = Shot::new("s", RgbaImage::new(4, 4), 1.)
            .expect(["x"])
            .with_dir(dir.path())
            .with_tolerance(tolerance)
            .run(&pass_judge());
        assert!(run.is_err(), "{tolerance}");
    }
}

#[test]
fn a_corrupt_baseline_fails_the_layer_and_update_repairs_it() {
    let dir = tempfile::tempdir().unwrap();
    let baselines = tempfile::tempdir().unwrap();
    std::fs::write(
        baselines.path().join("s.png"),
        "version https://git-lfs.github.com/spec/v1\n",
    )
    .unwrap();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let judge = FnJudge::new("count", |_, e: &[gpui_visual_test::Expectation]| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(e.iter()
            .map(|_| Verdict::new(Status::Pass, "ok", "count"))
            .collect())
    });
    let run = |update: bool| {
        Shot::new("s", RgbaImage::new(4, 4), 1.)
            .expect(["x"])
            .with_dir(dir.path())
            .with_baseline_dir(baselines.path())
            .with_diff(0.)
            .update_baseline(update)
            .run(&judge)
            .unwrap()
    };
    let broken = run(false);
    assert_eq!(broken.diff_status(), Some(Status::Fail));
    assert!(
        broken
            .manifest
            .diff
            .unwrap()
            .reason
            .contains("could not be read")
    );
    assert!(run(true).is_ok());
    assert!(run(false).is_ok());
}

#[test]
fn approving_clears_the_diff_and_reports_where_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let baselines = tempfile::tempdir().unwrap();
    let run = |color: u8| {
        Shot::new(
            "ui/s",
            RgbaImage::from_pixel(4, 4, Rgba([color, 0, 0, 255])),
            1.,
        )
        .expect(Vec::<gpui_visual_test::Expectation>::new())
        .with_dir(dir.path())
        .with_baseline_dir(baselines.path())
        .with_diff(0.)
        .run(&pass_judge())
        .unwrap()
    };
    run(1);
    let store = gpui_visual_test::Store::new(dir.path());
    store.approve("ui/s", None).unwrap();
    assert!(run(200).manifest.diff.unwrap().image.is_some());
    let written = store.approve("ui/s", None).unwrap();
    assert_eq!(written, baselines.path().join("ui/s.png"));
    let diff = store.load("ui/s").unwrap().diff.unwrap();
    assert_eq!((diff.changed, diff.image, diff.bounds), (0., None, None));
    assert!(!dir.path().join("ui/s.diff.png").exists());

    // Dropping the diff layer removes its old image.
    run(1);
    Shot::new("ui/s", RgbaImage::new(4, 4), 1.)
        .expect(["x"])
        .with_dir(dir.path())
        .run(&pass_judge())
        .unwrap();
    assert!(!dir.path().join("ui/s.diff.png").exists());
}

#[test]
fn top_level_diff_links_are_relative() {
    let dir = tempfile::tempdir().unwrap();
    let baselines = tempfile::tempdir().unwrap();
    let run = |color: u8| {
        Shot::new(
            "x",
            RgbaImage::from_pixel(4, 4, Rgba([color, 0, 0, 255])),
            1.,
        )
        .expect(Vec::<gpui_visual_test::Expectation>::new())
        .with_dir(dir.path())
        .with_baseline_dir(baselines.path())
        .with_diff(0.)
        .update_baseline(color == 1)
        .run(&pass_judge())
        .unwrap()
    };
    run(1);
    run(200);
    let html = std::fs::read_to_string(dir.path().join("index.html")).unwrap();
    assert!(html.contains("href=\"x.diff.png\""), "{html}");
}

#[test]
fn printed_commands_quote_unusual_names() {
    assert_eq!(gpui_visual_test::shell_word("ui/panel"), "ui/panel");
    assert_eq!(gpui_visual_test::shell_word("my shot"), "'my shot'");
    assert_eq!(gpui_visual_test::shell_word("it's"), "'it'\\''s'");
    assert!(gpui_visual_test::Store::validate_name("-x").is_err());
}
