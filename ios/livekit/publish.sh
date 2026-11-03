#!/bin/sh
# ios/livekit/publish.sh X.Y.Z: alkove-livekit-swift as its own repository in ios/build/alkove-livekit-swift, on alkove X.Y.Z by its URL, committed and tagged vX.Y.Z there; it sends nothing
set -eu
cd "$(dirname "$0")"
v=${1:?usage: ios/livekit/publish.sh X.Y.Z}
echo "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || { echo "not a version: $v"; exit 1; }
out=../build/alkove-livekit-swift
mkdir -p "$out"
[ -d "$out/.git" ] || git -C "$out" init -q -b main
if git -C "$out" rev-parse -q --verify "refs/tags/v$v" >/dev/null; then echo "v$v is already tagged in $out: a version is never replaced"; exit 1; fi
rm -rf "$out/Sources"
cp -R Sources "$out/Sources"
cp ../../LICENSE ../../NOTICE "$out/"
sed -e 's|        .package(path: "../.."),|        .package(url: "https://github.com/alkove-ai/alkove.git", from: "'"$v"'"),|' \
    -e '/\.testTarget(/d' Package.swift > "$out/Package.swift"
grep -q 'alkove-ai/alkove.git", from: "'"$v"'"' "$out/Package.swift" || { echo "Package.swift not rewritten"; exit 1; }
printf '.build/\n.swiftpm/\nPackage.resolved\n' > "$out/.gitignore"
cat > "$out/README.md" <<MD
# alkove-livekit-swift

[Alkove](https://github.com/alkove-ai/alkove)'s Focus in LiveKit Swift's microphone path: it keeps the user's voice, the closest one or the voice of a print, and removes the others and the noise, on the device. Built from [\`ios/livekit\`](https://github.com/alkove-ai/alkove/tree/main/ios/livekit) in Alkove's repository, released with it.

\`\`\`swift
.package(url: "https://github.com/alkove-ai/alkove-livekit-swift.git", from: "$v")
\`\`\`

\`\`\`swift
let models = try await Alkove.models(["focus", "cue"])
AudioManager.shared.capturePostProcessingDelegate = try AlkoveFocus(models: models)
\`\`\`

See [the iOS quick start](https://alkove.ai/docs/start/ios/#in-a-livekit-room). Apache-2.0, code and weights.
MD
# the same sources on this checkout's alkove: they must build before they are tagged
check=$(mktemp -d)
cp -R "$out/Sources" "$out/Package.swift" "$check/"
root=$(cd ../.. && pwd)
sed -i '' 's|        .package(url: "https://github.com/alkove-ai/alkove.git", from: "'"$v"'"),|        .package(path: "'"$root"'"),|' "$check/Package.swift"
if ! (cd "$check" && swift build -q >build.log 2>&1); then
  grep -E "error" "$check/build.log" | head -5
  rm -rf "$check"
  echo "the sources do not build on this checkout's alkove: nothing tagged"
  exit 1
fi
rm -rf "$check"
git -C "$out" add -A
git -C "$out" commit -q -m "alkove-livekit-swift $v, on alkove $v"
git -C "$out" tag "v$v"
echo "$out: committed and tagged v$v"
echo "next, by hand, once alkove v$v is out: gh repo create alkove-ai/alkove-livekit-swift --public --source $out --push (the first time), else git -C $out push origin main v$v"
