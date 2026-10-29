#!/bin/sh
# the bundle the component takes (bundle:): the library for the ESP32-S3, the model, manifest.json with their sha256
# ./dist.sh path/to/spot-q4.mcu   -> dist/alkove-spot-esp-<version>/, its manifest as components/alkove_spot/bundle.json (what the component downloads from the esphome-v<version> release)
set -e
here=$(cd "$(dirname "$0")" && pwd)
model=${1:?the model: a spot_mcu_export.py file (spot-q4.mcu)}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/lib/Cargo.toml")
out="$here/dist/alkove-spot-esp-$version"
. "$HOME/export-esp.sh"
cd "$here/../.."
RUSTFLAGS="--cfg espidf_time64" cargo +esp build --release -p alkove-spot-esp --features chip --target xtensa-esp32s3-espidf -Zbuild-std=std,panic_abort --lib
rm -rf "$out"
mkdir -p "$out"
xtensa-esp32s3-elf-strip --strip-debug -o "$out/libalkove_spot_esp.a" target/xtensa-esp32s3-espidf/release/libalkove_spot_esp.a
cp "$model" "$out/spot-q4.mcu"
cp "$here/lib/include/alk_spot.h" "$out/"
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
cat > "$out/manifest.json" <<JSON
{
  "version": "$version",
  "library": { "file": "libalkove_spot_esp.a", "sha256": "$(sha "$out/libalkove_spot_esp.a")", "target": "xtensa-esp32s3-espidf", "esp_nn": "1.4.1" },
  "model": { "file": "spot-q4.mcu", "sha256": "$(sha "$out/spot-q4.mcu")" }
}
JSON
cp "$out/manifest.json" "$here/components/alkove_spot/bundle.json"
ls -la "$out"
