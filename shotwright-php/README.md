# shotwright-php

PHP composer wrapper for the [shotwright](../README.md) URL-to-PNG binary.

## Install

```bash
composer require shotwright/shotwright
vendor/bin/shotwright-install   # downloads matching binary to ~/bin/shotwright/
```

The installer detects your CPU arch (`x86_64` or `aarch64`), downloads the
matching tarball from the GitHub release for the configured version, and
unpacks it to `~/bin/shotwright/`. The library finds it from there
automatically — no further configuration.

If you'd rather drop the binary in by hand, point the library at it:

```php
$shot = new Shotwright(new BinaryLocator('/path/to/shotwright'));
```

…or set the `SHOTWRIGHT_BIN` environment variable.

## Use

```php
use Shotwright\Shotwright;

$shot = new Shotwright();

// Returns PNG bytes
$png = $shot->capture('https://example.com');

// Stream to file (avoids buffering in memory)
$shot->captureToFile('https://example.com', '/tmp/shot.png', [
    'width'     => 1280,
    'height'    => 800,
    'full_page' => true,
    'timeout'   => 30,
    'dpr'       => 1.0,
    'settle_ms' => 200,
    'user_agent' => 'shotwright/0.1',
]);
```

## SSRF guards

By default, requests to `localhost`, `127.0.0.1`, and `::1` are rejected.
This is **not** a complete SSRF defense — you should still use a proper
upstream guard (validate-by-resolution, deny RFC1918, etc.) before passing
URLs in. To disable the built-in host check (e.g. you already validated):

```php
$shot = (new Shotwright())->withoutHostValidation();
```

The binary itself only enforces `http`/`https` schemes — `file://`,
`gopher://`, etc. are always rejected.

## Exceptions

| Class | When |
|---|---|
| `Shotwright\Exception\ShotwrightException` | Bad URL, bad options, env problems |
| `Shotwright\Exception\BinaryNotFoundException` | Binary not on disk |
| `Shotwright\Exception\RenderException` | Binary ran but failed (carries exit code + stderr) |

## Tests

```bash
composer install
composer test
```

The test suite covers `Options` validation only — anything that touches the
binary requires the binary, so it's run as integration tests in CI on
release builds, not as unit tests.
