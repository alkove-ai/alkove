// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "Alkove",
    platforms: [.iOS(.v15), .macOS(.v12)],
    products: [
        .library(name: "Alkove", targets: ["Alkove"]),
    ],
    targets: [
        // ios/build.sh builds it; a release points here to the zip on its GitHub release instead
        .binaryTarget(name: "AlkoveFFI", path: "ios/build/AlkoveFFI.xcframework"),
        .target(name: "Alkove", dependencies: ["AlkoveFFI"], path: "ios/Sources/Alkove"),
        .testTarget(name: "AlkoveTests", dependencies: ["Alkove", "AlkoveFFI"], path: "ios/Tests/AlkoveTests"),
    ]
)
