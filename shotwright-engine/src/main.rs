use anyhow::{Context, Result, bail};
use clap::Parser;
use std::path::PathBuf;
use std::time::Duration;
use url::Url;

mod render;

#[derive(Parser, Debug)]
#[command(
    name = "shotwright",
    version,
    about = "Headless URL-to-PNG screenshot CLI",
    long_about = "Renders a URL to a PNG using a bundled Servo-based engine.\n\
                  Designed to be invoked from PHP via exec() on shared hosting:\n\
                  no display server, no Chromium, no system browser required."
)]
struct Cli {
    /// URL to render (http or https only)
    #[arg(value_name = "URL")]
    url: String,

    /// Output file path. Use '-' for stdout.
    #[arg(short, long, value_name = "PATH", default_value = "-")]
    output: String,

    /// Viewport width in CSS pixels
    #[arg(long, default_value_t = 1280)]
    width: u32,

    /// Viewport height in CSS pixels (the rendered image may be taller if --full-page)
    #[arg(long, default_value_t = 800)]
    height: u32,

    /// Capture the full scrollable page, not just the viewport
    #[arg(long)]
    full_page: bool,

    /// Hard timeout in seconds for the entire load+render operation
    #[arg(long, default_value_t = 30)]
    timeout: u64,

    /// Device pixel ratio (1.0 = CSS pixels, 2.0 = retina)
    #[arg(long, default_value_t = 1.0)]
    dpr: f32,

    /// Wait this many milliseconds after load event before capturing
    #[arg(long, default_value_t = 0)]
    settle_ms: u64,

    /// User-Agent string to send
    #[arg(long)]
    user_agent: Option<String>,

    /// Print machine-readable JSON status to stderr on completion
    #[arg(long)]
    json_status: bool,
}

fn main() {
    let cli = Cli::parse();
    let exit_code = match run(cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("shotwright: {e:#}");
            1
        }
    };
    std::process::exit(exit_code);
}

fn run(cli: Cli) -> Result<()> {
    let url = Url::parse(&cli.url).with_context(|| format!("invalid URL: {}", cli.url))?;
    match url.scheme() {
        "http" | "https" => {}
        other => bail!("unsupported URL scheme '{other}': only http/https allowed"),
    }

    if cli.width == 0 || cli.height == 0 {
        bail!("width and height must be > 0");
    }
    if cli.width > 8192 || cli.height > 16384 {
        bail!("requested viewport exceeds safety limits (max 8192x16384)");
    }
    if cli.timeout == 0 || cli.timeout > 600 {
        bail!("--timeout must be between 1 and 600 seconds");
    }
    if !(0.1..=4.0).contains(&cli.dpr) {
        bail!("--dpr must be between 0.1 and 4.0");
    }
    if cli.settle_ms > 60_000 {
        bail!("--settle-ms must not exceed 60000");
    }

    let opts = render::RenderOptions {
        url,
        width: cli.width,
        height: cli.height,
        full_page: cli.full_page,
        timeout: Duration::from_secs(cli.timeout),
        dpr: cli.dpr,
        settle: Duration::from_millis(cli.settle_ms),
        user_agent: cli.user_agent,
    };

    let png = render::capture(opts)?;

    match cli.output.as_str() {
        "-" => {
            use std::io::Write;
            std::io::stdout()
                .write_all(&png)
                .context("writing PNG to stdout")?;
        }
        path => {
            let p = PathBuf::from(path);
            std::fs::write(&p, &png)
                .with_context(|| format!("writing PNG to {}", p.display()))?;
        }
    }

    if cli.json_status {
        eprintln!(
            r#"{{"ok":true,"bytes":{},"width":{},"height":{}}}"#,
            png.len(),
            cli.width,
            cli.height
        );
    }
    Ok(())
}
