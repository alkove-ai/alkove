// swift-tools-version:5.9
import Foundation
import PackageDescription

// ios/release.sh sets these two for the zip on the GitHub release vVERSION
let version = "0.4.3"
let checksum = "89d9364645ff0ff77f81868489087c38d425d6b1b8be9ace7ee926af93ae7b79"

// ios/build.sh's XCFramework when it is there (a clone that built it), else the release's zip: a package fetched by SwiftPM has no ios/build
let local = "ios/build/AlkoveFFI.xcframework"
let ffi: Target = FileManager.default.fileExists(atPath: Context.packageDirectory + "/" + local)
    ? .binaryTarget(name: "AlkoveFFI", path: local)
    : .binaryTarget(name: "AlkoveFFI", url: "https://github.com/alkove-ai/alkove/releases/download/v\(version)/AlkoveFFI.xcframework.zip", checksum: checksum)

let package = Package(
    name: "Alkove",
    platforms: [.iOS(.v15), .macOS(.v12)],
    products: [
        .library(name: "Alkove", targets: ["Alkove"]),
    ],
    targets: [
        ffi,
        .target(name: "Alkove", dependencies: ["AlkoveFFI"], path: "ios/Sources/Alkove"),
        .testTarget(name: "AlkoveTests", dependencies: ["Alkove", "AlkoveFFI"], path: "ios/Tests/AlkoveTests"),
    ]
)
