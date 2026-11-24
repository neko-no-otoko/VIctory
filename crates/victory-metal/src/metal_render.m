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
                         bool bgra, char *error, size_t error_len) {
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
        MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:bgra ? MTLPixelFormatBGRA8Unorm : MTLPixelFormatRGBA8Unorm
                                                                                       width:width height:height mipmapped:NO];
        desc.storageMode = MTLStorageModeShared;
        desc.usage = MTLTextureUsageShaderWrite;
        id<MTLTexture> texture = [device newTextureWithDescriptor:desc];
        if (!texture) return fail(error, error_len, @"clear texture allocation failed");
        size_t row = (size_t)width * 4;
        for (uint32_t y = 0; y < height; ++y) {
            MTLRegion region = MTLRegionMake2D(0, y, width, 1);
            [texture replaceRegion:region mipmapLevel:0 withBytes:pixels + (size_t)y * pitch bytesPerRow:row];
        }
        id<MTLCommandBuffer> command = [queue commandBuffer];
        id<MTLComputeCommandEncoder> encoder = [command computeCommandEncoder];
        if (!command || !encoder) return fail(error, error_len, @"clear command creation failed");
        [encoder setComputePipelineState:pipeline];
        [encoder setTexture:texture atIndex:0];
        [encoder setBytes:color length:4 atIndex:1];
        const uint32_t full[4] = {0, 0, width, height};
        size_t n = count ? count : 1;
        for (size_t i = 0; i < n; ++i) {
            const uint32_t *input = count ? rects[i] : full;
            const uint32_t rect[4] = { MIN(input[0], width), MIN(input[1], height),
                                       MIN(input[2], width), MIN(input[3], height) };
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
        for (uint32_t y = 0; y < height; ++y) {
            MTLRegion region = MTLRegionMake2D(0, y, width, 1);
            [texture getBytes:pixels + (size_t)y * pitch bytesPerRow:row fromRegion:region mipmapLevel:0];
        }
        return true;
    }
}


@interface VictoryMetalRenderer : NSObject
@property(nonatomic, strong) id<MTLDevice> device;
@property(nonatomic, strong) id<MTLCommandQueue> queue;
@property(nonatomic, strong) id<MTLRenderPipelineState> pipeline;
@property(nonatomic, strong) id<MTLTexture> texture;
@property(nonatomic, strong) id<MTLTexture> depthTexture;
@property(nonatomic, strong) id<MTLDepthStencilState> depthState;
@property(nonatomic, strong) id<MTLBuffer> readback;
@property(nonatomic, strong) id<MTLBuffer> vertices;
@property(nonatomic, strong) id<MTLBuffer> indices;
@property(nonatomic, strong) id<MTLBuffer> fragmentConstants;
@property(nonatomic, strong) id<MTLBuffer> fragmentArguments;
@property(nonatomic, strong) id<MTLTexture> sampledTexture;
@property(nonatomic, strong) id<MTLSamplerState> sampler;
@property(nonatomic, strong) id<MTLBuffer> textureHeap;
@property(nonatomic, strong) id<MTLBuffer> samplerHeap;
@property(nonatomic) BOOL textureEnabled;
@property(nonatomic) BOOL linearFilter;
@property(nonatomic) float minLod;
@property(nonatomic) float maxLod;
@property(nonatomic) size_t fragmentConstantBytes;
@property(nonatomic) size_t vertexStride;
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
                           bool texture_enabled, uint32_t depth_compare, bool depth_write,
                           bool blend_enabled, uint8_t write_mask, bool alpha_source,
                           bool vertex_uv, bool bgra, char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
#define FAIL(message) do { fail(error, error_len, (message)); return NULL; } while (0)
        if (write_mask & ~0x0f) FAIL(@"invalid color write mask");
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
        if (vertex_uv) {
            layout.attributes[12].format = MTLVertexFormatFloat2;
            layout.attributes[12].offset = 4 * sizeof(float);
            layout.attributes[12].bufferIndex = 6;
        }
        r.vertexStride = (vertex_uv ? 6 : 4) * sizeof(float);
        layout.layouts[6].stride = r.vertexStride;
        layout.layouts[6].stepFunction = MTLVertexStepFunctionPerVertex;
        MTLRenderPipelineDescriptor *desc = [MTLRenderPipelineDescriptor new];
        desc.vertexFunction = vertex;
        desc.fragmentFunction = fragment;
        desc.vertexDescriptor = layout;
        MTLPixelFormat colorFormat = bgra ? MTLPixelFormatBGRA8Unorm : MTLPixelFormatRGBA8Unorm;
        desc.colorAttachments[0].pixelFormat = colorFormat;
        MTLRenderPipelineColorAttachmentDescriptor *color = desc.colorAttachments[0];
        color.writeMask = ((write_mask & 1) ? MTLColorWriteMaskRed : 0)
                        | ((write_mask & 2) ? MTLColorWriteMaskGreen : 0)
                        | ((write_mask & 4) ? MTLColorWriteMaskBlue : 0)
                        | ((write_mask & 8) ? MTLColorWriteMaskAlpha : 0);
        color.blendingEnabled = blend_enabled;
        color.rgbBlendOperation = MTLBlendOperationAdd;
        color.alphaBlendOperation = MTLBlendOperationAdd;
        color.sourceRGBBlendFactor = MTLBlendFactorSourceAlpha;
        color.destinationRGBBlendFactor = MTLBlendFactorOneMinusSourceAlpha;
        color.sourceAlphaBlendFactor = alpha_source ? MTLBlendFactorSourceAlpha : MTLBlendFactorOne;
        color.destinationAlphaBlendFactor = MTLBlendFactorOneMinusSourceAlpha;
        if (depth_compare) desc.depthAttachmentPixelFormat = MTLPixelFormatDepth32Float;
        r.pipeline = [r.device newRenderPipelineStateWithDescriptor:desc error:&e];
        if (!r.pipeline) FAIL(e.localizedDescription ?: @"pipeline creation failed");
        MTLTextureDescriptor *texture = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:colorFormat width:width height:height mipmapped:NO];
        texture.storageMode = MTLStorageModePrivate;
        texture.usage = MTLTextureUsageRenderTarget;
        r.texture = [r.device newTextureWithDescriptor:texture];
        if (!r.texture) FAIL(@"texture allocation failed");
        if (depth_compare) {
            if (depth_compare > 8) FAIL(@"invalid depth comparison");
            MTLDepthStencilDescriptor *depth = [MTLDepthStencilDescriptor new];
            depth.depthCompareFunction = (MTLCompareFunction)(depth_compare - 1);
            depth.depthWriteEnabled = depth_write;
            r.depthState = [r.device newDepthStencilStateWithDescriptor:depth];
            MTLTextureDescriptor *depthTexture = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatDepth32Float width:width height:height mipmapped:NO];
            depthTexture.storageMode = MTLStorageModePrivate;
            depthTexture.usage = MTLTextureUsageRenderTarget;
            r.depthTexture = [r.device newTextureWithDescriptor:depthTexture];
            if (!r.depthState || !r.depthTexture) FAIL(@"depth attachment allocation failed");
        }
        size_t row = (size_t)width * 4;
        r.stride = (row + 255) & ~(size_t)255;
        if (r.stride > SIZE_MAX / height) FAIL(@"readback dimensions overflow");
        r.readback = [r.device newBufferWithLength:r.stride * height options:MTLResourceStorageModeShared];
        r.queue = [r.device newCommandQueue];
        if (!r.readback || !r.queue) FAIL(@"readback or queue allocation failed");
        if (r.depthTexture) {
            id<MTLCommandBuffer> command = [r.queue commandBuffer];
            MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
            pass.depthAttachment.texture = r.depthTexture;
            pass.depthAttachment.loadAction = MTLLoadActionClear;
            pass.depthAttachment.clearDepth = 1.0;
            pass.depthAttachment.storeAction = MTLStoreActionStore;
            id<MTLRenderCommandEncoder> encoder = [command renderCommandEncoderWithDescriptor:pass];
            if (!command || !encoder) FAIL(@"depth initialization command failed");
            [encoder endEncoding];
            [command commit];
            [command waitUntilCompleted];
            if (command.status != MTLCommandBufferStatusCompleted)
                FAIL(command.error.localizedDescription ?: @"depth initialization failed");
        }
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

static bool depthTransfer(id<MTLCommandQueue> queue, id<MTLTexture> texture, id<MTLBuffer> buffer,
                          size_t stride, uint8_t *pixels, size_t pitch, bool upload,
                          char *error, size_t error_len) {
    size_t row = texture.width * sizeof(float);
    if (upload) for (NSUInteger y = 0; y < texture.height; ++y)
        memcpy((uint8_t *)buffer.contents + y * stride, pixels + y * pitch, row);
    id<MTLCommandBuffer> command = [queue commandBuffer];
    id<MTLBlitCommandEncoder> blit = [command blitCommandEncoder];
    if (!command || !blit) return fail(error, error_len, @"depth transfer command failed");
    MTLSize size = MTLSizeMake(texture.width, texture.height, 1);
    if (upload)
        [blit copyFromBuffer:buffer sourceOffset:0 sourceBytesPerRow:stride
         sourceBytesPerImage:stride * texture.height sourceSize:size toTexture:texture
         destinationSlice:0 destinationLevel:0 destinationOrigin:MTLOriginMake(0, 0, 0)];
    else
        [blit copyFromTexture:texture sourceSlice:0 sourceLevel:0 sourceOrigin:MTLOriginMake(0, 0, 0)
         sourceSize:size toBuffer:buffer destinationOffset:0 destinationBytesPerRow:stride
         destinationBytesPerImage:stride * texture.height];
    [blit endEncoding];
    [command commit];
    [command waitUntilCompleted];
    if (command.status != MTLCommandBufferStatusCompleted)
        return fail(error, error_len, command.error.localizedDescription ?: @"depth transfer failed");
    if (!upload) for (NSUInteger y = 0; y < texture.height; ++y)
        memcpy(pixels + y * pitch, (const uint8_t *)buffer.contents + y * stride, row);
    return true;
}

bool victory_metal_depth_pixels(void *handle, float *pixels, bool upload, char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        VictoryMetalRenderer *r = (__bridge VictoryMetalRenderer *)handle;
        if (!r.depthTexture) return fail(error, error_len, @"depth attachment is not enabled");
        return depthTransfer(r.queue, r.depthTexture, r.readback, r.stride, (uint8_t *)pixels,
                             (size_t)r.width * sizeof(float), upload, error, error_len);
    }
}

bool victory_metal_clear_depth(uint32_t width, uint32_t height, size_t pitch, uint8_t *pixels,
                               float value, const uint32_t (*rects)[4], size_t count,
                               char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        static id<MTLDevice> device;
        static id<MTLCommandQueue> queue;
        static id<MTLRenderPipelineState> pipeline;
        static id<MTLDepthStencilState> state;
        static NSString *setupError;
        static dispatch_once_t once;
        dispatch_once(&once, ^{
            device = MTLCreateSystemDefaultDevice();
            if (!device) { setupError = @"no Metal device"; return; }
            NSError *e = nil;
            NSString *source = @"#include <metal_stdlib>\nusing namespace metal;\n"
                "vertex float4 depth_vertex(uint i [[vertex_id]], constant float &z [[buffer(0)]]) {"
                " float2 p[3] = {float2(-1,-1), float2(3,-1), float2(-1,3)};"
                " return float4(p[i], z, 1); }";
            id<MTLLibrary> library = [device newLibraryWithSource:source options:nil error:&e];
            MTLRenderPipelineDescriptor *desc = [MTLRenderPipelineDescriptor new];
            desc.vertexFunction = [library newFunctionWithName:@"depth_vertex"];
            desc.depthAttachmentPixelFormat = MTLPixelFormatDepth32Float;
            if (desc.vertexFunction) pipeline = [device newRenderPipelineStateWithDescriptor:desc error:&e];
            MTLDepthStencilDescriptor *depth = [MTLDepthStencilDescriptor new];
            depth.depthCompareFunction = MTLCompareFunctionAlways;
            depth.depthWriteEnabled = YES;
            state = [device newDepthStencilStateWithDescriptor:depth];
            queue = [device newCommandQueue];
            if (!pipeline || !state || !queue) setupError = e.localizedDescription ?: @"depth clear pipeline creation failed";
        });
        if (setupError) return fail(error, error_len, setupError);
        size_t row = (size_t)width * 4;
        if (row > SIZE_MAX - 255) return fail(error, error_len, @"depth dimensions overflow");
        size_t stride = (row + 255) & ~(size_t)255;
        if (stride > SIZE_MAX / height) return fail(error, error_len, @"depth dimensions overflow");
        MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatDepth32Float width:width height:height mipmapped:NO];
        desc.storageMode = MTLStorageModePrivate;
        desc.usage = MTLTextureUsageRenderTarget;
        id<MTLTexture> texture = [device newTextureWithDescriptor:desc];
        id<MTLBuffer> buffer = [device newBufferWithLength:stride * height options:MTLResourceStorageModeShared];
        if (!texture || !buffer) return fail(error, error_len, @"depth clear allocation failed");
        if (!depthTransfer(queue, texture, buffer, stride, pixels, pitch, true, error, error_len)) return false;
        id<MTLCommandBuffer> command = [queue commandBuffer];
        MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
        pass.depthAttachment.texture = texture;
        pass.depthAttachment.loadAction = MTLLoadActionLoad;
        pass.depthAttachment.storeAction = MTLStoreActionStore;
        id<MTLRenderCommandEncoder> encoder = [command renderCommandEncoderWithDescriptor:pass];
        if (!command || !encoder) return fail(error, error_len, @"depth clear command failed");
        [encoder setRenderPipelineState:pipeline];
        [encoder setDepthStencilState:state];
        [encoder setVertexBytes:&value length:sizeof(value) atIndex:0];
        const uint32_t full[4] = {0, 0, width, height};
        for (size_t i = 0; i < (count ? count : 1); ++i) {
            const uint32_t *r = count ? rects[i] : full;
            if (r[0] == r[2] || r[1] == r[3]) continue;
            [encoder setScissorRect:(MTLScissorRect){r[0], r[1], r[2] - r[0], r[3] - r[1]}];
            [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
        }
        [encoder endEncoding];
        [command commit];
        [command waitUntilCompleted];
        if (command.status != MTLCommandBufferStatusCompleted)
            return fail(error, error_len, command.error.localizedDescription ?: @"depth clear failed");
        return depthTransfer(queue, texture, buffer, stride, pixels, pitch, false, error, error_len);
    }
}

bool victory_metal_set_texture(void *handle, uint32_t width, uint32_t height, uint32_t levels,
                               const uint8_t *rgba, bool linear, uint32_t texture_format, float min_lod, float max_lod,
                               char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        VictoryMetalRenderer *r = (__bridge VictoryMetalRenderer *)handle;
        if (!r.textureEnabled) return fail(error, error_len, @"texture binding is not enabled");
        id<MTLTexture> texture = r.sampledTexture;
        MTLPixelFormat format;
        size_t blockBytes;
        switch (texture_format) {
            case 0: format = MTLPixelFormatRGBA8Unorm; blockBytes = 0; break;
            case 1: format = MTLPixelFormatBGRA8Unorm; blockBytes = 0; break;
            case 2: format = MTLPixelFormatBC1_RGBA; blockBytes = 8; break;
            case 3: format = MTLPixelFormatBC3_RGBA; blockBytes = 16; break;
            default: return fail(error, error_len, @"invalid sampled texture format");
        }
        if (blockBytes && ![r.device supportsBCTextureCompression])
            return fail(error, error_len, @"BC texture compression is not supported by the Metal device");
        if (!texture || texture.width != width || texture.height != height ||
            texture.pixelFormat != format || texture.mipmapLevelCount != levels) {
            MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:format width:width height:height mipmapped:YES];
            desc.mipmapLevelCount = levels;
            desc.textureType = MTLTextureType2DArray;
            desc.arrayLength = 1;
            desc.storageMode = MTLStorageModeShared;
            desc.usage = MTLTextureUsageShaderRead;
            texture = [r.device newTextureWithDescriptor:desc];
            if (!texture) return fail(error, error_len, @"sampled texture allocation failed");
        }
        id<MTLSamplerState> sampler = r.sampler;
        if (!sampler || r.linearFilter != linear || r.minLod != min_lod || r.maxLod != max_lod) {
            MTLSamplerDescriptor *desc = [MTLSamplerDescriptor new];
            desc.minFilter = linear ? MTLSamplerMinMagFilterLinear : MTLSamplerMinMagFilterNearest;
            desc.magFilter = desc.minFilter;
            desc.mipFilter = linear ? MTLSamplerMipFilterLinear : MTLSamplerMipFilterNearest;
            desc.lodMinClamp = min_lod;
            desc.lodMaxClamp = max_lod;
            desc.sAddressMode = MTLSamplerAddressModeClampToEdge;
            desc.tAddressMode = MTLSamplerAddressModeClampToEdge;
            desc.supportArgumentBuffers = YES;
            sampler = [r.device newSamplerStateWithDescriptor:desc];
            if (!sampler) return fail(error, error_len, @"sampler allocation failed");
        }
        uint32_t w = width, h = height;
        const uint8_t *levelPixels = rgba;
        for (uint32_t level = 0; level < levels; ++level) {
            size_t rowBytes = blockBytes ? (((size_t)w + 3) / 4) * blockBytes : (size_t)w * 4;
            size_t rows = blockBytes ? ((size_t)h + 3) / 4 : h;
            size_t imageBytes = rowBytes * rows;
            [texture replaceRegion:MTLRegionMake2D(0, 0, w, h) mipmapLevel:level slice:0
                         withBytes:levelPixels bytesPerRow:rowBytes bytesPerImage:imageBytes];
            levelPixels += imageBytes;
            w = w > 1 ? w / 2 : 1;
            h = h > 1 ? h / 2 : 1;
        }
        // IRDescriptorTableEntry: texture uses textureViewID, sampler uses
        // gpuVA; metadata is a zero min-LOD clamp or zero sampler LOD bias.
        uint64_t *tex = r.textureHeap.contents;
        tex[0] = 0; tex[1] = texture.gpuResourceID._impl; tex[2] = 0;
        uint64_t *smp = r.samplerHeap.contents;
        smp[0] = sampler.gpuResourceID._impl; smp[1] = 0; smp[2] = 0;
        r.sampledTexture = texture;
        r.sampler = sampler;
        r.linearFilter = linear;
        r.minLod = min_lod;
        r.maxLod = max_lod;
        return true;
    }
}

static bool draw(void *handle, const float *vertices, uint32_t count,
                 const void *indices, uint32_t index_count, uint32_t index_size,
                 int32_t base_vertex, const uint32_t *constants, uint8_t *pixels,
                 bool preserve, const double *viewport, const uint32_t *scissor,
                 char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        VictoryMetalRenderer *r = (__bridge VictoryMetalRenderer *)handle;
        if ((size_t)count > SIZE_MAX / r.vertexStride)
            return fail(error, error_len, @"vertex buffer size overflow");
        size_t size = (size_t)count * r.vertexStride;
        if (!r.vertices || r.vertices.length < size)
            r.vertices = [r.device newBufferWithLength:size options:MTLResourceStorageModeShared];
        if (!r.vertices) return fail(error, error_len, @"vertex buffer allocation failed");
        memcpy(r.vertices.contents, vertices, size);
        if (indices) {
            if ((index_size != 2 && index_size != 4) || index_count > SIZE_MAX / index_size)
                return fail(error, error_len, @"invalid index buffer size");
            size_t indexBytes = (size_t)index_count * index_size;
            if (!r.indices || r.indices.length < indexBytes)
                r.indices = [r.device newBufferWithLength:indexBytes options:MTLResourceStorageModeShared];
            if (!r.indices) return fail(error, error_len, @"index buffer allocation failed");
            memcpy(r.indices.contents, indices, indexBytes);
        }
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
        if (r.depthTexture) {
            pass.depthAttachment.texture = r.depthTexture;
            pass.depthAttachment.loadAction = MTLLoadActionLoad;
            pass.depthAttachment.storeAction = MTLStoreActionStore;
        }
        id<MTLRenderCommandEncoder> render = [command renderCommandEncoderWithDescriptor:pass];
        if (!command || !render) return fail(error, error_len, @"render command creation failed");
        bool rasterize = scissor[2] != 0 && scissor[3] != 0;
        if (rasterize) {
            MTLViewport vp = { viewport[0], viewport[1], viewport[2], viewport[3],
                               viewport[4], viewport[5] };
            MTLScissorRect rect = { scissor[0], scissor[1], scissor[2], scissor[3] };
            [render setViewport:vp];
            [render setScissorRect:rect];
        }
        [render setRenderPipelineState:r.pipeline];
        [render setVertexBuffer:r.vertices offset:0 atIndex:6];
        if (r.depthState) [render setDepthStencilState:r.depthState];
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
        if (rasterize && indices) {
            [render drawIndexedPrimitives:MTLPrimitiveTypeTriangle
                               indexCount:index_count
                                indexType:index_size == 2 ? MTLIndexTypeUInt16 : MTLIndexTypeUInt32
                              indexBuffer:r.indices
                        indexBufferOffset:0
                            instanceCount:1
                               baseVertex:(NSInteger)base_vertex
                             baseInstance:0];
        } else if (rasterize) {
            [render drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:count];
        }
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

bool victory_metal_draw(void *handle, const float *vertices, uint32_t count,
                        const uint32_t *constants, uint8_t *pixels, bool preserve,
                        const double *viewport, const uint32_t *scissor,
                        char *error, size_t error_len) {
    return draw(handle, vertices, count, NULL, 0, 0, 0, constants, pixels,
                preserve, viewport, scissor, error, error_len);
}

bool victory_metal_draw_indexed(void *handle, const float *vertices, uint32_t vertex_count,
                                const void *indices, uint32_t index_count, uint32_t index_size,
                                int32_t base_vertex, const uint32_t *constants, uint8_t *pixels,
                                const double *viewport, const uint32_t *scissor,
                                char *error, size_t error_len) {
    return draw(handle, vertices, vertex_count, indices, index_count, index_size, base_vertex,
                constants, pixels, true, viewport, scissor, error, error_len);
}

void victory_metal_destroy(void *handle) {
    @autoreleasepool {
        VictoryMetalRenderer *r = (__bridge_transfer VictoryMetalRenderer *)handle;
        (void)r;
    }
}
