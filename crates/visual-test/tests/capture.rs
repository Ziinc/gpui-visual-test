//! Renders a real GPUI view headlessly and runs it through a check.

use gpui::{
    AppContext as _, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px, rgb,
};
use gpui_visual_test::{Status, Store, Verdict, capture, judge::AgentJudge};

struct Panel;

impl Render for Panel {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0xffffff))
            .p(px(16.))
            .child(div().w(px(120.)).h(px(40.)).bg(rgb(0xd03030)))
    }
}

#[test]
fn captures_a_view_and_reuses_agent_verdicts() {
    let mut cx = capture::headless_app(None).expect("headless renderer");
    let shot = capture::render_view(&mut cx, "panel", 320., 200., |_, cx| cx.new(|_| Panel))
        .expect("capture");

    let (width, height) = shot.logical_size();
    assert_eq!((width.round(), height.round()), (320., 200.));
    let s = shot.scale();
    let red = shot.image().get_pixel((40. * s) as u32, (30. * s) as u32);
    let white = shot.image().get_pixel((200. * s) as u32, (150. * s) as u32);
    assert!(red[0] > 180 && red[1] < 80, "red box rendered: {red:?}");
    assert!(
        white.0[..3].iter().all(|c| *c > 240),
        "white background: {white:?}"
    );

    let dir = tempfile::tempdir().unwrap();
    let expectation = "A red rectangle sits in the top-left corner";
    let pending = shot
        .clone()
        .expect([expectation])
        .with_dir(dir.path())
        .run(&AgentJudge)
        .unwrap();
    assert_eq!(pending.manifest.status(), Status::Unsure);

    Store::new(dir.path())
        .record(
            "panel",
            Some(0),
            Verdict::new(Status::Pass, "red box at top left", "agent"),
        )
        .unwrap();
    let rerun =
        capture::render_view(&mut cx, "panel", 320., 200., |_, cx| cx.new(|_| Panel)).unwrap();
    let outcome = rerun
        .expect([expectation])
        .with_dir(dir.path())
        .assert(&AgentJudge);
    assert_eq!(outcome.judged, 0, "an identical render reuses the verdict");
}
