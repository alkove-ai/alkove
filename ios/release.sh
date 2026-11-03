#!/bin/sh
# ios/release.sh X.Y.Z: the XCFramework zipped for the GitHub release vX.Y.Z, its checksum in Package.swift; it sends nothing
set -eu
cd "$(dirname "$0")/.."
v=${1:?usage: ios/release.sh X.Y.Z}
echo "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || { echo "not a version: $v"; exit 1; }
ios/build.sh >/dev/null
zip=ios/build/AlkoveFFI.xcframework.zip
rm -f "$zip"
(cd ios/build && ditto -c -k --keepParent AlkoveFFI.xcframework AlkoveFFI.xcframework.zip)
sum=$(swift package compute-checksum "$zip")
perl -pi -e "s/^let version = \".*\"/let version = \"$v\"/; s/^let checksum = \".*\"/let checksum = \"$sum\"/" Package.swift
perl -pi -e "s/^    public static let version = \".*\"/    public static let version = \"$v\"/" ios/Sources/Alkove/Alkove.swift
grep -q "public static let version = \"$v\"" ios/Sources/Alkove/Alkove.swift || { echo "Alkove.swift not updated"; exit 1; }
grep -q "let version = \"$v\"" Package.swift && grep -q "let checksum = \"$sum\"" Package.swift || { echo "Package.swift not updated"; exit 1; }
# the zip as SwiftPM will unpack it: a package on it alone must build and give the version
check=$(mktemp -d)
ditto -x -k "$zip" "$check/unzipped"
mkdir -p "$check/app/Sources/App"
cat > "$check/app/Package.swift" <<SWIFT
// swift-tools-version:5.9
import PackageDescription
let package = Package(name: "App", platforms: [.macOS(.v12)], targets: [
    .binaryTarget(name: "AlkoveFFI", path: "../unzipped/AlkoveFFI.xcframework"),
    .executableTarget(name: "App", dependencies: ["AlkoveFFI"]),
])
SWIFT
cat > "$check/app/Sources/App/main.swift" <<'SWIFT'
import AlkoveFFI
var e = AlkError(kind: 0, message: AlkBytes(ptr: nil, len: 0))
let b = alk_info(&e)
print(e.kind == 0 && b.len > 1000 ? "ok" : "failed")
alk_bytes_free(b)
SWIFT
out=$(cd "$check/app" && swift run -q 2>&1 | tail -1)
rm -rf "$check"
[ "$out" = "ok" ] || { echo "the zip does not work: $out"; exit 1; }
echo "AlkoveFFI.xcframework.zip $(du -h "$zip" | cut -f1), checksum $sum, written to Package.swift for v$v"
echo "next, by hand: commit Package.swift with the release; tag v$v on that commit; gh release upload v$v $zip"
