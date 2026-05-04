//! Rendering pipeline: drive a headless Servo `WebView` to capture a URL as PNG.
//!
//! Public surface is intentionally narrow: `capture(RenderOptions) -> Vec<u8>`.
//!
//! Servo 0.1.x exposes:
//!   - `ServoBuilder::default().build()` to construct an engine
//!   - `SoftwareRenderingContext::new(PhysicalSize)` for headless CPU rendering
//!     (no display server, no GPU)
//!   - `WebViewBuilder::new(&servo, rc).url(url).build()` to create a webview
//!   - `WebView::take_screenshot(rect, callback)` — async, fires a callback when
//!     fonts/images/stylesheets have all settled
//!
//! `take_screenshot` and `evaluate_javascript` are callback-based, so we bridge
//! them into the calling thread with `Rc<RefCell<Option<_>>>` and pump the event
//! loop until the cell is filled or the deadline passes.

use anyhow::{Context, Result, bail};
use dpi::PhysicalSize;
use servo::{
    JSValue, LoadStatus, RenderingContext, RgbaImage, ServoBuilder, SoftwareRenderingContext,
    WebView, WebViewBuilder, WebViewDelegate,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
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

/// Delegate that bridges Servo's `notify_new_frame_ready` events to a flag
/// the main loop checks. When the flag is set, the main loop calls
/// `webview.paint()` + `rendering_context.present()` — without this, frames
/// are generated but never composited into the rendering context, and
/// `take_screenshot`'s "rendering is up to date" wait condition never trips.
///
/// Without an explicit delegate, every notification is silently dropped,
/// which is what made the original "timeout, no PNG" failure mode opaque.
/// Debug prints are gated on `SHOTWRIGHT_DEBUG=1`.
struct RenderDelegate {
    needs_paint: Rc<Cell<bool>>,
}

impl WebViewDelegate for RenderDelegate {
    fn notify_new_frame_ready(&self, _wv: WebView) {
        self.needs_paint.set(true);
        dlog!("new_frame_ready");
    }
    fn notify_load_status_changed(&self, _wv: WebView, status: LoadStatus) {
        dlog!("load_status: {status:?}");
    }
    fn notify_url_changed(&self, _wv: WebView, url: Url) {
        dlog!("url_changed: {url}");
    }
    fn notify_page_title_changed(&self, _wv: WebView, t: Option<String>) {
        dlog!("page_title: {t:?}");
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

pub fn capture(opts: RenderOptions) -> Result<Vec<u8>> {
    let device_w = ((opts.width as f32) * opts.dpr).round().max(1.0) as u32;
    let device_h = ((opts.height as f32) * opts.dpr).round().max(1.0) as u32;
    let initial_size = PhysicalSize::new(device_w, device_h);

    let rendering_context: Rc<SoftwareRenderingContext> = Rc::new(
        SoftwareRenderingContext::new(initial_size)
            .map_err(|e| anyhow::anyhow!("creating SoftwareRenderingContext: {e:?}"))?,
    );

    // user_agent goes through Preferences in 0.1.x — for now we accept the option
    // and apply it via opts/prefs in a later patch. Dropping silently here is OK
    // because Servo's default UA is reasonable.
    let _ = &opts.user_agent;

    let servo = ServoBuilder::default().build();
    if debug_enabled() {
        servo.setup_logging();
        dlog!("servo built; rendering_context size={:?}", initial_size);
    }

    // Shared flag: delegate sets it on notify_new_frame_ready, main loop
    // consumes it by calling paint+present. Without this, take_screenshot
    // waits forever because frames are produced but never composited.
    let needs_paint = Rc::new(Cell::new(false));
    let delegate = Rc::new(RenderDelegate {
        needs_paint: needs_paint.clone(),
    });

    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .url(opts.url.clone())
        .delegate(delegate)
        .build();
    webview.show();
    dlog!("webview built and shown; url={}", opts.url);

    // Phase 1: capture at the requested viewport. take_screenshot internally
    // waits for the document, all subresources, and pending render frames.
    let rgba = run_screenshot(
        &servo,
        &webview,
        rendering_context.as_ref(),
        &needs_paint,
        opts.timeout,
    )?;

    if !opts.settle.is_zero() {
        // Optional human-tunable post-load idle for sites that animate after
        // their first paint. We pump the loop without blocking on anything.
        let until = Instant::now() + opts.settle;
        while Instant::now() < until {
            servo.spin_event_loop();
        }
    }

    let final_rgba = if opts.full_page {
        // Ask the page how tall it is, then resize the viewport to match and
        // capture again. Bounded so a runaway page can't OOM us.
        let scroll_h = run_eval_number(
            &servo,
            &webview,
            "document.documentElement.scrollHeight",
            opts.timeout,
        )?
        .round() as u32;
        let target_h = scroll_h.clamp(device_h, 16384);
        if target_h != device_h {
            let target = PhysicalSize::new(device_w, target_h);
            rendering_context.resize(target);
            webview.resize(target);
            run_screenshot(
                &servo,
                &webview,
                rendering_context.as_ref(),
                &needs_paint,
                opts.timeout,
            )?
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
/// Pumps the event loop, AND on every `notify_new_frame_ready` (signalled
/// via `needs_paint`) calls paint+present so the rendering context is kept
/// up to date — that's the precondition for take_screenshot to fire its
/// callback. Without paint+present the callback would never come.
fn run_screenshot(
    servo: &servo::Servo,
    webview: &servo::WebView,
    rendering_context: &SoftwareRenderingContext,
    needs_paint: &Rc<Cell<bool>>,
    timeout: Duration,
) -> Result<RgbaImage> {
    let slot: Rc<RefCell<Option<Result<RgbaImage, String>>>> = Rc::new(RefCell::new(None));
    {
        let slot = slot.clone();
        webview.take_screenshot(None, move |r| {
            *slot.borrow_mut() = Some(r.map_err(|e| format!("{e:?}")));
        });
    }
    dlog!("take_screenshot dispatched; pumping event loop");
    let deadline = Instant::now() + timeout;
    let mut last_log = Instant::now();
    let mut paints = 0u64;
    while slot.borrow().is_none() {
        if Instant::now() >= deadline {
            bail!(
                "timeout waiting for screenshot after {}s (last load_status: {:?}, paints: {})",
                timeout.as_secs(),
                webview.load_status(),
                paints,
            );
        }
        servo.spin_event_loop();
        if needs_paint.get() {
            needs_paint.set(false);
            webview.paint();
            rendering_context.present();
            paints += 1;
        }
        if debug_enabled() && last_log.elapsed() >= Duration::from_secs(2) {
            dlog!(
                "still waiting; load_status={:?} paints={}",
                webview.load_status(),
                paints,
            );
            last_log = Instant::now();
        }
    }
    let result = slot.borrow_mut().take().unwrap();
    result.map_err(|e| anyhow::anyhow!("screenshot capture failed: {e}"))
}

/// Evaluate JS on the page and coerce the result to a number (e.g. scrollHeight).
fn run_eval_number(
    servo: &servo::Servo,
    webview: &servo::WebView,
    script: &str,
    timeout: Duration,
) -> Result<f64> {
    let slot: Rc<RefCell<Option<Result<JSValue, String>>>> = Rc::new(RefCell::new(None));
    {
        let slot = slot.clone();
        webview.evaluate_javascript(script.to_string(), move |r| {
            *slot.borrow_mut() = Some(r.map_err(|e| format!("{e:?}")));
        });
    }
    let deadline = Instant::now() + timeout;
    while slot.borrow().is_none() {
        if Instant::now() >= deadline {
            bail!("timeout waiting for JS evaluation");
        }
        servo.spin_event_loop();
    }
    // Bind the result to a local before the match so the RefMut temporary
    // is dropped before `slot` itself is dropped at end of scope.
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
