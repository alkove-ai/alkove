#import "AlkoveReactNative.h"

#import "AlkoveBridge.h"

@implementation AlkoveReactNative {
  AlkoveBridge *_bridge;
}

- (instancetype)init
{
  if (self = [super init]) {
    __weak AlkoveReactNative *weakSelf = self;
    _bridge = [[AlkoveBridge alloc] initWithSend:^(double id, NSString *type, NSString *json) {
      [weakSelf emitOnEvent:@{@"id" : @(id), @"type" : type, @"json" : json}];
    }];
  }
  return self;
}

- (void)configure:(NSString *)config
{
  [_bridge configure:config];
}

- (void)compile:(NSString *)text langs:(NSArray *)langs resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  [_bridge compile:text langs:langs done:^(NSString *json, NSString *code, NSString *message) {
    if (json) resolve(json); else reject(code, message, nil);
  }];
}

- (void)preload:(double)id usage:(NSString *)usage voiceprint:(BOOL)voiceprint typedWords:(BOOL)typedWords resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  [_bridge preload:id usage:usage voiceprint:voiceprint typedWords:typedWords done:^(NSString *json, NSString *code, NSString *message) {
    if (json) resolve(nil); else reject(code, message, nil);
  }];
}

- (void)cached:(NSString *)usage voiceprint:(BOOL)voiceprint typedWords:(BOOL)typedWords resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  resolve(@([_bridge cached:usage voiceprint:voiceprint typedWords:typedWords]));
}

- (void)start:(double)id kind:(NSString *)kind options:(NSString *)options resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  [_bridge start:id kind:kind options:options done:^(NSString *json, NSString *code, NSString *message) {
    if (json) resolve(json); else reject(code, message, nil);
  }];
}

- (void)call:(double)id method:(NSString *)method arg:(NSString *)arg
{
  [_bridge call:id method:method arg:arg];
}

- (void)recent:(double)id resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  [_bridge recent:id done:^(NSString *json, NSString *code, NSString *message) {
    if (json) resolve(json); else reject(code, message, nil);
  }];
}

- (void)record:(double)id seconds:(double)seconds resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  [_bridge record:id seconds:seconds done:^(NSString *json, NSString *code, NSString *message) {
    if (json) resolve(json); else reject(code, message, nil);
  }];
}

- (void)stats:(double)id resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  [_bridge stats:id done:^(NSString *json, NSString *code, NSString *message) {
    if (json) resolve(json); else reject(code, message, nil);
  }];
}

- (void)close:(double)id
{
  [_bridge close:id];
}

- (void)invalidate
{
  [_bridge closeAll];
}

- (std::shared_ptr<facebook::react::TurboModule>)getTurboModule:(const facebook::react::ObjCTurboModule::InitParams &)params
{
  return std::make_shared<facebook::react::NativeAlkoveSpecJSI>(params);
}

+ (NSString *)moduleName
{
  return @"Alkove";
}

@end
