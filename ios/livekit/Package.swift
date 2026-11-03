// swift-tools-version:5.9
import PackageDescription

// its own package: LiveKit's WebRTC is 2.3 GB of artifacts that an app using Alkove alone must not download
let package = Package(
    name: "AlkoveLiveKit",
    platforms: [.iOS(.v15), .macOS(.v12)],
    products: [
        .library(name: "AlkoveLiveKit", targets: ["AlkoveLiveKit"]),
    ],
    dependencies: [
        .package(path: "../.."),
        .package(url: "https://github.com/livekit/client-sdk-swift.git", from: "2.19.0"),
    ],
    targets: [
        .target(name: "AlkoveLiveKit", dependencies: [.product(name: "Alkove", package: "alkove"), .product(name: "LiveKit", package: "client-sdk-swift")]),
        .testTarget(name: "AlkoveLiveKitTests", dependencies: ["AlkoveLiveKit", .product(name: "Alkove", package: "alkove")]),
    ]
)
