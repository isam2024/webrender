<?php

declare(strict_types=1);

namespace Shotwright;

use Shotwright\Exception\BinaryNotFoundException;

/**
 * Locate the shotwright binary on disk.
 *
 * Resolution order (first hit wins):
 *   1. Explicit path passed to the constructor.
 *   2. SHOTWRIGHT_BIN environment variable.
 *   3. ~/bin/shotwright/shotwright (the documented drop-in install location).
 *   4. The system PATH (`shotwright`).
 *
 * We deliberately do NOT auto-download a binary at install time:
 *   - composer install on shared hosting often runs without network egress
 *   - binaries are platform-specific and can't be sniffed at install time
 *   - users want to know exactly what's on disk
 *
 * Instead, ship a separate `bin/shotwright-install` helper they can run
 * once on the target host (see that script).
 */
final class BinaryLocator
{
    public function __construct(private readonly ?string $explicitPath = null)
    {
    }

    public function find(): string
    {
        foreach ($this->candidates() as $path) {
            if ($path === null || $path === '') {
                continue;
            }
            if (is_executable($path)) {
                return $path;
            }
        }

        throw new BinaryNotFoundException(
            "shotwright binary not found. Checked: explicit path, \$SHOTWRIGHT_BIN, " .
            "~/bin/shotwright/shotwright, and \$PATH. Run `shotwright-install` " .
            "or set SHOTWRIGHT_BIN to a path."
        );
    }

    /** @return iterable<string|null> */
    private function candidates(): iterable
    {
        yield $this->explicitPath;
        yield getenv('SHOTWRIGHT_BIN') ?: null;

        $home = getenv('HOME') ?: null;
        if ($home !== null) {
            yield rtrim($home, '/') . '/bin/shotwright/shotwright';
        }

        yield $this->onPath('shotwright');
    }

    private function onPath(string $name): ?string
    {
        $path = getenv('PATH') ?: '';
        foreach (explode(PATH_SEPARATOR, $path) as $dir) {
            if ($dir === '') {
                continue;
            }
            $candidate = rtrim($dir, '/') . '/' . $name;
            if (is_executable($candidate)) {
                return $candidate;
            }
        }
        return null;
    }
}
