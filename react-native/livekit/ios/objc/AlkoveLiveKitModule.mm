#import "AlkoveLiveKitModule.h"

#import "AlkoveLiveKitBridge.h"

@implementation AlkoveLiveKitModule {
  AlkoveLiveKitBridge *_bridge;
}

- (instancetype)init
{
  if (self = [super init]) {
    _bridge = [AlkoveLiveKitBridge new];
  }
  return self;
}

- (void)attach:(NSString *)options resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  [_bridge attach:options done:^(NSString *code, NSString *message) {
    if (code) reject(code, message, nil); else resolve(nil);
  }];
}

- (void)detach
{
  [_bridge detach];
}

- (void)setEnabled:(BOOL)on
{
  [_bridge setEnabled:on];
}

- (void)setStrength:(double)strength
{
  [_bridge setStrength:strength];
}

- (void)setVoiceprint:(NSString *)voiceprint
{
  [_bridge setVoiceprint:voiceprint];
}

- (NSNumber *)processed
{
  return @([_bridge processed]);
}

- (void)invalidate
{
  [_bridge detach];
}

- (std::shared_ptr<facebook::react::TurboModule>)getTurboModule:(const facebook::react::ObjCTurboModule::InitParams &)params
{
  return std::make_shared<facebook::react::NativeAlkoveLiveKitSpecJSI>(params);
}

+ (NSString *)moduleName
{
  return @"AlkoveLiveKit";
}

@end
