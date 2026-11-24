/* A normally launched TLS guest exercising two raw buffer table entries. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <string.h>
#include "shaders.h"

__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { void *root; struct { const void *code; U64 size; } stage; } ComputeDesc;
typedef HRESULT (__stdcall *CreateHeap)(void *, const void *, const IID *, void **);
typedef HRESULT (__stdcall *CreateResource)(void *, const void *, unsigned, const void *, unsigned, const void *, const IID *, void **);
typedef HRESULT (__stdcall *CreatePipeline)(void *, const void *, const IID *, void **);
typedef HRESULT (__stdcall *Map)(void *, unsigned, const void *, void **);
typedef U64 (__stdcall *Handle)(void *);
typedef unsigned (__stdcall *Increment)(void *, unsigned);
typedef void (__stdcall *CreateSrv)(void *, void *, const void *, U64);
typedef void (__stdcall *CreateUav)(void *, void *, void *, const void *, U64);
typedef void (__stdcall *SetHeaps)(void *, unsigned, void **);
typedef void (__stdcall *SetTable)(void *, unsigned, U64);
typedef void (__stdcall *Dispatch)(void *, unsigned, unsigned, unsigned);
typedef unsigned (__stdcall *Release)(void *);
typedef HRESULT (__stdcall *CreateFence)(void *, U64, unsigned, const IID *, void **);
typedef HRESULT (__stdcall *QueueFence)(void *, void *, U64);
typedef HRESULT (__stdcall *FenceSignal)(void *, U64);
typedef HRESULT (__stdcall *FenceEvent)(void *, U64, HANDLE);
static _Thread_local volatile int tls_value = 17;
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), s, (DWORD)strlen(s), &n, NULL); }

enum Case { VALID, BAD_SLOT, MISSING, ALIAS, SRV_STATE, UAV_STATE, SRV_OFFSET, SRV_RANGE, UAV_OFFSET, UAV_RANGE, SRV_TYPED, UAV_STRUCTURED, UAV_COUNTER, UAV_FLAG };
int main(int argc, char **argv) {
    static const struct { const char *arg; enum Case value; } cases[] = {
        {"--invalid-slot", BAD_SLOT}, {"--invalid-missing", MISSING}, {"--invalid-alias", ALIAS},
        {"--invalid-srv-state", SRV_STATE}, {"--invalid-uav-state", UAV_STATE},
        {"--invalid-srv-offset", SRV_OFFSET}, {"--invalid-srv-range", SRV_RANGE},
        {"--invalid-uav-offset", UAV_OFFSET}, {"--invalid-uav-range", UAV_RANGE},
        {"--invalid-srv-typed", SRV_TYPED}, {"--invalid-uav-structured", UAV_STRUCTURED},
        {"--invalid-uav-counter", UAV_COUNTER}, {"--invalid-uav-flag", UAV_FLAG}
    };
    enum Case test = VALID;
    if (argc == 2) {
        unsigned i;
        for (i = 0; i < sizeof(cases)/sizeof(cases[0]); ++i) if (!strcmp(argv[1], cases[i].arg)) { test = cases[i].value; break; }
        if (i == sizeof(cases)/sizeof(cases[0])) return 90;
    } else if (argc != 1) return 90;
    if (tls_value != 17) return 91;
    tls_value = 29;
    unsigned version = 0x00020004;
    void *dev = NULL, *heap = NULL, *input = NULL, *output = NULL, *pipeline = NULL;
    if (D3D12XboxCreateDevice(NULL, &version, &IID_ID3D12Device, &dev) != S_OK || !dev) return 1;
    HeapDesc hd = {{D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, 2, D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, 0}, {0, 0}};
    if (SLOT(dev,0x70,CreateHeap)(dev, &hd, &IID_ID3D12DescriptorHeap, &heap) != S_OK || !heap) return 2;
    U64 handle = SLOT(heap,0x48,Handle)(heap);
    unsigned stride = SLOT(dev,0x78,Increment)(dev, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
    if (!stride) return 3;
    D3D12_HEAP_PROPERTIES hp = {D3D12_HEAP_TYPE_DEFAULT, 0, 0, 0, 0};
    D3D12_RESOURCE_DESC rd = {D3D12_RESOURCE_DIMENSION_BUFFER, 0, 128, 1, 1, 1, DXGI_FORMAT_UNKNOWN, {1, 0}, D3D12_TEXTURE_LAYOUT_ROW_MAJOR, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS};
    CreateResource make = SLOT(dev,0xd8,CreateResource);
    unsigned in_state = test == SRV_STATE ? D3D12_RESOURCE_STATE_COPY_DEST : D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE;
    if (make(dev, &hp, 0, &rd, in_state, NULL, &IID_ID3D12Resource, &input) != S_OK || !input) return 4;
    unsigned out_state = test == UAV_STATE ? D3D12_RESOURCE_STATE_COPY_SOURCE : D3D12_RESOURCE_STATE_UNORDERED_ACCESS;
    if (test == UAV_FLAG) rd.Flags = D3D12_RESOURCE_FLAG_NONE;
    if (make(dev, &hp, 0, &rd, out_state, NULL, &IID_ID3D12Resource, &output) != S_OK || !output) return 5;
    volatile unsigned *src = NULL, *dst = NULL;
    if (SLOT(input,0x40,Map)(input, 0, NULL, (void **)&src) != S_OK || !src ||
        SLOT(output,0x40,Map)(output, 0, NULL, (void **)&dst) != S_OK || !dst) return 6;
    for (unsigned i = 0; i < 32; ++i) { src[i] = 1000 + i; dst[i] = 2000 + i; }
    src[5] = 3; dst[11] = 1;
    if (test != VALID) say("D3D12X invalid raw views armed\r\n");
    D3D12_SHADER_RESOURCE_VIEW_DESC srv = {0};
    srv.Format = test == SRV_TYPED ? DXGI_FORMAT_R32_UINT : DXGI_FORMAT_R32_TYPELESS;
    srv.ViewDimension = D3D12_SRV_DIMENSION_BUFFER;
    srv.Shader4ComponentMapping = D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
    srv.Buffer.FirstElement = test == SRV_OFFSET ? 32 : 4;
    srv.Buffer.NumElements = test == SRV_RANGE ? 30 : 4;
    srv.Buffer.Flags = test == SRV_TYPED ? D3D12_BUFFER_SRV_FLAG_NONE : D3D12_BUFFER_SRV_FLAG_RAW;
    SLOT(dev,0x90,CreateSrv)(dev, input, &srv, handle);
    D3D12_UNORDERED_ACCESS_VIEW_DESC uav = {0};
    uav.Format = test == UAV_STRUCTURED ? DXGI_FORMAT_UNKNOWN : DXGI_FORMAT_R32_TYPELESS;
    uav.ViewDimension = D3D12_UAV_DIMENSION_BUFFER;
    uav.Buffer.FirstElement = test == UAV_OFFSET ? 32 : 10;
    uav.Buffer.NumElements = test == UAV_RANGE ? 30 : 4;
    uav.Buffer.StructureByteStride = test == UAV_STRUCTURED ? 4 : 0;
    uav.Buffer.Flags = test == UAV_STRUCTURED ? D3D12_BUFFER_UAV_FLAG_NONE : D3D12_BUFFER_UAV_FLAG_RAW;
    if (test != MISSING) SLOT(dev,0x98,CreateUav)(dev, test == ALIAS ? input : output, test == UAV_COUNTER ? input : NULL, &uav, handle + stride);
    if (test != VALID) say("D3D12X invalid descriptors accepted\r\n");
    ComputeDesc pd = {NULL, {shader, sizeof(shader)}};
    if (SLOT(dev,0x58,CreatePipeline)(dev, &pd, &IID_ID3D12PipelineState, &pipeline) != S_OK || !pipeline) return 7;
    ID3D12CommandAllocator *allocator = NULL;
    ID3D12GraphicsCommandList *lists[2] = {NULL, NULL};
    ID3D12CommandQueue *queue = NULL;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev, D3D12_COMMAND_LIST_TYPE_COMPUTE, &IID_ID3D12CommandAllocator, (void **)&allocator) != S_OK) return 8;
    unsigned count = test == VALID ? 2 : 1;
    for (unsigned i = 0; i < count; ++i) {
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev, 0, D3D12_COMMAND_LIST_TYPE_COMPUTE, allocator, (ID3D12PipelineState *)pipeline, &IID_ID3D12GraphicsCommandList, (void **)&lists[i]) != S_OK) return 9;
        ID3D12GraphicsCommandList_SetPipelineState(lists[i], (ID3D12PipelineState *)pipeline);
        SLOT(lists[i],0x100,SetHeaps)(lists[i], 1, &heap);
        SLOT(lists[i],0x118,SetTable)(lists[i], test == BAD_SLOT ? 2 : 1, handle);
        SLOT(lists[i],0x70,Dispatch)(lists[i], 1, 1, 1);
        HRESULT closed = ID3D12GraphicsCommandList_Close(lists[i]);
        if (test == BAD_SLOT || test == MISSING || test == ALIAS) {
            if (closed == S_OK || dst[10] != 2010) return 10;
            say("D3D12X invalid raw views rejected\r\n"); return 43;
        }
        if (closed != S_OK) return 11;
    }
    D3D12_COMMAND_QUEUE_DESC qd = {D3D12_COMMAND_LIST_TYPE_COMPUTE, 0, 0, 0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev, &qd, &IID_ID3D12CommandQueue, (void **)&queue) != S_OK) return 12;
    src[5] = 41;
    SLOT(input,0x10,Release)(input); SLOT(heap,0x10,Release)(heap);
    ID3D12CommandList *submit[] = {(ID3D12CommandList *)lists[0]};
    if (test != VALID) say("D3D12X invalid dispatch armed\r\n");
    ID3D12CommandQueue_ExecuteCommandLists(queue, 1, submit);
    if (test != VALID) return 13;
    if (dst[10] != 41 || dst[11] != 1) return 14;
    for (unsigned i = 0; i < 32; ++i) if (i != 10 && i != 11 && dst[i] != 2000 + i) return 15;
    void *gate = NULL, *done = NULL;
    HANDLE event = CreateEventW(NULL, FALSE, FALSE, NULL);
    if (!event || SLOT(dev,0x120,CreateFence)(dev,0,0,&IID_ID3D12Fence,&gate) != S_OK ||
        SLOT(dev,0x120,CreateFence)(dev,0,0,&IID_ID3D12Fence,&done) != S_OK ||
        SLOT(done,0x48,FenceEvent)(done,1,event) != S_OK || SLOT(queue,0x78,QueueFence)(queue,gate,1) != S_OK) return 16;
    submit[0] = (ID3D12CommandList *)lists[1];
    ID3D12CommandQueue_ExecuteCommandLists(queue, 1, submit);
    if (SLOT(queue,0x70,QueueFence)(queue,done,1) != S_OK || WaitForSingleObject(event,50) != WAIT_TIMEOUT || dst[10] != 41) return 17;
    src[5] = 83;
    if (SLOT(gate,0x50,FenceSignal)(gate,1) != S_OK || WaitForSingleObject(event,10000) != WAIT_OBJECT_0 || dst[10] != 83 || dst[11] != 1 || tls_value != 29) return 18;
    for (unsigned i = 0; i < 32; ++i) if (i != 10 && i != 11 && dst[i] != 2000 + i) return 19;
    CloseHandle(event); SLOT(gate,0x10,Release)(gate); SLOT(done,0x10,Release)(done);
    SLOT(queue,0x10,Release)(queue); SLOT(lists[1],0x10,Release)(lists[1]); SLOT(lists[0],0x10,Release)(lists[0]);
    SLOT(allocator,0x10,Release)(allocator); SLOT(pipeline,0x10,Release)(pipeline); SLOT(output,0x10,Release)(output); SLOT(dev,0x10,Release)(dev);
    say("D3D12X raw views passed\r\n"); return 42;
}
