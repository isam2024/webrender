# shotwright

A self-hosted URL-to-PNG screenshot tool that runs on cheap shared hosting.
No Chromium. No Node. No display server. No root.

`shotwright` ships as:

1. A single static-ish Linux binary built on the [Servo](https://servo.org)
   browser engine — drop it into `~/bin/` on a cPanel-style host and shell
   out to it from PHP.
2. A composer-installable PHP wrapper (`shotwright/shotwright`) that calls
   the binary safely with proc_open, validates inputs, and returns PNG bytes.

This fills the gap left when `wkhtmltoimage` was archived in 2023:
**the "single binary, one shell call, no browser install" shape, but on a
maintained engine.**

## Status

- **Engine**: Servo 0.1.0 (April 2026 release; first formal embedding API)
- **Renderer**: `SoftwareRenderingContext` — pure CPU, no GPU, no display
- **JavaScript**: yes (SpiderMonkey, integrated)
- **Binary size**: ~300 MB stripped (full browser engine; SPA-capable)
- **Memory**: ~200–400 MB per capture
- **Platforms**: x86_64 and aarch64 Linux, glibc ≥ 2.28 (covers AlmaLinux 8+, Rocky 8+, CloudLinux 8+, Ubuntu 20.04+, Debian 11+)

This project is alpha. The binary hasn't been published to GitHub Releases
yet — the build pipeline (`build/Dockerfile.linux`, `build/package.sh`,
`.github/workflows/release.yml`) produces tarballs but no v0.1.0 has shipped.

## Constraints we honored vs. relaxed

| Constraint | Honored? |
|---|---|
| No Chromium, Node, system browser | ✅ |
| No display server | ✅ (SoftwareRenderingContext) |
| Single binary + one shell call | ✅ |
| Composer-installable PHP package | ✅ |
| Active maintenance post-2024 | ✅ (Servo, monthly releases) |
| Modern HTML/CSS, including SPAs | ✅ (SpiderMonkey integrated) |
| 256–512 MB RAM ceiling | ⚠️ tight — full browser; expect 200–400 MB |
| Static binary (musl) | ❌ → glibc 2.28+. cPanel hosts are ~all glibc; full musl static link with mozjs/fontconfig/freetype is impractical |
| Runs on RHEL 7 / CentOS 7 era | ❌ — glibc 2.28 cutoff excludes those |

The musl→glibc relaxation is the main tradeoff. cPanel-style hosting in
2026 is essentially all glibc Linux ≥ 2.31; the binary works there. If
you need to run on RHEL 7, build your own from `build/Dockerfile.linux`
with `FROM centos:7` (substantial work).

## Repo layout

```
shotwright-engine/         Rust crate — the binary
  src/main.rs              CLI surface (clap)
  src/render.rs            Servo driver: load URL, take_screenshot, full-page
  build/Dockerfile.linux   Reproducible build container (AlmaLinux 8)
  build/package.sh         Builds the artifact tarball
shotwright-php/            Composer package — the PHP wrapper
  src/Shotwright.php       Main class: capture(), captureToFile()
  src/Options.php          Strongly-typed options + CLI arg mapping
  src/BinaryLocator.php    Finds the binary on disk
  bin/shotwright-install   One-shot installer for shared hosts
.github/workflows/         CI: builds tarballs on tag, attaches to GH release
```

## Quick start (PHP, end-user)

```bash
composer require shotwright/shotwright
vendor/bin/shotwright-install              # downloads matching binary to ~/bin/shotwright/
```

```php
use Shotwright\Shotwright;

$shot = new Shotwright();
$png  = $shot->capture('https://example.com', [
    'width'     => 1280,
    'height'    => 800,
    'full_page' => true,
]);
file_put_contents('out.png', $png);
```

## Quick start (build the engine yourself)

```bash
cd shotwright-engine
ARCH=x86_64 ./build/package.sh             # ~30 min on cold cache
# → dist/shotwright-0.1.0-x86_64-unknown-linux-gnu.tar.gz
```

## CLI surface

```
shotwright [OPTIONS] <URL>

  -o, --output <PATH>     Output path, or - for stdout (default: -)
      --width <N>         Viewport width in CSS px (default: 1280)
      --height <N>        Viewport height in CSS px (default: 800)
      --full-page         Capture full scrollable page
      --timeout <SECS>    Hard timeout (default: 30)
      --dpr <F>           Device pixel ratio (default: 1.0)
      --settle-ms <N>     Wait N ms after load (default: 0)
      --user-agent <S>    Override UA
      --json-status       Print JSON to stderr on completion
```

Exit codes: `0` success, `1` error (message on stderr).

## Why Servo and not Blitz / Ladybird / a pure-PHP renderer?

- **Blitz** (DioxusLabs): pre-alpha as of May 2026, no public screenshot API,
  GPU-dependent (Vello/wgpu, no documented CPU fallback). Wrong fit.
- **Ladybird/LibWeb**: not yet packaged for embedding; C++ deps make
  static-binary distribution hard.
- **Pure-PHP renderers** (dompdf, mPDF + Imagick): work, but have ~CSS 2.1
  fidelity and no JS — defeats the "modern HTML/CSS" requirement.

Servo is the only project right now that ships a documented headless
screenshot API, has SoftwareRenderingContext for GPU-less rendering,
executes JavaScript, and has been releasing monthly.

## Limitations / known sharp edges

- **Memory**: SpiderMonkey + WebRender alone push past 256 MB on any
  non-trivial page. Plan for 384+ MB headroom on shared hosts.
- **First-build time**: SpiderMonkey takes ~25 min on a typical CI runner.
  Releases will ship pre-built artifacts.
- **glibc floor**: 2.31. Older hosts need a custom build.
- **Servo 0.1.0 itself is "early embedding"** — expect rendering quirks on
  cutting-edge CSS that Chromium handles fine.
- **Resource limits**: if you screenshot untrusted URLs, run inside a
  systemd unit / cgroup with memory + CPU caps. The PHP wrapper enforces a
  process timeout, but a runaway page can still consume a lot before that.

## License

MPL-2.0, matching Servo.
