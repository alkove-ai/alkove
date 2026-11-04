#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

// implemented in Swift (ios/swift/AlkoveLiveKitBridge.swift), declared here as in @alkove/react-native
@interface AlkoveLiveKitBridge : NSObject

- (void)attach:(NSString *)options done:(void (^)(NSString *_Nullable code, NSString *_Nullable message))done;
- (void)detach;
- (void)setEnabled:(BOOL)on;
- (void)setStrength:(double)strength;
- (void)setVoiceprint:(NSString *)voiceprint;
- (double)processed;

@end

NS_ASSUME_NONNULL_END
