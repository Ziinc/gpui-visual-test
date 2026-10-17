//! Headless capture of GPUI windows into [`Shot`]s.
//!
//! Uses GPUI's offscreen renderer: Metal on macOS, Vulkan on Linux. Linux CI
//! without a GPU needs lavapipe (`mesa-vulkan-drivers`).

use crate::Shot;
use anyhow::{Context as _, Result, bail};
use gpui::{
    AnyWindowHandle, App, AssetSource, Bounds, Entity, HeadlessAppContext, Render, Window,
    WindowBounds, WindowOptions, px, size,
};
use std::sync::Arc;

/// A headless app that can render and capture windows. Run your own setup
/// (`gpui_kit::init`, themes) with `cx.update` before opening windows.
pub fn headless_app(assets: Option<Arc<dyn AssetSource>>) -> Result<HeadlessAppContext> {
    if gpui_platform::current_headless_renderer()?.is_none() {
        bail!(
            "GPUI has no headless renderer on this platform; capture on macOS or Linux (with Vulkan, e.g. lavapipe)"
        );
    }
    let text_system = gpui_platform::current_platform(true).text_system();
    Ok(HeadlessAppContext::with_platform(
        text_system,
        assets.unwrap_or_else(|| Arc::new(())),
        gpui_platform::current_headless_renderer,
    ))
}

/// Options for a hidden window of `width` x `height` logical pixels.
pub fn window_options(width: f32, height: f32) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(width), px(height)),
        })),
        focus: false,
        show: false,
        ..Default::default()
    }
}

/// Settles pending work, draws so layout that depends on a previous frame
/// (measured text, scroll) is final, and captures the window.
pub fn capture_window(
    cx: &mut HeadlessAppContext,
    handle: AnyWindowHandle,
    name: &str,
) -> Result<Shot> {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })?;
    }
    let scale = cx.update_window(handle, |_, window, _| window.scale_factor())?;
    let image = cx
        .capture_screenshot(handle)
        .with_context(|| format!("capturing `{name}`"))?;
    Ok(Shot::new(name, image, scale))
}

/// The last frame `window` drew, as a shot, for checking a running app
/// from inside it (a test hook, a debug command) without a screen grab.
/// Needs a platform that reads windows back
/// (`PlatformWindow::render_to_image`), such as the headless renderer.
pub fn window_shot(window: &Window, name: &str) -> Result<Shot> {
    let image = window.render_to_image().with_context(|| {
        format!("reading back `{name}`; this platform may not support render_to_image")
    })?;
    Ok(Shot::new(name, image, window.scale_factor()))
}

/// Opens `build`'s view in a hidden window, captures it, and closes the
/// window.
pub fn render_view<V: Render + 'static>(
    cx: &mut HeadlessAppContext,
    name: &str,
    width: f32,
    height: f32,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<Shot> {
    let handle: AnyWindowHandle = cx
        .update(|cx| cx.open_window(window_options(width, height), build))?
        .into();
    let shot = capture_window(cx, handle, name);
    cx.update_window(handle, |_, window, _| window.remove_window())?;
    cx.run_until_parked();
    shot
}
