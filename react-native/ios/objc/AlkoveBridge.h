#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

typedef void (^AlkoveDone)(NSString *_Nullable json, NSString *_Nullable code, NSString *_Nullable message);

// implemented in Swift (ios/swift/AlkoveBridge.swift), declared here: no generated -Swift.h, whose place differs under CocoaPods and Swift Package Manager
@interface AlkoveBridge : NSObject

- (instancetype)initWithSend:(void (^)(double id, NSString *type, NSString *json))send;
- (void)configure:(NSString *)config;
- (void)compile:(NSString *)text langs:(NSArray<NSString *> *)langs done:(AlkoveDone)done;
- (void)preload:(double)id usage:(NSString *)usage voiceprint:(BOOL)voiceprint typedWords:(BOOL)typedWords done:(AlkoveDone)done;
- (BOOL)cached:(NSString *)usage voiceprint:(BOOL)voiceprint typedWords:(BOOL)typedWords;
- (void)start:(double)id kind:(NSString *)kind options:(NSString *)options done:(AlkoveDone)done;
- (void)call:(double)id method:(NSString *)method arg:(NSString *)arg;
- (void)recent:(double)id done:(AlkoveDone)done;
- (void)record:(double)id seconds:(double)seconds done:(AlkoveDone)done;
- (void)stats:(double)id done:(AlkoveDone)done;
- (void)close:(double)id;
- (void)closeAll;

@end

NS_ASSUME_NONNULL_END
