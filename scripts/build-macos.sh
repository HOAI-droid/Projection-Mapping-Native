#!/bin/bash
# Baut „Projection Mapping.app“ (Apple Silicon) inklusive projectM für Milkdrop.
#
#   ./scripts/build-macos.sh            → dist/Projection Mapping.app und dist/ProjectionMapping-macOS.zip
#
# Voraussetzungen: Xcode-Kommandozeilenwerkzeuge (xcode-select --install), Rust (https://rustup.rs),
# cmake und git (brew install cmake). Für Videos/Kamera/Player zur Laufzeit: brew install ffmpeg.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)

# projectM ab 4.2 (zeichnet in ein eigenes Framebuffer-Objekt) – feste Version für reproduzierbare Builds
PROJECTM_REV=${PROJECTM_REV:-e98fca85e57802d27a6d11499642de2a1d5e994e}
PM_SRC="$ROOT/build/projectm-src"
PM_OUT="$ROOT/build/projectm"
export MACOSX_DEPLOYMENT_TARGET=12.0

if [ ! -f "$PM_OUT/lib/libprojectM-4.dylib" ]; then
  echo "▶ projectM bauen ($PROJECTM_REV)"
  rm -rf "$PM_SRC"
  git init -q "$PM_SRC"
  git -C "$PM_SRC" remote add origin https://github.com/projectM-visualizer/projectm.git
  git -C "$PM_SRC" fetch -q --depth 1 origin "$PROJECTM_REV"
  git -C "$PM_SRC" checkout -q FETCH_HEAD
  git -C "$PM_SRC" submodule update -q --init --depth 1
  cmake -S "$PM_SRC" -B "$PM_SRC/build" -DCMAKE_BUILD_TYPE=Release -DENABLE_PLAYLIST=OFF \
    -DENABLE_SYSTEM_PROJECTM_EVAL=OFF -DBUILD_TESTING=OFF -DBUILD_SHARED_LIBS=ON \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=12.0 -DCMAKE_INSTALL_PREFIX="$PM_OUT"
  cmake --build "$PM_SRC/build" --parallel
  cmake --install "$PM_SRC/build"
fi

echo "▶ App bauen"
cargo build --release

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
APP="$ROOT/dist/Projection Mapping.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp target/release/projection-mapping "$APP/Contents/MacOS/"
sed "s/__VERSION__/$VERSION/g" macos/Info.plist > "$APP/Contents/Info.plist"

# projectM mitliefern (echte Datei, nicht der Symlink)
cp -L "$PM_OUT/lib/libprojectM-4.dylib" "$APP/Contents/Frameworks/libprojectM-4.dylib"
install_name_tool -id "@rpath/libprojectM-4.dylib" "$APP/Contents/Frameworks/libprojectM-4.dylib"
cp "$PM_SRC/LICENSE.txt" "$APP/Contents/Resources/projectM-LICENSE.txt" 2>/dev/null || true

# Symbol aus dem PNG
ICONSET="$ROOT/build/AppIcon.iconset"
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s macos/AppIcon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) macos/AppIcon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

# Ad-hoc signieren (ohne Apple-Entwicklerkonto)
codesign --force --sign - "$APP/Contents/Frameworks/libprojectM-4.dylib"
codesign --force --sign - "$APP"

(cd dist && rm -f ProjectionMapping-macOS.zip && ditto -c -k --keepParent "Projection Mapping.app" ProjectionMapping-macOS.zip)
echo "✔ Fertig: $APP"
echo "  Zip: $ROOT/dist/ProjectionMapping-macOS.zip"
