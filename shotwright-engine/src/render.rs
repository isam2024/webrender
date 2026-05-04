//! Rendering pipeline: drive a headless Servo `WebView` to capture a URL as PNG.
//!
//! Public surface is intentionally narrow: `capture(RenderOptions) -> Vec<u8>`.
//!
//! The shape of this code matches Servo's own test harness in
//! `servo/tests/common/mod.rs` and `servo/tests/webview.rs`, which is the
//! authoritative example for headless rendering against
//! `SoftwareRenderingContext`. Three details that aren't obvious from the
//! public docs but are required:
//!
//!   1. `rendering_context.make_current()` must be called after construction
//!      so the OpenGL context is bound to this thread. The screenshot
//!      pipeline's `read_to_image` reads pixels from this context.
//!   2. Spin the event loop (`servo.spin_event_loop()`) with a small sleep
//!      between iterations — `take_screenshot`'s callback is fired from
//!      inside the painter, which runs as part of `spin_event_loop`'s work.
//!   3. Wait for `LoadStatus::Complete` via the `WebViewDelegate` before
//!      dispatching `take_screenshot`. Calling it earlier means the
//!      screenshot-readiness handshake races with `pending_changes` clearing
//!      in the constellation, and the callback may never fire.
//!
//! Notably, headless rendering does NOT require explicit `webview.paint()` /
//! `rendering_context.present()` calls (those are for windowed embedders like
//! the winit example). The painter drives compositing internally.

use anyhow::{Context, Result, bail};
use dpi::PhysicalSize;
use servo::{
    JSValue, LoadStatus, RenderingContext, RgbaImage, ServoBuilder, SoftwareRenderingContext,
    WebView, WebViewBuilder, WebViewDelegate,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

/// Verbose diagnostic stderr output. Activated by `SHOTWRIGHT_DEBUG=1`.
fn debug_enabled() -> bool {
    std::env::var_os("SHOTWRIGHT_DEBUG").is_some()
}

macro_rules! dlog {
    ($($arg:tt)*) => {
        if debug_enabled() {
            eprintln!("[shotwright] {}", format_args!($($arg)*));
        }
    };
}

/// `WebViewDelegate` that surfaces lifecycle events to flags the main loop
/// can poll. Without an explicit delegate, every notification (load progress,
/// crashes, navigations) is silently dropped.
struct RenderDelegate {
    load_complete: Rc<Cell<bool>>,
}

impl WebViewDelegate for RenderDelegate {
    fn notify_load_status_changed(&self, _wv: WebView, status: LoadStatus) {
        dlog!("load_status: {status:?}");
        if matches!(status, LoadStatus::Complete) {
            self.load_complete.set(true);
        }
    }
    fn notify_url_changed(&self, _wv: WebView, url: Url) {
        dlog!("url_changed: {url}");
    }
    fn notify_page_title_changed(&self, _wv: WebView, t: Option<String>) {
        dlog!("page_title: {t:?}");
    }
    fn notify_new_frame_ready(&self, _wv: WebView) {
        dlog!("new_frame_ready");
    }
    fn notify_crashed(&self, _wv: WebView, reason: String, bt: Option<String>) {
        eprintln!("[shotwright] CRASHED: {reason}");
        if let Some(b) = bt {
            eprintln!("[shotwright] backtrace: {b}");
        }
    }
}

pub struct RenderOptions {
    pub url: Url,
    pub width: u32,
    pub height: u32,
    pub full_page: bool,
    pub timeout: Duration,
    pub dpr: f32,
    pub settle: Duration,
    pub user_agent: Option<String>,
}

/// Spin Servo's event loop until `done` returns true, the `deadline` passes,
/// or `timeout_msg` is bailed. Mirrors `ServoTest::spin` in Servo's own tests:
/// busy-loop with a 1 ms sleep so we don't pin a CPU but stay responsive.
fn spin_until<F>(
    servo: &servo::Servo,
    deadline: Instant,
    timeout_msg: &str,
    mut done: F,
) -> Result<()>
where
    F: FnMut() -> bool,
{
    while !done() {
        if Instant::now() >= deadline {
            bail!("{timeout_msg}");
        }
        servo.spin_event_loop();
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

pub fn capture(opts: RenderOptions) -> Result<Vec<u8>> {
    let device_w = ((opts.width as f32) * opts.dpr).round().max(1.0) as u32;
    let device_h = ((opts.height as f32) * opts.dpr).round().max(1.0) as u32;
    let initial_size = PhysicalSize::new(device_w, device_h);

    let rendering_context: Rc<SoftwareRenderingContext> = Rc::new(
        SoftwareRenderingContext::new(initial_size)
            .map_err(|e| anyhow::anyhow!("creating SoftwareRenderingContext: {e:?}"))?,
    );
    rendering_context
        .make_current()
        .map_err(|e| anyhow::anyhow!("rendering_context.make_current: {e:?}"))?;

    // user_agent goes through Preferences in 0.1.x — accepting the option
    // and applying it later. Servo's default UA is reasonable in the meantime.
    let _ = &opts.user_agent;

    let servo = ServoBuilder::default().build();
    if debug_enabled() {
        servo.setup_logging();
        dlog!("servo built; rendering_context size={initial_size:?}");
    }

    let load_complete = Rc::new(Cell::new(false));
    let delegate = Rc::new(RenderDelegate {
        load_complete: load_complete.clone(),
    });

    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .url(opts.url.clone())
        .delegate(delegate)
        .build();
    dlog!("webview built; url={}", opts.url);

    // Single deadline covers the whole capture (load + screenshot + optional
    // settle + optional full-page reshot). Predictable upper bound from the
    // user's perspective rather than per-phase budgets that compound.
    let deadline = Instant::now() + opts.timeout;

    spin_until(
        &servo,
        deadline,
        &format!(
            "page did not load within {}s (last status: {:?})",
            opts.timeout.as_secs(),
            webview.load_status()
        ),
        || load_complete.get(),
    )?;
    dlog!("page loaded; dispatching take_screenshot");

    let rgba = run_screenshot(&servo, &webview, deadline)?;

    if !opts.settle.is_zero() {
        let settle_until = (Instant::now() + opts.settle).min(deadline);
        let _ = spin_until(&servo, settle_until, "(unreachable)", || false);
    }

    let final_rgba = if opts.full_page {
        let scroll_h = run_eval_number(
            &servo,
            &webview,
            "document.documentElement.scrollHeight",
            deadline,
        )?
        .round() as u32;
        let target_h = scroll_h.clamp(device_h, 16384);
        if target_h != device_h {
            let target = PhysicalSize::new(device_w, target_h);
            rendering_context.resize(target);
            webview.resize(target);
            run_screenshot(&servo, &webview, deadline)?
        } else {
            rgba
        }
    } else {
        rgba
    };

    let (w, h) = final_rgba.dimensions();
    encode_png(final_rgba.as_raw(), w, h).context("encoding PNG")
}

/// Bridge `WebView::take_screenshot`'s callback onto the calling thread.
fn run_screenshot(
    servo: &servo::Servo,
    webview: &servo::WebView,
    deadline: Instant,
) -> Result<RgbaImage> {
    let slot: Rc<RefCell<Option<Result<RgbaImage, String>>>> = Rc::new(RefCell::new(None));
    {
        let slot = slot.clone();
        webview.take_screenshot(None, move |r| {
            *slot.borrow_mut() = Some(r.map_err(|e| format!("{e:?}")));
        });
    }
    dlog!("take_screenshot dispatched");

    let last_log = Cell::new(Instant::now());
    spin_until(
        &servo,
        deadline,
        &format!(
            "timeout waiting for screenshot (last load_status: {:?})",
            webview.load_status()
        ),
        || {
            if debug_enabled() && last_log.get().elapsed() >= Duration::from_secs(5) {
                eprintln!(
                    "[shotwright] still waiting; load_status={:?}",
                    webview.load_status()
                );
                last_log.set(Instant::now());
            }
            slot.borrow().is_some()
        },
    )?;
    let result = slot.borrow_mut().take().unwrap();
    result.map_err(|e| anyhow::anyhow!("screenshot capture failed: {e}"))
}

/// Evaluate JS on the page and coerce the result to a number (e.g. scrollHeight).
fn run_eval_number(
    servo: &servo::Servo,
    webview: &servo::WebView,
    script: &str,
    deadline: Instant,
) -> Result<f64> {
    let slot: Rc<RefCell<Option<Result<JSValue, String>>>> = Rc::new(RefCell::new(None));
    {
        let slot = slot.clone();
        webview.evaluate_javascript(script.to_string(), move |r| {
            *slot.borrow_mut() = Some(r.map_err(|e| format!("{e:?}")));
        });
    }
    spin_until(&servo, deadline, "timeout waiting for JS evaluation", || {
        slot.borrow().is_some()
    })?;
    // Bind before match so RefMut temporary drops before slot does.
    let result = slot.borrow_mut().take().unwrap();
    match result {
        Ok(JSValue::Number(n)) => Ok(n),
        Ok(other) => bail!("expected JS number, got {other:?}"),
        Err(e) => bail!("JS evaluation error: {e}"),
    }
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
    let mut out = Vec::with_capacity((width * height) as usize);
    PngEncoder::new(&mut out).write_image(rgba, width, height, ExtendedColorType::Rgba8)?;
    Ok(out)
}
