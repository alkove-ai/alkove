#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
export IPHONEOS_DEPLOYMENT_TARGET=15.0 MACOSX_DEPLOYMENT_TARGET=12.0
out=ios/build
rm -rf "$out/AlkoveFFI.xcframework"
mkdir -p "$out"
args=""
for t in aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin; do
  cargo build --profile ios -p alkove-ios --target "$t"
  args="$args -library target/$t/ios/libalkove.a -headers ios/ffi/include"
done
# shellcheck disable=SC2086
xcodebuild -create-xcframework $args -output "$out/AlkoveFFI.xcframework" >/dev/null
for t in aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin; do
  printf '%s %s\n' "$(du -h "target/$t/ios/libalkove.a" | cut -f1)" "$t"
done
if [ "${1:-}" = "--zip" ]; then
  (cd "$out" && rm -f AlkoveFFI.xcframework.zip && ditto -c -k --keepParent AlkoveFFI.xcframework AlkoveFFI.xcframework.zip)
  swift package compute-checksum "$out/AlkoveFFI.xcframework.zip"
fi
