#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <stdint.h>
#import <stdio.h>
#import <string.h>

static bool fail(char *dst, size_t len, NSString *message) {
    if (len) snprintf(dst, len, "%s", message.UTF8String ?: "Metal error");
    return false;
}

@interface VictoryMetalRenderer : NSObject
@property(nonatomic, strong) id<MTLDevice> device;
@property(nonatomic, strong) id<MTLCommandQueue> queue;
@property(nonatomic, strong) id<MTLRenderPipelineState> pipeline;
@property(nonatomic, strong) id<MTLTexture> texture;
@property(nonatomic, strong) id<MTLBuffer> readback;
@property(nonatomic, strong) id<MTLBuffer> vertices;
@property(nonatomic) uint32_t width;
@property(nonatomic) uint32_t height;
@property(nonatomic) size_t stride;
@end
@implementation VictoryMetalRenderer
@end

static id<MTLLibrary> loadLibrary(id<MTLDevice> device, const uint8_t *bytes,
                                  size_t len, NSError **error) {
    // Retain a copy for as long as Metal retains the dispatch data.
    NSData *owned = [NSData dataWithBytes:bytes length:len];
    dispatch_data_t data = dispatch_data_create(owned.bytes, owned.length,
                                                dispatch_get_main_queue(), ^{ (void)owned; });
    return [device newLibraryWithData:data error:error];
}

void *victory_metal_create(const uint8_t *vertex_data, size_t vertex_len, const char *vertex_name,
                           const uint8_t *fragment_data, size_t fragment_len, const char *fragment_name,
                           uint32_t width, uint32_t height, char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
#define FAIL(message) do { fail(error, error_len, (message)); return NULL; } while (0)
        VictoryMetalRenderer *r = [VictoryMetalRenderer new];
        r.device = MTLCreateSystemDefaultDevice();
        if (!r.device) FAIL(@"no Metal device");
        NSError *e = nil;
        id<MTLLibrary> vl = loadLibrary(r.device, vertex_data, vertex_len, &e);
        if (!vl) FAIL(e.localizedDescription ?: @"vertex library loading failed");
        id<MTLFunction> vertex = [vl newFunctionWithName:[NSString stringWithUTF8String:vertex_name]];
        if (!vertex) FAIL(@"vertex function not found");
        id<MTLLibrary> fl = loadLibrary(r.device, fragment_data, fragment_len, &e);
        if (!fl) FAIL(e.localizedDescription ?: @"fragment library loading failed");
        id<MTLFunction> fragment = [fl newFunctionWithName:[NSString stringWithUTF8String:fragment_name]];
        if (!fragment) FAIL(@"fragment function not found");
        MTLVertexDescriptor *layout = [MTLVertexDescriptor vertexDescriptor];
        // Apple's converter reserves attributes 0-10 and vertex buffers 0-5.
        layout.attributes[11].format = MTLVertexFormatFloat4;
        layout.attributes[11].offset = 0;
        layout.attributes[11].bufferIndex = 6;
        layout.layouts[6].stride = 4 * sizeof(float);
        layout.layouts[6].stepFunction = MTLVertexStepFunctionPerVertex;
        MTLRenderPipelineDescriptor *desc = [MTLRenderPipelineDescriptor new];
        desc.vertexFunction = vertex;
        desc.fragmentFunction = fragment;
        desc.vertexDescriptor = layout;
        desc.colorAttachments[0].pixelFormat = MTLPixelFormatRGBA8Unorm;
        r.pipeline = [r.device newRenderPipelineStateWithDescriptor:desc error:&e];
        if (!r.pipeline) FAIL(e.localizedDescription ?: @"pipeline creation failed");
        MTLTextureDescriptor *texture = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm width:width height:height mipmapped:NO];
        texture.storageMode = MTLStorageModePrivate;
        texture.usage = MTLTextureUsageRenderTarget;
        r.texture = [r.device newTextureWithDescriptor:texture];
        if (!r.texture) FAIL(@"texture allocation failed");
        size_t row = (size_t)width * 4;
        r.stride = (row + 255) & ~(size_t)255;
        if (r.stride > SIZE_MAX / height) FAIL(@"readback dimensions overflow");
        r.readback = [r.device newBufferWithLength:r.stride * height options:MTLResourceStorageModeShared];
        r.queue = [r.device newCommandQueue];
        if (!r.readback || !r.queue) FAIL(@"readback or queue allocation failed");
        r.width = width;
        r.height = height;
#undef FAIL
        return (__bridge_retained void *)r;
    }
}

bool victory_metal_draw(void *handle, const float *vertices, uint32_t count, uint8_t *pixels,
                        char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        VictoryMetalRenderer *r = (__bridge VictoryMetalRenderer *)handle;
        size_t size = (size_t)count * 4 * sizeof(float);
        if (!r.vertices || r.vertices.length < size)
            r.vertices = [r.device newBufferWithLength:size options:MTLResourceStorageModeShared];
        if (!r.vertices) return fail(error, error_len, @"vertex buffer allocation failed");
        memcpy(r.vertices.contents, vertices, size);
        MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
        pass.colorAttachments[0].texture = r.texture;
        pass.colorAttachments[0].loadAction = MTLLoadActionClear;
        pass.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 0);
        pass.colorAttachments[0].storeAction = MTLStoreActionStore;
        id<MTLCommandBuffer> command = [r.queue commandBuffer];
        id<MTLRenderCommandEncoder> render = [command renderCommandEncoderWithDescriptor:pass];
        if (!command || !render) return fail(error, error_len, @"render command creation failed");
        [render setRenderPipelineState:r.pipeline];
        [render setVertexBuffer:r.vertices offset:0 atIndex:6];
        [render drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:count];
        [render endEncoding];
        id<MTLBlitCommandEncoder> blit = [command blitCommandEncoder];
        if (!blit) return fail(error, error_len, @"blit command creation failed");
        [blit copyFromTexture:r.texture sourceSlice:0 sourceLevel:0 sourceOrigin:MTLOriginMake(0, 0, 0)
                   sourceSize:MTLSizeMake(r.width, r.height, 1) toBuffer:r.readback destinationOffset:0
              destinationBytesPerRow:r.stride destinationBytesPerImage:r.stride * r.height];
        [blit endEncoding];
        [command commit];
        [command waitUntilCompleted];
        if (command.status != MTLCommandBufferStatusCompleted)
            return fail(error, error_len, command.error.localizedDescription ?: @"GPU command failed");
        size_t row = (size_t)r.width * 4;
        for (uint32_t y = 0; y < r.height; ++y)
            memcpy(pixels + (size_t)y * row, (const uint8_t *)r.readback.contents + (size_t)y * r.stride, row);
        return true;
    }
}

void victory_metal_destroy(void *handle) {
    @autoreleasepool {
        VictoryMetalRenderer *r = (__bridge_transfer VictoryMetalRenderer *)handle;
        (void)r;
    }
}
