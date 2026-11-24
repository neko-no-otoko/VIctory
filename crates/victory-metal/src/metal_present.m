#import <AppKit/AppKit.h>
#import <Metal/Metal.h>
#import <QuartzCore/CAMetalLayer.h>
#import <dispatch/dispatch.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_CAPTURE (16u * 1024u * 1024u)

static void set_error(char *out, size_t capacity, NSString *message) {
    if (out && capacity) snprintf(out, capacity, "%s", message.UTF8String ?: "presentation failed");
}

@interface VictoryPresentDelegate : NSObject <NSWindowDelegate, NSApplicationDelegate>
@property(nonatomic) BOOL closed;
@end
@implementation VictoryPresentDelegate
- (void)windowWillClose:(NSNotification *)notification { (void)notification; self.closed = YES; }
- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender { (void)sender; return NO; }
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender { (void)sender; return NSTerminateCancel; }
@end

static NSWindow *window;
static CAMetalLayer *layer;
static VictoryPresentDelegate *delegate;
static id<MTLDevice> device;
static id<MTLCommandQueue> queue;
static id<MTLTexture> source_texture;
static id<MTLRenderPipelineState> pipeline;
static BOOL running;

static BOOL setup_metal(char *error, size_t error_len) {
    device = MTLCreateSystemDefaultDevice();
    if (!device || !(queue = [device newCommandQueue])) {
        set_error(error, error_len, @"Metal device or command queue unavailable"); return NO;
    }
    NSString *source = @"#include <metal_stdlib>\n"
        "using namespace metal;\n"
        "struct V { float4 pos [[position]]; float2 uv; };\n"
        "vertex V vs(uint i [[vertex_id]]) {\n"
        "  float2 p[3] = {float2(-1,-1),float2(3,-1),float2(-1,3)};\n"
        "  V o; o.pos=float4(p[i],0,1); o.uv=float2((p[i].x+1)*0.5,(1-p[i].y)*0.5); return o; }\n"
        "fragment float4 fs(V in [[stage_in]], texture2d<float> t [[texture(0)]]) {\n"
        "  constexpr sampler s(coord::normalized, address::clamp_to_edge, filter::nearest); return t.sample(s,in.uv); }\n";
    NSError *err = nil;
    id<MTLLibrary> library = [device newLibraryWithSource:source options:nil error:&err];
    if (!library) { set_error(error, error_len, err.localizedDescription ?: @"Metal shader compilation failed"); return NO; }
    MTLRenderPipelineDescriptor *desc = [MTLRenderPipelineDescriptor new];
    desc.vertexFunction = [library newFunctionWithName:@"vs"];
    desc.fragmentFunction = [library newFunctionWithName:@"fs"];
    desc.colorAttachments[0].pixelFormat = MTLPixelFormatBGRA8Unorm;
    pipeline = [device newRenderPipelineStateWithDescriptor:desc error:&err];
    if (!pipeline) { set_error(error, error_len, err.localizedDescription ?: @"Metal pipeline creation failed"); return NO; }
    return YES;
}

static BOOL create_window(uint32_t width, uint32_t height, char *error, size_t error_len) {
    if (delegate.closed) { set_error(error, error_len, @"presentation window is closed"); return NO; }
    if (window) return YES;
    if (!NSScreen.mainScreen) { set_error(error, error_len, @"no display available for presentation"); return NO; }
    NSRect rect = NSMakeRect(0, 0, width, height);
    window = [[NSWindow alloc] initWithContentRect:rect
        styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskResizable | NSWindowStyleMaskMiniaturizable
        backing:NSBackingStoreBuffered defer:NO];
    if (!window) { set_error(error, error_len, @"could not create presentation window"); return NO; }
    window.releasedWhenClosed = NO;
    window.title = @"VIctory Metal Presentation";
    window.delegate = delegate;
    NSView *view = window.contentView;
    view.wantsLayer = YES;
    layer = [CAMetalLayer layer];
    layer.device = device;
    layer.pixelFormat = MTLPixelFormatBGRA8Unorm;
    layer.framebufferOnly = NO;
    layer.opaque = YES;
    view.layer = layer;
    [window center];
    [window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
    fprintf(stderr, "VIctory presentation window number: %ld\n", (long)window.windowNumber);
    return YES;
}

static int draw_frame(uint32_t width, uint32_t height, uint32_t format, uint32_t pitch,
                      const uint8_t *pixels, BOOL capture, uint32_t *out_width,
                      uint32_t *out_height, uint8_t **out_pixels, size_t *out_len,
                      char *error, size_t error_len) {
    if (!running || !delegate || delegate.closed) { set_error(error, error_len, @"presentation window is closed or application is not running"); return -1; }
    if (!width || !height || !pixels || (format != 28 && format != 87) || (uint64_t)pitch < (uint64_t)width * 4 ||
        (uint64_t)(height - 1) * pitch + (uint64_t)width * 4 > MAX_CAPTURE) {
        set_error(error, error_len, @"invalid presentation image"); return -1;
    }
    if (!create_window(width, height, error, error_len)) return -1;
    NSView *view = window.contentView;
    NSRect backing = [view convertRectToBacking:view.bounds];
    if (backing.size.width < 1 || backing.size.height < 1 || backing.size.width > UINT32_MAX || backing.size.height > UINT32_MAX) {
        set_error(error, error_len, @"presentation view has no drawable size"); return -1;
    }
    NSUInteger dw = (NSUInteger)backing.size.width, dh = (NSUInteger)backing.size.height;
    if (capture && ((uint64_t)dw * dh * 4 > MAX_CAPTURE)) {
        set_error(error, error_len, @"presentation capture exceeds 16 MiB"); return -1;
    }
    layer.contentsScale = window.backingScaleFactor;
    layer.drawableSize = CGSizeMake(dw, dh);
    id<CAMetalDrawable> drawable = [layer nextDrawable];
    if (!drawable) { set_error(error, error_len, @"no Metal drawable available"); return -1; }
    dw = drawable.texture.width; dh = drawable.texture.height;
    if (!dw || !dh || dw > UINT32_MAX || dh > UINT32_MAX || (capture && (uint64_t)dw * dh * 4 > MAX_CAPTURE)) {
        set_error(error, error_len, @"invalid drawable dimensions"); return -1;
    }
    MTLPixelFormat source_format = format == 28 ? MTLPixelFormatRGBA8Unorm : MTLPixelFormatBGRA8Unorm;
    if (!source_texture || source_texture.width != width || source_texture.height != height ||
        source_texture.pixelFormat != source_format) {
        MTLTextureDescriptor *td = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:source_format width:width height:height mipmapped:NO];
        td.usage = MTLTextureUsageShaderRead;
        td.storageMode = MTLStorageModeShared;
        source_texture = [device newTextureWithDescriptor:td];
    }
    id<MTLTexture> source = source_texture;
    if (!source) { set_error(error, error_len, @"could not allocate presentation source texture"); return -1; }
    [source replaceRegion:MTLRegionMake2D(0, 0, width, height) mipmapLevel:0 withBytes:pixels bytesPerRow:pitch];
    id<MTLCommandBuffer> commands = [queue commandBuffer];
    MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
    pass.colorAttachments[0].texture = drawable.texture;
    pass.colorAttachments[0].loadAction = MTLLoadActionDontCare;
    pass.colorAttachments[0].storeAction = MTLStoreActionStore;
    id<MTLRenderCommandEncoder> encoder = [commands renderCommandEncoderWithDescriptor:pass];
    if (!commands || !encoder) { set_error(error, error_len, @"could not create presentation render commands"); return -1; }
    [encoder setRenderPipelineState:pipeline];
    [encoder setFragmentTexture:source atIndex:0];
    [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
    [encoder endEncoding];
    id<MTLBuffer> readback = nil;
    NSUInteger row = (dw * 4 + 255) & ~(NSUInteger)255;
    if (capture) {
        readback = [device newBufferWithLength:row * dh options:MTLResourceStorageModeShared];
        id<MTLBlitCommandEncoder> blit = [commands blitCommandEncoder];
        if (!readback || !blit) { set_error(error, error_len, @"could not allocate drawable readback"); return -1; }
        [blit copyFromTexture:drawable.texture sourceSlice:0 sourceLevel:0 sourceOrigin:MTLOriginMake(0, 0, 0)
                   sourceSize:MTLSizeMake(dw, dh, 1) toBuffer:readback destinationOffset:0
          destinationBytesPerRow:row destinationBytesPerImage:row * dh];
        [blit endEncoding];
    }
    [commands presentDrawable:drawable];
    [commands commit];
    [commands waitUntilCompleted];
    if (commands.status != MTLCommandBufferStatusCompleted) {
        set_error(error, error_len, commands.error.localizedDescription ?: @"Metal presentation command failed"); return -1;
    }
    if (capture) {
        size_t length = dw * dh * 4;
        uint8_t *result = malloc(length);
        if (!result) { set_error(error, error_len, @"could not allocate capture result"); return -1; }
        for (NSUInteger y = 0; y < dh; ++y) memcpy(result + y * dw * 4, (const uint8_t *)readback.contents + y * row, dw * 4);
        *out_pixels = result; *out_len = length;
    }
    *out_width = (uint32_t)dw; *out_height = (uint32_t)dh;
    return 0;
}

int victory_present_frame(uint32_t width, uint32_t height, uint32_t format, uint32_t pitch,
                         const uint8_t *pixels, int capture, uint32_t *out_width, uint32_t *out_height,
                         uint8_t **out_pixels, size_t *out_len, char *error, size_t error_len) {
    __block int result;
    void (^work)(void) = ^{ @autoreleasepool {
        result = draw_frame(width, height, format, pitch, pixels, capture != 0, out_width, out_height, out_pixels, out_len, error, error_len);
    }};
    if (NSThread.isMainThread) work(); else dispatch_sync(dispatch_get_main_queue(), work);
    return result;
}

void victory_present_free(uint8_t *pixels) { free(pixels); }

int victory_present_resize(uint32_t width, uint32_t height, char *error, size_t error_len) {
    __block int result = -1;
    void (^work)(void) = ^{
        if (!running || !window || delegate.closed || !width || !height) {
            set_error(error, error_len, @"cannot resize an unavailable presentation window");
            return;
        }
        [window setContentSize:NSMakeSize(width, height)];
        result = 0;
    };
    if (NSThread.isMainThread) work(); else dispatch_sync(dispatch_get_main_queue(), work);
    return result;
}

void victory_present_close(void) {
    void (^work)(void) = ^{ if (window && !delegate.closed) [window close]; };
    if (NSThread.isMainThread) work(); else dispatch_sync(dispatch_get_main_queue(), work);
}

void victory_present_stop(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        [NSApp stop:nil];
        [NSApp postEvent:[NSEvent otherEventWithType:NSEventTypeApplicationDefined location:NSZeroPoint
                        modifierFlags:0 timestamp:0 windowNumber:0 context:nil subtype:0 data1:0 data2:0]
                 atStart:NO];
    });
}

int victory_present_run(void (*start)(void *), void *context, char *error, size_t error_len) {
    @autoreleasepool {
        if (!NSThread.isMainThread || running) { set_error(error, error_len, @"presentation event loop requires the main thread and may only run once"); return -1; }
        [NSApplication sharedApplication];
        if (!NSApp || ![NSApp setActivationPolicy:NSApplicationActivationPolicyRegular]) {
            set_error(error, error_len, @"could not initialize AppKit application"); return -1;
        }
        delegate = [VictoryPresentDelegate new];
        NSApp.delegate = delegate;
        if (!setup_metal(error, error_len)) { NSApp.delegate = nil; delegate = nil; return -1; }
        [NSApp finishLaunching];
        running = YES;
        start(context);
        [NSApp run];
        running = NO;
        if (window && !delegate.closed) [window close];
        window = nil; layer = nil; source_texture = nil; pipeline = nil; queue = nil; device = nil;
        NSApp.delegate = nil; delegate = nil;
        return 0;
    }
}
