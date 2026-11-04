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

- (void)startWake:(double)id words:(NSArray *)words context:(NSString *)context resolve:(RCTPromiseResolveBlock)resolve reject:(RCTPromiseRejectBlock)reject
{
  [_bridge startWake:id words:words context:context done:^(NSString *json, NSString *code, NSString *message) {
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
