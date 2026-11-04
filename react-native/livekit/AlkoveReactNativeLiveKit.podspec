require "json"

package = JSON.parse(File.read(File.join(__dir__, "package.json")))

Pod::Spec.new do |s|
  s.name         = "AlkoveReactNativeLiveKit"
  s.version      = package["version"]
  s.summary      = package["description"]
  s.homepage     = package["homepage"]
  s.license      = package["license"]
  s.authors      = package["author"]

  s.platforms    = { :ios => "15.0" }
  s.source       = { :git => "https://github.com/alkove-ai/alkove.git", :tag => "react-native-livekit-v#{s.version}" }

  s.source_files = "ios/objc/*.{h,mm}", "ios/swift/*.swift"
  s.private_header_files = "ios/objc/*.h"
  s.swift_version = "5.9"

  s.dependency "AlkoveReactNative"
  s.dependency "livekit-react-native"
  s.dependency "livekit-react-native-webrtc"

  install_modules_dependencies(s)
end
