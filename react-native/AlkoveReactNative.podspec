require "json"
require "fileutils"

package = JSON.parse(File.read(File.join(__dir__, "package.json")))
vendor = File.join(__dir__, "ios", "vendor")
repo = File.expand_path("..", __dir__)

# inside the alkove repository: the Swift package's sources and ios/build.sh's XCFramework, copied (CocoaPods reads nothing outside the pod)
if File.exist?(File.join(repo, "Package.swift")) && File.directory?(File.join(repo, "ios", "Sources", "Alkove"))
  FileUtils.mkdir_p(File.join(vendor, "Alkove"))
  FileUtils.rm_f(Dir[File.join(vendor, "Alkove", "*.swift")])
  FileUtils.cp(Dir[File.join(repo, "ios", "Sources", "Alkove", "*.swift")], File.join(vendor, "Alkove"))
  built = File.join(repo, "ios", "build", "AlkoveFFI.xcframework.zip")
  FileUtils.cp(built, vendor) if File.exist?(built)
end
# the zip the npm package carries, unpacked once
unless File.directory?(File.join(vendor, "AlkoveFFI.xcframework"))
  system("unzip", "-q", "-o", File.join(vendor, "AlkoveFFI.xcframework.zip"), "-d", vendor) or raise "AlkoveFFI.xcframework.zip missing in #{vendor}"
end

Pod::Spec.new do |s|
  s.name         = "AlkoveReactNative"
  s.version      = package["version"]
  s.summary      = package["description"]
  s.homepage     = package["homepage"]
  s.license      = package["license"]
  s.authors      = package["author"]

  s.platforms    = { :ios => "15.0" }
  s.source       = { :git => "https://github.com/alkove-ai/alkove.git", :tag => "react-native-v#{s.version}" }

  s.source_files = "ios/*.{h,mm,swift}", "ios/vendor/Alkove/*.swift"
  s.private_header_files = "ios/*.h"
  s.vendored_frameworks = "ios/vendor/AlkoveFFI.xcframework"
  s.frameworks = "AVFoundation"
  s.swift_version = "5.9"
  # the XCFramework's simulator slice is arm64 only, as the Swift package's: CocoaPods copies no slice for an x86_64 build
  s.pod_target_xcconfig = { "EXCLUDED_ARCHS[sdk=iphonesimulator*]" => "x86_64" }
  s.user_target_xcconfig = { "EXCLUDED_ARCHS[sdk=iphonesimulator*]" => "x86_64" }

  install_modules_dependencies(s)
end
