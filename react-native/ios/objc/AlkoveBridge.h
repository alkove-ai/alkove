#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

typedef void (^AlkoveDone)(NSString *_Nullable json, NSString *_Nullable code, NSString *_Nullable message);

// implemented in Swift (ios/swift/AlkoveBridge.swift), declared here: no generated -Swift.h, whose place differs under CocoaPods and Swift Package Manager
@interface AlkoveBridge : NSObject

- (instancetype)initWithSend:(void (^)(double id, NSString *type, NSString *json))send;
- (void)compile:(NSString *)text langs:(NSArray<NSString *> *)langs done:(AlkoveDone)done;
- (void)start:(double)id kind:(NSString *)kind options:(NSString *)options done:(AlkoveDone)done;
- (void)call:(double)id method:(NSString *)method arg:(NSString *)arg;
- (void)close:(double)id;
- (void)closeAll;

@end

NS_ASSUME_NONNULL_END
