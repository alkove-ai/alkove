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

- (void)compile:(NSString *)text langs:(NSArray *)langs resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  [_bridge compile:text langs:langs done:^(NSString *json, NSString *code, NSString *message) {
    if (json) resolve(json); else reject(code, message, nil);
  }];
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
