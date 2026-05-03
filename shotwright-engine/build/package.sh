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

# Memory cap is opt-in. Set BUILD_MEM (e.g. BUILD_MEM=5g) when building on a
# shared Docker VM that's running other containers — it spawns a containerized
# buildx builder with a hard ceiling so the build can't starve them. CI
# runners have dedicated RAM (~16 GB on GH ubuntu-latest), so leave it unset
# there to give rustc the room it needs for servo-script.
BUILDX_FLAGS=()
if [ -n "${BUILD_MEM:-}" ]; then
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
    BUILDX_FLAGS+=(--builder "${BUILDER}")
fi

docker buildx build \
    "${BUILDX_FLAGS[@]}" \
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
