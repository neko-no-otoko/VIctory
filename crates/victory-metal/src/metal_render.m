#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <stdint.h>
#import <stdio.h>
#import <string.h>

static bool fail(char *dst, size_t len, NSString *message) {
    if (len) snprintf(dst, len, "%s", message.UTF8String ?: "Metal error");
    return false;
}

// Synchronous procedural triangle render into caller-owned tightly packed
// RGBA8 bytes. No console bindings or graphics state are emulated here.
bool victory_metal_render(const char *source, const char *vertex_name,
                          const uint8_t *fragment, size_t fragment_len,
                          const char *fragment_name, uint32_t width, uint32_t height,
                          uint8_t *pixels, char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        id<MTLDevice> device = MTLCreateSystemDefaultDevice();
        if (!device) return fail(error, error_len, @"no Metal device");
        NSError *e = nil;
        NSString *vertexSource = [NSString stringWithUTF8String:source];
        id<MTLLibrary> vertexLibrary = [device newLibraryWithSource:vertexSource options:nil error:&e];
        if (!vertexLibrary) return fail(error, error_len, e.localizedDescription ?: @"vertex compilation failed");
        id<MTLFunction> vertex = [vertexLibrary newFunctionWithName:[NSString stringWithUTF8String:vertex_name]];
        if (!vertex) return fail(error, error_len, @"vertex function not found");
        NSData *bytes = [NSData dataWithBytesNoCopy:(void *)fragment length:fragment_len freeWhenDone:NO];
        dispatch_data_t data = dispatch_data_create(bytes.bytes, bytes.length, dispatch_get_main_queue(), ^{ (void)bytes; });
        id<MTLLibrary> fragmentLibrary = [device newLibraryWithData:data error:&e];
        if (!fragmentLibrary) return fail(error, error_len, e.localizedDescription ?: @"fragment library loading failed");
        id<MTLFunction> pixel = [fragmentLibrary newFunctionWithName:[NSString stringWithUTF8String:fragment_name]];
        if (!pixel) return fail(error, error_len, @"fragment function not found");
        MTLRenderPipelineDescriptor *desc = [MTLRenderPipelineDescriptor new];
        desc.vertexFunction = vertex;
        desc.fragmentFunction = pixel;
        desc.colorAttachments[0].pixelFormat = MTLPixelFormatRGBA8Unorm;
        id<MTLRenderPipelineState> pipeline = [device newRenderPipelineStateWithDescriptor:desc error:&e];
        if (!pipeline) return fail(error, error_len, e.localizedDescription ?: @"pipeline creation failed");
        MTLTextureDescriptor *textureDesc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm width:width height:height mipmapped:NO];
        textureDesc.storageMode = MTLStorageModePrivate;
        textureDesc.usage = MTLTextureUsageRenderTarget;
        id<MTLTexture> texture = [device newTextureWithDescriptor:textureDesc];
        if (!texture) return fail(error, error_len, @"texture allocation failed");
        size_t rowBytes = (size_t)width * 4;
        size_t stride = (rowBytes + 255) & ~(size_t)255;
        if (stride > SIZE_MAX / height) return fail(error, error_len, @"readback dimensions overflow");
        id<MTLBuffer> readback = [device newBufferWithLength:stride * height options:MTLResourceStorageModeShared];
        if (!readback) return fail(error, error_len, @"readback allocation failed");
        MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
        pass.colorAttachments[0].texture = texture;
        pass.colorAttachments[0].loadAction = MTLLoadActionClear;
        pass.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 0);
        pass.colorAttachments[0].storeAction = MTLStoreActionStore;
        id<MTLCommandQueue> queue = [device newCommandQueue];
        id<MTLCommandBuffer> command = [queue commandBuffer];
        id<MTLRenderCommandEncoder> render = [command renderCommandEncoderWithDescriptor:pass];
        if (!queue || !command || !render) return fail(error, error_len, @"render command creation failed");
        [render setRenderPipelineState:pipeline];
        [render drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
        [render endEncoding];
        id<MTLBlitCommandEncoder> blit = [command blitCommandEncoder];
        if (!blit) return fail(error, error_len, @"blit command creation failed");
        [blit copyFromTexture:texture sourceSlice:0 sourceLevel:0 sourceOrigin:MTLOriginMake(0, 0, 0)
                   sourceSize:MTLSizeMake(width, height, 1) toBuffer:readback destinationOffset:0
              destinationBytesPerRow:stride destinationBytesPerImage:stride * height];
        [blit endEncoding];
        [command commit];
        [command waitUntilCompleted];
        if (command.status != MTLCommandBufferStatusCompleted)
            return fail(error, error_len, command.error.localizedDescription ?: @"GPU command failed");
        for (uint32_t y = 0; y < height; ++y)
            memcpy(pixels + (size_t)y * rowBytes, (const uint8_t *)readback.contents + (size_t)y * stride, rowBytes);
        return true;
    }
}
