<?php

declare(strict_types=1);

namespace Shotwright;

use Shotwright\Exception\ShotwrightException;

/**
 * Strongly-typed options for a single capture call.
 *
 * Defaults match the binary's defaults so an empty Options{} round-trips.
 */
final class Options
{
    public function __construct(
        public readonly int $width = 1280,
        public readonly int $height = 800,
        public readonly bool $fullPage = false,
        public readonly int $timeoutSeconds = 30,
        public readonly float $devicePixelRatio = 1.0,
        public readonly int $settleMilliseconds = 0,
        public readonly ?string $userAgent = null,
    ) {
        if ($width <= 0 || $height <= 0) {
            throw new ShotwrightException('width and height must be positive');
        }
        if ($width > 8192 || $height > 16384) {
            throw new ShotwrightException('viewport exceeds safety limits (8192x16384)');
        }
        if ($timeoutSeconds <= 0 || $timeoutSeconds > 600) {
            throw new ShotwrightException('timeout must be between 1 and 600 seconds');
        }
        if ($devicePixelRatio < 0.5 || $devicePixelRatio > 4.0) {
            throw new ShotwrightException('dpr must be between 0.5 and 4.0');
        }
        if ($settleMilliseconds < 0 || $settleMilliseconds > 60000) {
            throw new ShotwrightException('settle_ms must be between 0 and 60000');
        }
    }

    /**
     * Build an Options from an associative array (the public ergonomic shape).
     *
     * @param array<string, mixed> $opts
     */
    public static function fromArray(array $opts): self
    {
        return new self(
            width: (int) ($opts['width'] ?? 1280),
            height: (int) ($opts['height'] ?? 800),
            fullPage: (bool) ($opts['full_page'] ?? false),
            timeoutSeconds: (int) ($opts['timeout'] ?? 30),
            devicePixelRatio: (float) ($opts['dpr'] ?? 1.0),
            settleMilliseconds: (int) ($opts['settle_ms'] ?? 0),
            userAgent: isset($opts['user_agent']) ? (string) $opts['user_agent'] : null,
        );
    }

    /** @return list<string> */
    public function toCliArgs(): array
    {
        $args = [
            '--width', (string) $this->width,
            '--height', (string) $this->height,
            '--timeout', (string) $this->timeoutSeconds,
            '--dpr', (string) $this->devicePixelRatio,
            '--settle-ms', (string) $this->settleMilliseconds,
        ];
        if ($this->fullPage) {
            $args[] = '--full-page';
        }
        if ($this->userAgent !== null) {
            $args[] = '--user-agent';
            $args[] = $this->userAgent;
        }
        return $args;
    }
}
