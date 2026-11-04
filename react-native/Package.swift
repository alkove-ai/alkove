// swift-tools-version: 6.0

import Foundation
import PackageDescription

// React Native reads this manifest through its autolinking link, <app>/ios/build/generated/autolinking/libs/AlkoveReactNative
let here = URL(fileURLWithPath: Context.packageDirectory).resolvingSymlinksInPath()
// inside the alkove repository: its own Swift package, else the published one
let repo = here.deletingLastPathComponent()
let alkove: Package.Dependency = FileManager.default.fileExists(atPath: repo.appendingPathComponent("ios/Sources/Alkove").path)
  ? .package(name: "alkove", path: repo.path)
  : .package(url: "https://github.com/alkove-ai/alkove", from: "0.4.1")

let reactHeaders: [Target.Dependency] = [
  .product(name: "ReactHeaders", package: "ReactNative"),
  .product(name: "ReactNativeHeaders", package: "ReactNative"),
  .product(name: "ReactNativeDependenciesHeaders", package: "ReactNative"),
  .product(name: "ReactAppHeaders", package: "React-GeneratedCode"),
]

// React Native's headers change class layouts on NDEBUG: DEBUG and NDEBUG follow the configuration, as in its own targets
let cxxSettings: [CXXSetting] = [
  .define("RCT_NEW_ARCH_ENABLED", to: "1"),
  .define("DEBUG", to: "1", .when(configuration: .debug)),
  .define("NDEBUG", .when(configuration: .release)),
]

let package = Package(
  name: "AlkoveReactNative",
  platforms: [.iOS(.v15)],
  products: [
    .library(name: "AlkoveReactNative", targets: ["AlkoveReactNative"]),
  ],
  dependencies: [
    .package(name: "ReactNative", path: "../../../../xcframeworks"),
    .package(name: "React-GeneratedCode", path: "../../../ios"),
    alkove,
  ],
  targets: [
    // Swift Package Manager compiles no Swift with Objective-C++ in one target: the bridge on its own
    .target(name: "AlkoveBridge", dependencies: [.product(name: "Alkove", package: "alkove")], path: "ios/swift"),
    .target(name: "AlkoveReactNative", dependencies: reactHeaders + ["AlkoveBridge"], path: "ios/objc", publicHeadersPath: ".", cxxSettings: cxxSettings),
  ],
  swiftLanguageModes: [.v5],
  cxxLanguageStandard: .cxx20
)
