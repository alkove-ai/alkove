#!/bin/sh
# the esphome-v<version> release from dist/ (dist.sh), checked against bundle.json; published at the pushed HEAD, its tag then publishes alkove-esphome (release.yml)
set -e
here=$(cd "$(dirname "$0")" && pwd)
b="$here/components/alkove_spot/bundle.json"
v=$(jq -r .version "$b")
dir="$here/dist/alkove-spot-esp-$v"
test "$(sed -n 's/^__version__ = "\(.*\)"/\1/p' "$here/alkove_esphome/__init__.py")" = "$v"
for k in library model; do
  test "$(shasum -a 256 "$dir/$(jq -r ".$k.file" "$b")" | cut -d' ' -f1)" = "$(jq -r ".$k.sha256" "$b")"
done
head=$(git -C "$here" rev-parse HEAD)
git -C "$here" fetch -q origin
git -C "$here" merge-base --is-ancestor "$head" origin/main
tag="esphome-v$v"
printf 'Alkove Spot for ESPHome %s (preview): typed wake words on the ESP32-S3, in place of `micro_wake_word`. The component downloads these files itself and checks them against `bundle.json`.\n\nSee [integrations/esphome](https://github.com/alkove-ai/alkove/tree/%s/integrations/esphome).\n' "$v" "$tag" > "$dir/notes.md"
# a draft first: its tag, which starts release.yml, exists only once every file is up
gh release create "$tag" --draft --target "$head" --title "ESPHome component $v" --notes-file "$dir/notes.md" --latest=false \
  "$dir/libalkove_spot_esp.a" "$dir/spot-q4.mcu" "$dir/alk_spot.h" "$dir/manifest.json"
gh release edit "$tag" --draft=false --latest=false
