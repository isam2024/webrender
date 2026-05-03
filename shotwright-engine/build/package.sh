#!/usr/bin/env bash
# Build the linux artifact via Docker, then assemble a release tarball.
#
# Usage:
#   build/package.sh                # builds x86_64-unknown-linux-gnu
#   ARCH=aarch64 build/package.sh   # builds aarch64-unknown-linux-gnu
#
# Output: dist/shotwright-${VERSION}-${TARGET}.tar.gz
#
# Tarball layout:
#   shotwright-${VERSION}-${TARGET}/
#     shotwright              (the binary)
#     fonts/                  (bundled fonts)
#     LICENSE
#     README.txt              (drop-in instructions)
set -euo pipefail

ARCH="${ARCH:-x86_64}"
TARGET="${ARCH}-unknown-linux-gnu"

cd "$(dirname "$0")/.."
ROOT="$PWD"

VERSION="$(awk -F'"' '/^version/ {print $2; exit}' Cargo.toml)"
NAME="shotwright-${VERSION}-${TARGET}"
DIST="${ROOT}/dist"
STAGE="${DIST}/${NAME}"

rm -rf "${DIST}"
mkdir -p "${STAGE}"

DOCKER_PLATFORM="linux/amd64"
[ "${ARCH}" = "aarch64" ] && DOCKER_PLATFORM="linux/arm64"

# `docker buildx build` doesn't accept --memory directly. To bound the build's
# RAM (so we don't starve other containers on a shared Docker VM), we create
# a one-shot containerized buildx builder with a memory cap, use it, then
# remove it on exit. Tune via BUILD_MEM (e.g. BUILD_MEM=8g for fatter hosts).
BUILD_MEM="${BUILD_MEM:-5g}"
BUILDER="shotwright-bldr-$$"

cleanup_builder() {
    docker buildx rm "${BUILDER}" >/dev/null 2>&1 || true
}
trap cleanup_builder EXIT INT TERM

docker buildx create \
    --name "${BUILDER}" \
    --driver docker-container \
    --driver-opt "memory=${BUILD_MEM},memory-swap=${BUILD_MEM}" \
    --bootstrap >/dev/null

docker buildx build \
    --builder "${BUILDER}" \
    --platform "${DOCKER_PLATFORM}" \
    --build-arg "TARGET=${TARGET}" \
    -f build/Dockerfile.linux \
    --target artifact \
    --output "type=local,dest=${DIST}/_raw" \
    .

cp "${DIST}/_raw/out/shotwright" "${STAGE}/shotwright"
cp -r "${DIST}/_raw/out/fonts" "${STAGE}/fonts"
chmod 0755 "${STAGE}/shotwright"

cp LICENSE 2>/dev/null "${STAGE}/" || echo "(LICENSE missing, skipping)" >&2

cat > "${STAGE}/README.txt" <<EOF
shotwright ${VERSION} (${TARGET})

This is a single-file URL-to-PNG screenshot binary. No system browser, no
display server, no Chromium required.

Drop-in install (cPanel-style shared host):
  1. Upload this whole directory to ~/bin/shotwright/ on your host.
  2. From PHP:
       \$bin = '/home/USER/bin/shotwright/shotwright';
       \$png = shell_exec(escapeshellcmd(\$bin) . ' https://example.com');
     Or use the shotwright PHP composer package.

Requires glibc 2.35 or newer (Ubuntu 22.04+, AlmaLinux 10+, RHEL 10+, Debian 12+).

Bundled fonts live in ./fonts. shotwright will search this directory
relative to its own path, so do NOT separate the binary from the fonts/.
EOF

cd "${DIST}"
tar -czf "${NAME}.tar.gz" "${NAME}"
rm -rf "_raw" "${NAME}"

echo
echo "Built: ${DIST}/${NAME}.tar.gz"
ls -lh "${DIST}/${NAME}.tar.gz"
