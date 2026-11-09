#!/usr/bin/env bash
# Builds Noctorium Song Find for one platform and packs it the way a release ships it.
#
#   packaging/package.sh <version> <platform>
#
#   <version>   the release's version, with or without the v: 1.2.3, v1.2.3, 1.2.3-beta.1
#   <platform>  windows-x64   on windows-latest (in bash)  -> dist/noctorium-song-find-<version>-windows-x64.zip
#               linux-x64     on ubuntu-latest             -> dist/noctorium-song-find-<version>-linux-x64.tar.gz
#               macos-arm64   on macos-latest              -> dist/noctorium-song-find-<version>-macos-arm64.zip
#               macos-x64     on macos-latest              -> dist/noctorium-song-find-<version>-macos-x64.zip
#
# The version is built into the program as NOCTORIUM_VERSION, so its About line and its update check know
# which release they are. The last line printed is the path of the file made, and nothing else is left in
# dist/ by this run. The Rust target is the platform's own unless NOCTORIUM_SONG_FIND_TARGET names another --
# x86_64-pc-windows-gnu, say, to try the Windows packaging on a machine with no MSVC.
set -euo pipefail

usage() {
  sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

[ $# -eq 2 ] || usage
version=${1#v}
platform=$2
[ -n "$version" ] || usage

case "$platform" in
  windows-x64) target=x86_64-pc-windows-msvc ;;
  linux-x64) target=x86_64-unknown-linux-gnu ;;
  macos-arm64) target=aarch64-apple-darwin ;;
  macos-x64) target=x86_64-apple-darwin ;;
  *) echo "package.sh: unknown platform '$platform'" >&2; usage ;;
esac
target=${NOCTORIUM_SONG_FIND_TARGET:-$target}

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# Git Bash hands a Windows CARGO_TARGET_DIR through as it is; the paths below want it as bash sees it.
target_dir=${CARGO_TARGET_DIR:-$root/target}
if command -v cygpath > /dev/null 2>&1; then
  target_dir=$(cygpath -u "$target_dir")
fi

if command -v rustup > /dev/null 2>&1; then
  # A no-op when the target is the machine's own or already there; macos-x64 on an Apple silicon runner is
  # the case that needs it.
  rustup target add "$target" > /dev/null 2>&1 || true
fi

echo "==> Building Noctorium Song Find $version for $platform ($target)" >&2
NOCTORIUM_VERSION=$version cargo build --release --locked --target "$target"

exe=noctorium-song-find
case "$platform" in windows-*) exe=noctorium-song-find.exe ;; esac
binary="$target_dir/$target/release/$exe"
[ -f "$binary" ] || { echo "package.sh: the build left no $binary" >&2; exit 1; }

dist="$root/dist"
mkdir -p "$dist"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Run, not just built, where this machine can run it: --version needs no display, and says the version
# that went in is the version that came out.
native=false
case "$(uname -s)/$(uname -m)/$platform" in
  MINGW*/x86_64/windows-x64 | MSYS*/x86_64/windows-x64 | CYGWIN*/x86_64/windows-x64) native=true ;;
  Linux/x86_64/linux-x64) native=true ;;
  Darwin/arm64/macos-arm64 | Darwin/x86_64/macos-x64) native=true ;;
esac
if [ "$native" = true ]; then
  said=$("$binary" --version | tr -d '\r')
  echo "    $said" >&2
  [ "$said" = "Noctorium Song Find $version" ] || { echo "package.sh: it says '$said', not $version" >&2; exit 1; }
fi

case "$platform" in
  windows-x64)
    out="$dist/noctorium-song-find-$version-windows-x64.zip"
    cp "$binary" "$work/Noctorium Song Find.exe"
    rm -f "$out"
    if command -v 7z > /dev/null 2>&1; then
      (cd "$work" && 7z a -tzip -mx=9 "$out" "Noctorium Song Find.exe" > /dev/null)
    else
      powershell -NoProfile -Command "Compress-Archive -LiteralPath '$(cygpath -w "$work/Noctorium Song Find.exe")' -DestinationPath '$(cygpath -w "$out")'"
    fi
    ;;

  linux-x64)
    out="$dist/noctorium-song-find-$version-linux-x64.tar.gz"
    install -m 755 "$binary" "$work/noctorium-song-find"
    install -m 644 packaging/linux/noctorium-song-find.desktop "$work/noctorium-song-find.desktop"
    install -m 644 assets/noctorium-song-find.png "$work/noctorium-song-find.png"
    if command -v desktop-file-validate > /dev/null 2>&1; then
      desktop-file-validate "$work/noctorium-song-find.desktop"
    fi
    # Owned by nobody in particular, so unpacking it as root does not hand the files to the build's user.
    tar -C "$work" --owner=0 --group=0 --numeric-owner -czf "$out" \
      noctorium-song-find noctorium-song-find.desktop noctorium-song-find.png
    ;;

  macos-*)
    out="$dist/noctorium-song-find-$version-$platform.zip"
    app="$work/Noctorium Song Find.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    install -m 755 "$binary" "$app/Contents/MacOS/noctorium-song-find"

    # The bundle's own version has to be numbers and dots: 1.2.3-beta.1 is 1.2.3 there, as it is in the .msi.
    short=${version%%[-+]*}
    minimum=11.0
    [ "$platform" = macos-x64 ] && minimum=10.13
    sed -e "s/@SHORT_VERSION@/$short/g" -e "s/@VERSION@/$version/g" -e "s/@MINIMUM_SYSTEM@/$minimum/g" \
      packaging/macos/Info.plist > "$app/Contents/Info.plist"
    plutil -lint "$app/Contents/Info.plist" > /dev/null
    printf 'APPL????' > "$app/Contents/PkgInfo"

    # The icon, every size Finder and the Dock ask for, from the 512-pixel mark.
    iconset="$work/noctorium-song-find.iconset"
    mkdir -p "$iconset"
    for size in 16 32 128 256 512; do
      sips -z "$size" "$size" assets/noctorium-song-find.png --out "$iconset/icon_${size}x${size}.png" > /dev/null
      double=$((size * 2))
      sips -z "$double" "$double" assets/noctorium-song-find.png --out "$iconset/icon_${size}x${size}@2x.png" > /dev/null
    done
    iconutil -c icns "$iconset" -o "$app/Contents/Resources/noctorium-song-find.icns"

    # Signed ad hoc, which Apple silicon requires of anything it runs and which seals the Info.plist to the
    # program. Not notarised: like Noctorium.app, it is opened the first time with right-click, Open.
    codesign --force --deep --sign - "$app"
    codesign --verify --strict "$app"

    rm -f "$out"
    # ditto rather than zip, so the bundle's permissions and signature come out of the archive intact.
    (cd "$work" && ditto -c -k --keepParent "Noctorium Song Find.app" "$out")
    ;;
esac

echo "==> $(basename "$out"): $(wc -c < "$out" | tr -d ' ') bytes" >&2
echo "$out"
