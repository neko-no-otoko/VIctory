#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <stdint.h>
#import <stdio.h>
#import <string.h>

static bool fail(char *dst, size_t len, NSString *message) {
    if (len) snprintf(dst, len, "%s", message.UTF8String ?: "Metal error");
    return false;
}
// The console backend stores texture contents in linear guest memory. Upload,
// clear on a real Metal texture, and copy back synchronously so that clears,
// draws and readbacks remain ordered in its existing submission pipeline.
bool victory_metal_clear(uint32_t width, uint32_t height, size_t pitch, uint8_t *pixels,
                         const uint8_t *color, const uint32_t (*rects)[4], size_t count,
                         char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        static id<MTLDevice> device;
        static id<MTLCommandQueue> queue;
        static id<MTLComputePipelineState> pipeline;
        static NSString *setupError;
        static dispatch_once_t once;
        dispatch_once(&once, ^{
            device = MTLCreateSystemDefaultDevice();
            if (!device) { setupError = @"no Metal device"; return; }
            NSError *e = nil;
            NSString *source = @"#include <metal_stdlib>\nusing namespace metal;\n"
                "kernel void clear_target(texture2d<float, access::write> target [[texture(0)]], "
                "constant uint4 &rect [[buffer(0)]], constant uchar4 &color [[buffer(1)]], "
                "uint2 p [[thread_position_in_grid]]) { "
                "uint2 xy = p + rect.xy; if (xy.x < rect.z && xy.y < rect.w) "
                "target.write(float4(color) / 255.0f, xy); }";
            id<MTLLibrary> library = [device newLibraryWithSource:source options:nil error:&e];
            id<MTLFunction> function = [library newFunctionWithName:@"clear_target"];
            if (function) pipeline = [device newComputePipelineStateWithFunction:function error:&e];
            queue = [device newCommandQueue];
            if (!pipeline || !queue) setupError = e.localizedDescription ?: @"clear pipeline creation failed";
        });
        if (setupError) return fail(error, error_len, setupError);
        MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm
                                                                                       width:width height:height mipmapped:NO];
        desc.storageMode = MTLStorageModeShared;
        desc.usage = MTLTextureUsageShaderWrite;
        id<MTLTexture> texture = [device newTextureWithDescriptor:desc];
        if (!texture) return fail(error, error_len, @"clear texture allocation failed");
        MTLRegion region = MTLRegionMake2D(0, 0, width, height);
        [texture replaceRegion:region mipmapLevel:0 withBytes:pixels bytesPerRow:pitch];
        id<MTLCommandBuffer> command = [queue commandBuffer];
        id<MTLComputeCommandEncoder> encoder = [command computeCommandEncoder];
        if (!command || !encoder) return fail(error, error_len, @"clear command creation failed");
        [encoder setComputePipelineState:pipeline];
        [encoder setTexture:texture atIndex:0];
        [encoder setBytes:color length:4 atIndex:1];
        const uint32_t full[4] = {0, 0, width, height};
        size_t n = count ? count : 1;
        for (size_t i = 0; i < n; ++i) {
            const uint32_t *rect = count ? rects[i] : full;
            uint32_t w = rect[2] - rect[0], h = rect[3] - rect[1];
            if (!w || !h) continue;
            [encoder setBytes:rect length:4 * sizeof(uint32_t) atIndex:0];
            [encoder dispatchThreads:MTLSizeMake(w, h, 1) threadsPerThreadgroup:MTLSizeMake(8, 8, 1)];
        }
        [encoder endEncoding];
        [command commit];
        [command waitUntilCompleted];
        if (command.status != MTLCommandBufferStatusCompleted)
            return fail(error, error_len, command.error.localizedDescription ?: @"GPU clear failed");
        [texture getBytes:pixels bytesPerRow:pitch fromRegion:region mipmapLevel:0];
        return true;
    }
}


@interface VictoryMetalRenderer : NSObject
@property(nonatomic, strong) id<MTLDevice> device;
@property(nonatomic, strong) id<MTLCommandQueue> queue;
@property(nonatomic, strong) id<MTLRenderPipelineState> pipeline;
@property(nonatomic, strong) id<MTLTexture> texture;
@property(nonatomic, strong) id<MTLBuffer> readback;
@property(nonatomic, strong) id<MTLBuffer> vertices;
@property(nonatomic, strong) id<MTLBuffer> fragmentConstants;
@property(nonatomic, strong) id<MTLBuffer> fragmentArguments;
@property(nonatomic, strong) id<MTLTexture> sampledTexture;
@property(nonatomic, strong) id<MTLSamplerState> sampler;
@property(nonatomic, strong) id<MTLBuffer> textureHeap;
@property(nonatomic, strong) id<MTLBuffer> samplerHeap;
@property(nonatomic) BOOL textureEnabled;
@property(nonatomic) BOOL linearFilter;
@property(nonatomic) size_t fragmentConstantBytes;
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
                           uint32_t width, uint32_t height, uint32_t constant_dwords,
                           bool texture_enabled, char *error, size_t error_len) {
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
        if (constant_dwords) {
            r.fragmentConstantBytes = (size_t)constant_dwords * sizeof(uint32_t);
            r.fragmentConstants = [r.device newBufferWithLength:r.fragmentConstantBytes options:MTLResourceStorageModeShared];
            r.fragmentArguments = [r.device newBufferWithLength:24 options:MTLResourceStorageModeShared];
            if (!r.fragmentConstants || !r.fragmentArguments) FAIL(@"fragment constant allocation failed");
            // Converter's automatic linear CBV descriptor: GPU address,
            // unused texture ID, and buffer byte length in the low 32 bits.
            uint64_t *entry = r.fragmentArguments.contents;
            entry[0] = r.fragmentConstants.gpuAddress;
            entry[1] = 0;
            entry[2] = r.fragmentConstantBytes;
        }
        if (texture_enabled) {
            if (r.device.argumentBuffersSupport != MTLArgumentBuffersTier2)
                FAIL(@"texture binding requires argument buffers tier 2");
            r.textureHeap = [r.device newBufferWithLength:24 options:MTLResourceStorageModeShared];
            r.samplerHeap = [r.device newBufferWithLength:24 options:MTLResourceStorageModeShared];
            if (!r.textureHeap || !r.samplerHeap) FAIL(@"fragment descriptor heap allocation failed");
            r.textureEnabled = YES;
        }
        r.width = width;
        r.height = height;
#undef FAIL
        return (__bridge_retained void *)r;
    }
}

bool victory_metal_set_texture(void *handle, uint32_t width, uint32_t height, const uint8_t *rgba,
                               bool linear, char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        VictoryMetalRenderer *r = (__bridge VictoryMetalRenderer *)handle;
        if (!r.textureEnabled) return fail(error, error_len, @"texture binding is not enabled");
        id<MTLTexture> texture = r.sampledTexture;
        if (!texture || texture.width != width || texture.height != height) {
            MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm width:width height:height mipmapped:NO];
            desc.textureType = MTLTextureType2DArray;
            desc.arrayLength = 1;
            desc.storageMode = MTLStorageModeShared;
            desc.usage = MTLTextureUsageShaderRead;
            texture = [r.device newTextureWithDescriptor:desc];
            if (!texture) return fail(error, error_len, @"sampled texture allocation failed");
        }
        id<MTLSamplerState> sampler = r.sampler;
        if (!sampler || r.linearFilter != linear) {
            MTLSamplerDescriptor *desc = [MTLSamplerDescriptor new];
            desc.minFilter = linear ? MTLSamplerMinMagFilterLinear : MTLSamplerMinMagFilterNearest;
            desc.magFilter = desc.minFilter;
            desc.sAddressMode = MTLSamplerAddressModeClampToEdge;
            desc.tAddressMode = MTLSamplerAddressModeClampToEdge;
            desc.supportArgumentBuffers = YES;
            sampler = [r.device newSamplerStateWithDescriptor:desc];
            if (!sampler) return fail(error, error_len, @"sampler allocation failed");
        }
        [texture replaceRegion:MTLRegionMake2D(0, 0, width, height) mipmapLevel:0 slice:0
                     withBytes:rgba bytesPerRow:(size_t)width * 4 bytesPerImage:(size_t)width * height * 4];
        // IRDescriptorTableEntry: texture uses textureViewID, sampler uses
        // gpuVA; metadata is a zero min-LOD clamp or zero sampler LOD bias.
        uint64_t *tex = r.textureHeap.contents;
        tex[0] = 0; tex[1] = texture.gpuResourceID._impl; tex[2] = 0;
        uint64_t *smp = r.samplerHeap.contents;
        smp[0] = sampler.gpuResourceID._impl; smp[1] = 0; smp[2] = 0;
        r.sampledTexture = texture;
        r.sampler = sampler;
        r.linearFilter = linear;
        return true;
    }
}

bool victory_metal_draw(void *handle, const float *vertices, uint32_t count,
                        const uint32_t *constants, uint8_t *pixels, bool preserve,
                        char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        VictoryMetalRenderer *r = (__bridge VictoryMetalRenderer *)handle;
        size_t size = (size_t)count * 4 * sizeof(float);
        if (!r.vertices || r.vertices.length < size)
            r.vertices = [r.device newBufferWithLength:size options:MTLResourceStorageModeShared];
        if (!r.vertices) return fail(error, error_len, @"vertex buffer allocation failed");
        memcpy(r.vertices.contents, vertices, size);
        if (r.fragmentConstants)
            memcpy(r.fragmentConstants.contents, constants, r.fragmentConstantBytes);
        id<MTLCommandBuffer> command = [r.queue commandBuffer];
        if (!command) return fail(error, error_len, @"render command creation failed");
        if (preserve) {
            size_t row = (size_t)r.width * 4;
            for (uint32_t y = 0; y < r.height; ++y)
                memcpy((uint8_t *)r.readback.contents + (size_t)y * r.stride, pixels + (size_t)y * row, row);
            id<MTLBlitCommandEncoder> upload = [command blitCommandEncoder];
            if (!upload) return fail(error, error_len, @"upload blit command creation failed");
            [upload copyFromBuffer:r.readback sourceOffset:0 sourceBytesPerRow:r.stride
               sourceBytesPerImage:r.stride * r.height sourceSize:MTLSizeMake(r.width, r.height, 1)
                         toTexture:r.texture destinationSlice:0 destinationLevel:0
                 destinationOrigin:MTLOriginMake(0, 0, 0)];
            [upload endEncoding];
        }
        MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
        pass.colorAttachments[0].texture = r.texture;
        pass.colorAttachments[0].loadAction = preserve ? MTLLoadActionLoad : MTLLoadActionClear;
        pass.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 0);
        pass.colorAttachments[0].storeAction = MTLStoreActionStore;
        id<MTLRenderCommandEncoder> render = [command renderCommandEncoderWithDescriptor:pass];
        if (!command || !render) return fail(error, error_len, @"render command creation failed");
        [render setRenderPipelineState:r.pipeline];
        [render setVertexBuffer:r.vertices offset:0 atIndex:6];
        if (r.fragmentConstants) {
            [render setFragmentBuffer:r.fragmentArguments offset:0 atIndex:2];
            [render useResource:r.fragmentConstants usage:MTLResourceUsageRead stages:MTLRenderStageFragment];
        }
        if (r.textureEnabled) {
            if (!r.sampledTexture || !r.sampler) return fail(error, error_len, @"fragment texture has not been bound");
            [render setFragmentBuffer:r.textureHeap offset:0 atIndex:0];
            [render setFragmentBuffer:r.samplerHeap offset:0 atIndex:1];
            [render useResource:r.sampledTexture usage:MTLResourceUsageRead stages:MTLRenderStageFragment];
        }
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
