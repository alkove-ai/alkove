package com.alkove.reactnative.livekit

import com.facebook.react.BaseReactPackage
import com.facebook.react.bridge.NativeModule
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.module.model.ReactModuleInfo
import com.facebook.react.module.model.ReactModuleInfoProvider

class AlkoveLiveKitPackage : BaseReactPackage() {
  override fun getModule(name: String, reactContext: ReactApplicationContext): NativeModule? =
    if (name == AlkoveLiveKitModule.NAME) AlkoveLiveKitModule(reactContext) else null

  override fun getReactModuleInfoProvider() = ReactModuleInfoProvider {
    mapOf(AlkoveLiveKitModule.NAME to ReactModuleInfo(AlkoveLiveKitModule.NAME, AlkoveLiveKitModule.NAME, false, false, false, true))
  }
}
