#!/usr/bin/env bash
# Build ASCII Painter for Linux (x86_64 or arm64, whichever this machine is) into
# dist/ascii-painter-linux-<arch>.tar.gz. The archive is portable: ascii-painter (GUI), ascii-painter-cli and a
# bundled ffmpeg/ffprobe.
# Needs: cargo, curl, unzip, sha256sum, and the GUI's system libraries, e.g. on Debian/Ubuntu:
#   sudo apt install libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
# Usage: ./build.sh
set -euo pipefail
cd "$(dirname "$0")"

# ffmpeg 9.0.2 static builds (GPL, include libx264) from ffmpeg.martin-riedl.de, pinned by SHA-256.
case "$(uname -m)" in
    x86_64)
        arch=x64
        base=https://ffmpeg.martin-riedl.de/download/linux/amd64/1789931100_9.0.2
        ffmpeg_sha=fa8ecf4abbd290d98f7d188b8649cc6b391ae209a98452be955a15aab1909d7f
        ffprobe_sha=3f428c49070be3d24ec338602b76d412e401ffcb8a5641ef0e729181a232fc32
        ;;
    aarch64 | arm64)
        arch=arm64
        base=https://ffmpeg.martin-riedl.de/download/linux/arm64/1789931697_9.0.2
        ffmpeg_sha=93a76ae90db5474eecdf951a729857c64f3de23567228d6a7d5e6e8e3cd1021b
        ffprobe_sha=bcbe80fb741c180083327afaf5434812e006b33cacde2016b9aeaf6936128330
        ;;
    *)
        echo "unsupported architecture $(uname -m)" >&2
        exit 1
        ;;
esac

target="${CARGO_TARGET_DIR:-target}"
name="ascii-painter-linux-$arch"
out="dist/$name"
cache=dist/cache

echo "==> Building (release)"
cargo build --release --workspace

echo "==> Fetching ffmpeg"
mkdir -p "$cache"
fetch() { # tool sha256
    local archive="$cache/$1-linux-$arch.zip"
    if [[ ! -f "$archive" ]] || [[ "$(sha256sum "$archive" | cut -d' ' -f1)" != "$2" ]]; then
        curl -fsSL "$base/$1.zip" -o "$archive.part"
        local actual
        actual="$(sha256sum "$archive.part" | cut -d' ' -f1)"
        if [[ "$actual" != "$2" ]]; then
            rm -f "$archive.part"
            echo "$1 checksum mismatch: expected $2, got $actual" >&2
            exit 1
        fi
        mv "$archive.part" "$archive"
    fi
    unzip -o -q "$archive" "$1" -d "$out"
    chmod +x "$out/$1"
}

echo "==> Assembling $out"
rm -rf "$out"
mkdir -p "$out"
cp "$target/release/ascii-painter" "$target/release/ascii-painter-cli" README.md "$out/"
cp -r LICENSES "$out/"
fetch ffmpeg "$ffmpeg_sha"
fetch ffprobe "$ffprobe_sha"

tar -czf "dist/$name.tar.gz" -C dist "$name"
echo "==> Done: dist/$name.tar.gz (run $out/ascii-painter)"
