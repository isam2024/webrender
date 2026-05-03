<?php
// Drop this anywhere PHP runs on your JaguarPC host (e.g. public_html/probe.php),
// hit it once in your browser, paste the output back. It checks every constraint
// shotwright actually needs: glibc, exec, memory ceiling, binary execution.
//
// Safe — runs only stdlib commands. Delete the file after.

declare(strict_types=1);
header('Content-Type: text/plain; charset=utf-8');

function ok(string $msg): void   { echo "[ok]   $msg\n"; }
function bad(string $msg): void  { echo "[FAIL] $msg\n"; }
function warn(string $msg): void { echo "[warn] $msg\n"; }
function info(string $msg): void { echo "       $msg\n"; }

echo "=== shotwright host probe ===\n";
echo "Date: " . date('c') . "\n\n";

// 1. PHP basics
echo "--- PHP ---\n";
info("Version: " . PHP_VERSION);
info("SAPI:    " . php_sapi_name());
info("uname:   " . php_uname());

$disabled = ini_get('disable_functions') ?: '(none)';
info("disable_functions: $disabled");

foreach (['proc_open', 'shell_exec', 'exec', 'popen', 'passthru', 'system'] as $fn) {
    if (function_exists($fn) && !in_array($fn, array_map('trim', explode(',', $disabled)), true)) {
        ok("$fn is callable");
    } else {
        bad("$fn is disabled or unavailable");
    }
}

$openBase = ini_get('open_basedir') ?: '(unset)';
info("open_basedir: $openBase");

echo "\n--- OS / glibc ---\n";

// 2. OS release
if (is_readable('/etc/os-release')) {
    $rel = file_get_contents('/etc/os-release');
    foreach (explode("\n", $rel) as $line) {
        if (preg_match('/^(NAME|VERSION|PRETTY_NAME|VERSION_ID|PLATFORM_ID)=/', $line)) {
            info($line);
        }
    }
} else {
    warn("/etc/os-release not readable");
}

// 3. glibc version — the make-or-break constraint (need >= 2.35)
$glibc = null;
foreach (['/lib64/libc.so.6', '/lib/x86_64-linux-gnu/libc.so.6', '/lib/aarch64-linux-gnu/libc.so.6', '/lib/libc.so.6'] as $libc) {
    if (is_executable($libc)) {
        $out = @shell_exec("$libc 2>&1 | head -1");
        if ($out && preg_match('/release version (\d+\.\d+)/', $out, $m)) {
            $glibc = $m[1];
            break;
        }
    }
}
if ($glibc) {
    if (version_compare($glibc, '2.35', '>=')) {
        ok("glibc $glibc (>= 2.35, shotwright will run)");
    } else {
        bad("glibc $glibc (< 2.35, shotwright binary will NOT run on this host)");
    }
} else {
    warn("could not detect glibc version");
}

echo "\n--- arch ---\n";
$arch = trim((string) @shell_exec('uname -m'));
info("arch: $arch");
if (in_array($arch, ['x86_64', 'aarch64', 'arm64'], true)) {
    ok("supported architecture");
} else {
    bad("unsupported architecture (need x86_64 or aarch64)");
}

echo "\n--- memory ---\n";

// 4. CloudLinux LVE cap (the most common shared-hosting killer)
if (is_readable('/proc/user_beancounters')) {
    info("/proc/user_beancounters present (OpenVZ/Virtuozzo container)");
}
$cgPaths = [
    '/sys/fs/cgroup/memory/lve/memory.limit_in_bytes',
    '/sys/fs/cgroup/memory.max',
    '/sys/fs/cgroup/lve/memory.max',
];
$found_cap = false;
foreach ($cgPaths as $p) {
    if (is_readable($p)) {
        $val = trim((string) file_get_contents($p));
        if ($val && $val !== 'max' && is_numeric($val)) {
            $mb = (int)$val / 1024 / 1024;
            info("cgroup limit at $p: " . round($mb) . " MB");
            $found_cap = true;
            if ($mb < 512) {
                bad("memory cap below 512 MB — shotwright will get killed during render");
            } elseif ($mb < 1024) {
                warn("memory cap " . round($mb) . " MB — tight; rendering complex pages may OOM");
            } else {
                ok("memory cap " . round($mb) . " MB — adequate");
            }
        }
    }
}
if (!$found_cap) {
    info("no cgroup memory cap detected (good — or running in a non-cgroup container)");
}

// LVE-specific
if (is_executable('/usr/sbin/lvectl') || is_executable('/usr/sbin/lveps')) {
    info("LVE/CloudLinux tooling detected — there IS a per-user LVE; check it via cPanel resource panel");
}

echo "\n--- can we execute a downloaded binary? ---\n";

// 5. Probe-test executing a tiny binary in home dir.
$home = getenv('HOME') ?: '/tmp';
$probeDir = rtrim($home, '/') . '/.shotwright-probe';
@mkdir($probeDir, 0o755, true);

$arch_for_url = $arch === 'arm64' ? 'aarch64' : $arch;
// busybox-static is small (~1 MB), self-contained, glibc-independent enough
// for a "can I execute anything I download" smoke test.
$urls = [
    "https://busybox.net/downloads/binaries/1.35.0-$arch_for_url-linux-musl/busybox",
    "https://github.com/canonical/busybox/raw/master/busybox-$arch_for_url",
];
$probeBin = "$probeDir/busybox";
$got = false;
foreach ($urls as $url) {
    $ctx = stream_context_create(['http' => ['timeout' => 8]]);
    $body = @file_get_contents($url, false, $ctx);
    if ($body !== false && strlen($body) > 100000) {
        file_put_contents($probeBin, $body);
        chmod($probeBin, 0o755);
        $got = true;
        info("downloaded test binary from $url (" . strlen($body) . " bytes)");
        break;
    }
}
if ($got) {
    $out = @shell_exec(escapeshellarg($probeBin) . " --help 2>&1 | head -3");
    if ($out && stripos($out, 'busybox') !== false) {
        ok("downloaded binary executed successfully — exec() chain works");
    } else {
        bad("could not execute downloaded binary. Output: " . trim((string)$out));
    }
    @unlink($probeBin);
    @rmdir($probeDir);
} else {
    warn("could not download a test binary (outbound fetch may be blocked, or both URLs are down)");
}

echo "\n--- summary ---\n";
echo "If every line above starts with [ok], shotwright will run here.\n";
echo "Any [FAIL] line is a hard blocker.\n";
echo "[warn] lines are usually fine but worth eyeballing.\n";
