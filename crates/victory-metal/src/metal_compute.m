#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <stdint.h>
#import <stdio.h>
#import <string.h>

static bool compute_fail(char *dst, size_t len, NSString *message) {
    if (len) snprintf(dst, len, "%s", message.UTF8String ?: "Metal compute error");
    return false;
}

@interface VictoryMetalCompute : NSObject
@property(nonatomic, strong) id<MTLDevice> device;
@property(nonatomic, strong) id<MTLCommandQueue> queue;
@property(nonatomic, strong) id<MTLComputePipelineState> pipeline;
@property(nonatomic, strong) id<MTLBuffer> dataBuffer;
@property(nonatomic, strong) id<MTLBuffer> inputBuffer;
@property(nonatomic, strong) id<MTLBuffer> constantBuffer;
@property(nonatomic, strong) id<MTLBuffer> rootCBVBuffer;
@property(nonatomic, strong) id<MTLBuffer> resourceHeap;
@property(nonatomic, strong) NSMutableArray<id<MTLBuffer>> *heapBuffers;
@property(nonatomic, strong) id<MTLBuffer> arguments;
@end
@implementation VictoryMetalCompute
@end

void *victory_metal_compute_create(const uint8_t *bytes, size_t len, const char *entry,
                                   char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
#define FAIL(message) do { compute_fail(error, error_len, (message)); return NULL; } while (0)
        VictoryMetalCompute *c = [VictoryMetalCompute new];
        c.device = MTLCreateSystemDefaultDevice();
        if (!c.device) FAIL(@"no Metal device");
        if (c.device.argumentBuffersSupport != MTLArgumentBuffersTier2)
            FAIL(@"compute resource heap requires argument buffers tier 2");
        NSData *owned = [NSData dataWithBytes:bytes length:len];
        if (!owned) FAIL(@"metallib allocation failed");
        dispatch_data_t data = dispatch_data_create(owned.bytes, owned.length,
                                                    dispatch_get_main_queue(), ^{ (void)owned; });
        if (!data) FAIL(@"metallib dispatch data allocation failed");
        NSError *e = nil;
        id<MTLLibrary> library = [c.device newLibraryWithData:data error:&e];
        if (!library) FAIL(e.localizedDescription ?: @"compute library loading failed");
        NSString *name = [NSString stringWithUTF8String:entry];
        if (!name) FAIL(@"invalid compute entry name");
        id<MTLFunction> function = [library newFunctionWithName:name];
        if (!function) FAIL(@"compute function not found");
        c.pipeline = [c.device newComputePipelineStateWithFunction:function error:&e];
        if (!c.pipeline) FAIL(e.localizedDescription ?: @"compute pipeline creation failed");
        c.queue = [c.device newCommandQueue];
        c.resourceHeap = [c.device newBufferWithLength:16 * 24 options:MTLResourceStorageModeShared];
        c.heapBuffers = [NSMutableArray array];
        c.arguments = [c.device newBufferWithLength:24 options:MTLResourceStorageModeShared];
        if (!c.queue || !c.resourceHeap || !c.arguments || !c.heapBuffers) FAIL(@"compute queue or descriptor allocation failed");
#undef FAIL
        return (__bridge_retained void *)c;
    }
}

static bool compute_dispatch(void *handle, uint8_t *buffer, size_t buffer_len,
                             const uint8_t *input, size_t input_len,
                             size_t srv_offset, size_t srv_len, size_t uav_offset, size_t uav_len,
                             const uint32_t *constants, size_t constant_bytes,
                             const uint8_t *cbv, size_t cbv_len, size_t cbv_offset,
                             const uint32_t *groups, const uint32_t *threads,
                             char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        VictoryMetalCompute *c = (__bridge VictoryMetalCompute *)handle;
        if (!c || !buffer || !constants || !groups || !threads || !buffer_len ||
            buffer_len > UINT32_MAX || constant_bytes < 16 || constant_bytes % 16 ||
            constant_bytes > UINT32_MAX || !groups[0] || !groups[1] || !groups[2] ||
            !threads[0] || !threads[1] || !threads[2])
            return compute_fail(error, error_len, @"invalid compute dispatch arguments");
        if ((cbv && (!cbv_len || cbv_len > 65536 || cbv_offset % 8 ||
                     cbv_offset > SIZE_MAX - 24)) ||
            (!cbv && (cbv_len || cbv_offset)))
            return compute_fail(error, error_len, @"invalid compute root CBV arguments");
        if (input && (!input_len || input_len > UINT32_MAX || !srv_len || !uav_len ||
                      srv_offset > input_len || srv_len > input_len - srv_offset ||
                      uav_offset > buffer_len || uav_len > buffer_len - uav_offset))
            return compute_fail(error, error_len, @"invalid raw compute buffer views");
        size_t argument_len = cbv && cbv_offset + 24 > 24 ? cbv_offset + 24 : 24;
        if (argument_len > c.device.maxBufferLength)
            return compute_fail(error, error_len, @"compute argument buffer exceeds Metal device limit");
        MTLSize maximum = c.device.maxThreadsPerThreadgroup;
        if (threads[0] > maximum.width || threads[1] > maximum.height || threads[2] > maximum.depth ||
            (uint64_t)threads[0] * threads[1] > c.pipeline.maxTotalThreadsPerThreadgroup ||
            (uint64_t)threads[0] * threads[1] * threads[2] > c.pipeline.maxTotalThreadsPerThreadgroup)
            return compute_fail(error, error_len, @"threadgroup dimensions exceed Metal pipeline limits");
        if (c.dataBuffer.length < buffer_len) {
            id<MTLBuffer> replacement = [c.device newBufferWithLength:buffer_len options:MTLResourceStorageModeShared];
            if (!replacement) return compute_fail(error, error_len, @"compute data buffer allocation failed");
            c.dataBuffer = replacement;
        }
        if (input && c.inputBuffer.length < input_len) {
            id<MTLBuffer> replacement = [c.device newBufferWithLength:input_len options:MTLResourceStorageModeShared];
            if (!replacement) return compute_fail(error, error_len, @"compute input buffer allocation failed");
            c.inputBuffer = replacement;
        }
        if (c.constantBuffer.length < constant_bytes) {
            id<MTLBuffer> replacement = [c.device newBufferWithLength:constant_bytes options:MTLResourceStorageModeShared];
            if (!replacement) return compute_fail(error, error_len, @"compute constant buffer allocation failed");
            c.constantBuffer = replacement;
        }
        if (cbv && c.rootCBVBuffer.length < cbv_len) {
            id<MTLBuffer> replacement = [c.device newBufferWithLength:cbv_len options:MTLResourceStorageModeShared];
            if (!replacement) return compute_fail(error, error_len, @"compute root CBV allocation failed");
            c.rootCBVBuffer = replacement;
        }
        if (c.arguments.length < argument_len) {
            id<MTLBuffer> replacement = [c.device newBufferWithLength:argument_len options:MTLResourceStorageModeShared];
            if (!replacement) return compute_fail(error, error_len, @"compute argument buffer allocation failed");
            c.arguments = replacement;
        }
        memcpy(c.dataBuffer.contents, buffer, buffer_len);
        if (input) memcpy(c.inputBuffer.contents, input, input_len);
        memcpy(c.constantBuffer.contents, constants, constant_bytes);
        if (cbv) memcpy(c.rootCBVBuffer.contents, cbv, cbv_len);
        // IRDescriptorTableEntry: GPU address, texture ID, and byte length.
        uint64_t *heap = c.resourceHeap.contents;
        if (input) {
            if (c.inputBuffer.gpuAddress > UINT64_MAX - srv_offset ||
                c.dataBuffer.gpuAddress > UINT64_MAX - uav_offset)
                return compute_fail(error, error_len, @"compute view GPU address overflow");
            heap[0] = c.inputBuffer.gpuAddress + srv_offset; heap[1] = 0; heap[2] = srv_len;
            heap[3] = c.dataBuffer.gpuAddress + uav_offset; heap[4] = 0; heap[5] = uav_len;
        } else {
            heap[0] = c.dataBuffer.gpuAddress; heap[1] = 0; heap[2] = buffer_len;
        }
        memset(c.arguments.contents, 0, argument_len);
        if (!cbv || cbv_offset >= 24) {
            uint64_t *args = c.arguments.contents;
            args[0] = c.constantBuffer.gpuAddress; args[1] = 0; args[2] = constant_bytes;
        }
        if (cbv) {
            uint64_t *entry = (uint64_t *)((uint8_t *)c.arguments.contents + cbv_offset);
            entry[0] = c.rootCBVBuffer.gpuAddress; entry[1] = 0; entry[2] = cbv_len;
        }
        id<MTLCommandBuffer> command = [c.queue commandBuffer];
        if (!command) return compute_fail(error, error_len, @"compute command buffer creation failed");
        id<MTLComputeCommandEncoder> encoder = [command computeCommandEncoder];
        if (!encoder) return compute_fail(error, error_len, @"compute encoder creation failed");
        [encoder setComputePipelineState:c.pipeline];
        [encoder setBuffer:c.resourceHeap offset:0 atIndex:0];
        [encoder setBuffer:c.arguments offset:0 atIndex:2];
        // GPU addresses in descriptors are indirect: explicitly declare residency.
        [encoder useResource:c.dataBuffer usage:MTLResourceUsageRead | MTLResourceUsageWrite];
        if (input) [encoder useResource:c.inputBuffer usage:MTLResourceUsageRead];
        [encoder useResource:c.constantBuffer usage:MTLResourceUsageRead];
        if (cbv) [encoder useResource:c.rootCBVBuffer usage:MTLResourceUsageRead];
        [encoder dispatchThreadgroups:MTLSizeMake(groups[0], groups[1], groups[2])
               threadsPerThreadgroup:MTLSizeMake(threads[0], threads[1], threads[2])];
        [encoder endEncoding];
        [command commit];
        [command waitUntilCompleted];
        if (command.status != MTLCommandBufferStatusCompleted)
            return compute_fail(error, error_len, command.error.localizedDescription ?: @"compute command failed");
        memcpy(buffer, c.dataBuffer.contents, buffer_len);
        return true;
    }
}

bool victory_metal_compute_dispatch(void *handle, uint8_t *buffer, size_t buffer_len,
                                    const uint32_t *constants, size_t constant_bytes,
                                    const uint8_t *cbv, size_t cbv_len, size_t cbv_offset,
                                    const uint32_t *groups, const uint32_t *threads,
                                    char *error, size_t error_len) {
    return compute_dispatch(handle, buffer, buffer_len, NULL, 0, 0, 0, 0, 0,
                            constants, constant_bytes, cbv, cbv_len, cbv_offset,
                            groups, threads, error, error_len);
}

bool victory_metal_compute_dispatch_raw_views(void *handle, uint8_t *output, size_t output_len,
                                              const uint8_t *input, size_t input_len,
                                              size_t srv_offset, size_t srv_len,
                                              size_t uav_offset, size_t uav_len,
                                              const uint32_t *constants, size_t constant_bytes,
                                              const uint32_t *groups, const uint32_t *threads,
                                              char *error, size_t error_len) {
    if (!input) return compute_fail(error, error_len, @"invalid compute input buffer");
    return compute_dispatch(handle, output, output_len, input, input_len,
                            srv_offset, srv_len, uav_offset, uav_len,
                            constants, constant_bytes, NULL, 0, 0,
                            groups, threads, error, error_len);
}

// The layout matches NativeRawHeapBuffer in compute.rs.
typedef struct {
    uint8_t *bytes;
    size_t size;
    size_t offset;
    size_t length;
    uint32_t writable;
} VictoryRawHeapBuffer;

bool victory_metal_compute_dispatch_raw_heap(void *handle,
                                             const VictoryRawHeapBuffer *resources,
                                             size_t resource_count,
                                             const uint32_t *constants, size_t constant_bytes,
                                             const uint32_t *groups, const uint32_t *threads,
                                             char *error, size_t error_len) {
    @autoreleasepool {
        if (error_len) error[0] = 0;
        VictoryMetalCompute *c = (__bridge VictoryMetalCompute *)handle;
        if (!c || !resources || resource_count > 16 || !constants || !groups || !threads ||
            constant_bytes > UINT32_MAX || constant_bytes % sizeof(uint32_t) ||
            !groups[0] || !groups[1] || !groups[2] ||
            !threads[0] || !threads[1] || !threads[2])
            return compute_fail(error, error_len, @"invalid raw heap dispatch arguments");
        MTLSize maximum = c.device.maxThreadsPerThreadgroup;
        if (threads[0] > maximum.width || threads[1] > maximum.height || threads[2] > maximum.depth ||
            (uint64_t)threads[0] * threads[1] > c.pipeline.maxTotalThreadsPerThreadgroup ||
            (uint64_t)threads[0] * threads[1] * threads[2] > c.pipeline.maxTotalThreadsPerThreadgroup)
            return compute_fail(error, error_len, @"threadgroup dimensions exceed Metal pipeline limits");
        for (size_t i = 0; i < resource_count; i++) {
            const VictoryRawHeapBuffer *r = &resources[i];
            if (!r->bytes || !r->size || r->size > UINT32_MAX || !r->length ||
                r->offset > r->size || r->length > r->size - r->offset || r->writable > 1)
                return compute_fail(error, error_len, @"invalid raw heap buffer view");
        }
        size_t padded_constants = (constant_bytes + 15) & ~(size_t)15;
        if (padded_constants < 16) padded_constants = 16;
        if (padded_constants > c.device.maxBufferLength)
            return compute_fail(error, error_len, @"compute constants exceed Metal device limit");
        if (c.constantBuffer.length < padded_constants) {
            id<MTLBuffer> replacement = [c.device newBufferWithLength:padded_constants options:MTLResourceStorageModeShared];
            if (!replacement) return compute_fail(error, error_len, @"compute constant buffer allocation failed");
            c.constantBuffer = replacement;
        }
        memset(c.constantBuffer.contents, 0, padded_constants);
        if (constant_bytes) memcpy(c.constantBuffer.contents, constants, constant_bytes);
        uint64_t *heap = c.resourceHeap.contents;
        memset(heap, 0, 16 * 24);
        for (size_t i = 0; i < resource_count; i++) {
            const VictoryRawHeapBuffer *r = &resources[i];
            id<MTLBuffer> buffer = i < c.heapBuffers.count ? c.heapBuffers[i] : nil;
            if (buffer.length < r->size) {
                buffer = [c.device newBufferWithLength:r->size options:MTLResourceStorageModeShared];
                if (!buffer) return compute_fail(error, error_len, @"compute heap buffer allocation failed");
                if (i < c.heapBuffers.count) c.heapBuffers[i] = buffer;
                else [c.heapBuffers addObject:buffer];
            }
            memcpy(buffer.contents, r->bytes, r->size);
            if (buffer.gpuAddress > UINT64_MAX - r->offset)
                return compute_fail(error, error_len, @"compute view GPU address overflow");
            // IRDescriptorTableEntry is three 64-bit words: address, texture ID, length.
            heap[3 * i] = buffer.gpuAddress + r->offset;
            heap[3 * i + 1] = 0;
            heap[3 * i + 2] = r->length;
        }
        uint64_t *args = c.arguments.contents;
        args[0] = c.constantBuffer.gpuAddress;
        args[1] = 0;
        args[2] = padded_constants;
        id<MTLCommandBuffer> command = [c.queue commandBuffer];
        if (!command) return compute_fail(error, error_len, @"compute command buffer creation failed");
        id<MTLComputeCommandEncoder> encoder = [command computeCommandEncoder];
        if (!encoder) return compute_fail(error, error_len, @"compute encoder creation failed");
        [encoder setComputePipelineState:c.pipeline];
        [encoder setBuffer:c.resourceHeap offset:0 atIndex:0];
        [encoder setBuffer:c.arguments offset:0 atIndex:2];
        for (size_t i = 0; i < resource_count; i++) {
            MTLResourceUsage usage = MTLResourceUsageRead;
            if (resources[i].writable) usage |= MTLResourceUsageWrite;
            [encoder useResource:c.heapBuffers[i] usage:usage];
        }
        [encoder useResource:c.constantBuffer usage:MTLResourceUsageRead];
        [encoder dispatchThreadgroups:MTLSizeMake(groups[0], groups[1], groups[2])
               threadsPerThreadgroup:MTLSizeMake(threads[0], threads[1], threads[2])];
        [encoder endEncoding];
        [command commit];
        [command waitUntilCompleted];
        if (command.status != MTLCommandBufferStatusCompleted)
            return compute_fail(error, error_len, command.error.localizedDescription ?: @"compute command failed");
        for (size_t i = 0; i < resource_count; i++) {
            if (resources[i].writable)
                memcpy(resources[i].bytes, c.heapBuffers[i].contents, resources[i].size);
        }
        return true;
    }
}

void victory_metal_compute_destroy(void *handle) {
    @autoreleasepool {
        if (handle) { (void)(__bridge_transfer VictoryMetalCompute *)handle; }
    }
}
