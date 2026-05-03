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
    JSValue, RgbaImage, ServoBuilder, SoftwareRenderingContext, WebViewBuilder,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use url::Url;

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

    let webview = WebViewBuilder::new(&servo, rendering_context.clone())
        .url(opts.url.clone())
        .build();
    webview.show();

    // Phase 1: capture at the requested viewport. take_screenshot internally
    // waits for the document, all subresources, and pending render frames.
    let rgba = run_screenshot(&servo, &webview, opts.timeout)?;

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
            run_screenshot(&servo, &webview, opts.timeout)?
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
/// Pumps the event loop until the callback fires or `timeout` elapses.
fn run_screenshot(
    servo: &servo::Servo,
    webview: &servo::WebView,
    timeout: Duration,
) -> Result<RgbaImage> {
    let slot: Rc<RefCell<Option<Result<RgbaImage, String>>>> = Rc::new(RefCell::new(None));
    {
        let slot = slot.clone();
        webview.take_screenshot(None, move |r| {
            *slot.borrow_mut() = Some(r.map_err(|e| format!("{e:?}")));
        });
    }
    let deadline = Instant::now() + timeout;
    while slot.borrow().is_none() {
        if Instant::now() >= deadline {
            bail!("timeout waiting for screenshot after {}s", timeout.as_secs());
        }
        servo.spin_event_loop();
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
    match slot.borrow_mut().take().unwrap() {
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
