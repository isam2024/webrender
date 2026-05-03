<?php

declare(strict_types=1);

namespace Shotwright;

use Shotwright\Exception\RenderException;
use Shotwright\Exception\ShotwrightException;

/**
 * Capture a URL as a PNG image by shelling out to the bundled binary.
 *
 * Usage:
 *
 *   $shot = new Shotwright();
 *   $png  = $shot->capture('https://example.com', ['width' => 1280]);
 *   file_put_contents('shot.png', $png);
 *
 * Or stream straight to a file (avoids buffering the PNG in memory):
 *
 *   $shot->captureToFile('https://example.com', '/tmp/shot.png');
 *
 * URL validation is done HERE because the caller's intent ("an http(s) page
 * the public internet can see") is stricter than what the binary alone enforces.
 * If you have your own SSRF guard upstream, call ::withoutHostValidation() to
 * relax this check — the binary still rejects file://, etc.
 */
final class Shotwright
{
    private bool $validateHost = true;

    public function __construct(
        private readonly BinaryLocator $locator = new BinaryLocator(),
    ) {
    }

    public function withoutHostValidation(): self
    {
        $clone = clone $this;
        $clone->validateHost = false;
        return $clone;
    }

    /**
     * Capture and return PNG bytes.
     *
     * @param array<string, mixed>|Options $opts
     */
    public function capture(string $url, array|Options $opts = []): string
    {
        $tmp = tempnam(sys_get_temp_dir(), 'shotwright-');
        if ($tmp === false) {
            throw new ShotwrightException('failed to allocate temp file');
        }
        try {
            $this->captureToFile($url, $tmp, $opts);
            $bytes = file_get_contents($tmp);
            if ($bytes === false) {
                throw new ShotwrightException('failed to read captured PNG');
            }
            return $bytes;
        } finally {
            @unlink($tmp);
        }
    }

    /**
     * Capture and write PNG to $path.
     *
     * @param array<string, mixed>|Options $opts
     */
    public function captureToFile(string $url, string $path, array|Options $opts = []): void
    {
        $this->assertUrlAllowed($url);
        $options = $opts instanceof Options ? $opts : Options::fromArray($opts);
        $binary  = $this->locator->find();

        $cmd = array_merge(
            [$binary, '--output', $path],
            $options->toCliArgs(),
            [$url],
        );

        $stderr = '';
        $exit   = $this->runProcess($cmd, $options->timeoutSeconds, $stderr);

        if ($exit !== 0) {
            throw new RenderException(
                "shotwright exited {$exit}: " . trim($stderr),
                $exit,
                $stderr,
            );
        }
        if (!is_file($path) || filesize($path) === 0) {
            throw new RenderException(
                'shotwright reported success but produced no PNG',
                0,
                $stderr,
            );
        }
    }

    /**
     * Run the binary with proc_open so we never invoke a shell — every argv
     * entry is passed verbatim, no $url interpolation. We add a hard wallclock
     * timeout slightly larger than the binary's own timeout to catch deadlocks.
     *
     * @param list<string> $cmd
     */
    private function runProcess(array $cmd, int $innerTimeout, string &$stderr): int
    {
        $descriptors = [
            0 => ['pipe', 'r'],
            1 => ['pipe', 'w'],
            2 => ['pipe', 'w'],
        ];
        $proc = proc_open($cmd, $descriptors, $pipes);
        if (!is_resource($proc)) {
            throw new ShotwrightException('failed to spawn shotwright process');
        }
        fclose($pipes[0]);
        stream_set_blocking($pipes[1], false);
        stream_set_blocking($pipes[2], false);

        $hardDeadline = microtime(true) + $innerTimeout + 5;
        $stdout = '';
        $stderr = '';

        while (true) {
            $status = proc_get_status($proc);
            $stdout .= (string) stream_get_contents($pipes[1]);
            $stderr .= (string) stream_get_contents($pipes[2]);

            if (!$status['running']) {
                break;
            }
            if (microtime(true) >= $hardDeadline) {
                proc_terminate($proc, 15);
                usleep(200_000);
                if (proc_get_status($proc)['running']) {
                    proc_terminate($proc, 9);
                }
                fclose($pipes[1]);
                fclose($pipes[2]);
                proc_close($proc);
                throw new RenderException(
                    "shotwright did not exit within {$innerTimeout}s + 5s grace",
                    -1,
                    $stderr,
                );
            }
            usleep(50_000);
        }

        fclose($pipes[1]);
        fclose($pipes[2]);
        return proc_close($proc);
    }

    private function assertUrlAllowed(string $url): void
    {
        $parts = parse_url($url);
        if ($parts === false || empty($parts['scheme']) || empty($parts['host'])) {
            throw new ShotwrightException("invalid URL: {$url}");
        }
        if (!in_array(strtolower($parts['scheme']), ['http', 'https'], true)) {
            throw new ShotwrightException(
                "scheme '{$parts['scheme']}' not allowed: only http/https are permitted"
            );
        }
        if ($this->validateHost) {
            $host = strtolower($parts['host']);
            if ($host === 'localhost' || $host === '127.0.0.1' || $host === '::1') {
                throw new ShotwrightException(
                    "host '{$host}' is blocked. Use withoutHostValidation() if intentional."
                );
            }
        }
    }
}
