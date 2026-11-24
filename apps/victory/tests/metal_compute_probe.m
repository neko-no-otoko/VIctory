// Test-only probe for the synthetic shader in metal_compute.rs. Binding layout
// is checked against the converter's public reflection API before GPU use.
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <dlfcn.h>
#import <stdint.h>

struct ResourceLocation {
    int type;
    uint32_t space, slot, offset;
    uint64_t size;
    const char *name;
};

static BOOL checkReflection(const char *path, NSData *dxil) {
    void *lib = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    if (!lib) { fprintf(stderr, "dlopen: %s\n", dlerror()); return NO; }
#define LOAD(name) __typeof__(&name) p_##name = (__typeof__(&name))dlsym(lib, #name); \
    if (!p_##name) { fprintf(stderr, "missing converter symbol %s\n", #name); dlclose(lib); return NO; }
    // Public signatures from Apple's metal_irconverter.h; the library is
    // supplied by the test user, not linked or redistributed by the project.
    extern void *IRCompilerCreate(void);
    extern void IRCompilerDestroy(void *);
    extern void *IRObjectCreateFromDXIL(const uint8_t *, size_t, int);
    extern void IRObjectDestroy(void *);
    extern void *IRCompilerAllocCompileAndLink(void *, const char *, const void *, void **);
    extern void IRErrorDestroy(void *);
    extern void *IRShaderReflectionCreate(void);
    extern void IRShaderReflectionDestroy(void *);
    extern bool IRObjectGetReflection(const void *, int, void *);
    extern size_t IRShaderReflectionGetResourceCount(const void *);
    extern void IRShaderReflectionGetResourceLocations(const void *, struct ResourceLocation *);
    LOAD(IRCompilerCreate); LOAD(IRCompilerDestroy); LOAD(IRObjectCreateFromDXIL);
    LOAD(IRObjectDestroy); LOAD(IRCompilerAllocCompileAndLink); LOAD(IRErrorDestroy);
    LOAD(IRShaderReflectionCreate); LOAD(IRShaderReflectionDestroy); LOAD(IRObjectGetReflection);
    LOAD(IRShaderReflectionGetResourceCount); LOAD(IRShaderReflectionGetResourceLocations);
#undef LOAD
    void *compiler = p_IRCompilerCreate();
    void *input = p_IRObjectCreateFromDXIL(dxil.bytes, dxil.length, 0);
    void *error = NULL;
    void *result = compiler && input ? p_IRCompilerAllocCompileAndLink(compiler, NULL, input, &error) : NULL;
    void *reflection = p_IRShaderReflectionCreate();
    BOOL ok = NO;
    if (result && reflection && p_IRObjectGetReflection(result, 8, reflection) &&
        p_IRShaderReflectionGetResourceCount(reflection) == 1) {
        struct ResourceLocation resource = {0};
        p_IRShaderReflectionGetResourceLocations(reflection, &resource);
        // IRResourceTypeCBV = 2, b0 space0, 24-byte linear descriptor at 0.
        ok = resource.type == 2 && resource.space == 0 && resource.slot == 0 &&
             resource.offset == 0 && resource.size == 24;
    }
    if (!ok) fprintf(stderr, "unexpected converter reflection; refusing unverified bindings\n");
    if (reflection) p_IRShaderReflectionDestroy(reflection);
    if (result) p_IRObjectDestroy(result);
    if (error) p_IRErrorDestroy(error);
    if (input) p_IRObjectDestroy(input);
    if (compiler) p_IRCompilerDestroy(compiler);
    dlclose(lib);
    return ok;
}

int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc != 4) return 1;
        NSData *dxil = [NSData dataWithContentsOfFile:[NSString stringWithUTF8String:argv[1]]];
        if (!dxil || !checkReflection(argv[3], dxil)) return 2;
        id<MTLDevice> device = MTLCreateSystemDefaultDevice();
        if (!device || device.argumentBuffersSupport != MTLArgumentBuffersTier2) {
            fprintf(stderr, "Metal device with argument buffers tier 2 required\n"); return 3;
        }
        NSData *bytes = [NSData dataWithContentsOfFile:[NSString stringWithUTF8String:argv[2]]];
        if (!bytes) return 4;
        dispatch_data_t data = dispatch_data_create(bytes.bytes, bytes.length, dispatch_get_main_queue(), ^{ (void)bytes; });
        NSError *error = nil;
        id<MTLLibrary> lib = [device newLibraryWithData:data error:&error];
        id<MTLFunction> function = [lib newFunctionWithName:@"main"];
        if (!function) { fprintf(stderr, "Metal library/function: %s\n", error.description.UTF8String); return 5; }
        id<MTLComputePipelineState> pipeline = [device newComputePipelineStateWithFunction:function error:&error];
        if (!pipeline) { fprintf(stderr, "Metal pipeline: %s\n", error.description.UTF8String); return 6; }
        id<MTLBuffer> output = [device newBufferWithLength:64 options:MTLResourceStorageModeShared];
        id<MTLBuffer> constants = [device newBufferWithLength:256 options:MTLResourceStorageModeShared];
        id<MTLBuffer> heap = [device newBufferWithLength:24 options:MTLResourceStorageModeShared];
        id<MTLBuffer> top = [device newBufferWithLength:24 options:MTLResourceStorageModeShared];
        if (!output || !constants || !heap || !top) return 7;
        memset(output.contents, 0xA5, 64);
        memset(constants.contents, 0, 256); // root dword 4: heap index zero
        // Apple's IRDescriptorTableEntry for an untyped buffer: GPU address,
        // unused texture ID, then byte length in the low 32 bits.
        uint64_t *heapEntry = heap.contents;
        heapEntry[0] = output.gpuAddress; heapEntry[1] = 0; heapEntry[2] = 64;
        uint64_t *topEntry = top.contents;
        topEntry[0] = constants.gpuAddress; topEntry[1] = 0; topEntry[2] = 256;
        id<MTLCommandQueue> queue = [device newCommandQueue];
        id<MTLCommandBuffer> cmd = [queue commandBuffer];
        id<MTLComputeCommandEncoder> enc = [cmd computeCommandEncoder];
        if (!queue || !cmd || !enc) return 8;
        [enc setComputePipelineState:pipeline];
        [enc setBuffer:heap offset:0 atIndex:0]; // kIRDescriptorHeapBindPoint
        [enc setBuffer:top offset:0 atIndex:2];  // kIRArgumentBufferBindPoint
        [enc useResource:constants usage:MTLResourceUsageRead];
        [enc useResource:output usage:MTLResourceUsageWrite];
        [enc dispatchThreadgroups:MTLSizeMake(1, 1, 1) threadsPerThreadgroup:MTLSizeMake(1, 1, 1)];
        [enc endEncoding];
        [cmd commit];
        [cmd waitUntilCompleted];
        if (cmd.status != MTLCommandBufferStatusCompleted) {
            fprintf(stderr, "GPU dispatch failed: %s\n", cmd.error.description.UTF8String); return 9;
        }
        const uint32_t *values = output.contents;
        if (values[0] != 42) { fprintf(stderr, "expected 42, got %u\n", values[0]); return 10; }
        for (int i = 1; i < 16; ++i) {
            if (values[i] != 0xA5A5A5A5) { fprintf(stderr, "unexpected write at %d\n", i); return 11; }
        }
        printf("device=%s result=%u\n", device.name.UTF8String, values[0]);
        return 0;
    }
}
