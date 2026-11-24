/* Normally launched TLS guest exercising a nonzero-base multi-view table. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <string.h>
#include <stdio.h>
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

enum Case { VALID, SHORT_VIEW, BAD_SLOT, MISSING, ALIAS, MISMATCH, SRV_STATE, UAV_STATE, SRV_OFFSET, SRV_RANGE, UAV_OFFSET, UAV_RANGE };
int main(int argc, char **argv) {
    static const struct { const char *arg; enum Case value; } cases[] = {
        {"--short-view", SHORT_VIEW}, {"--invalid-slot", BAD_SLOT},
        {"--invalid-missing", MISSING}, {"--invalid-alias", ALIAS}, {"--invalid-mismatch", MISMATCH},
        {"--invalid-srv-state", SRV_STATE}, {"--invalid-uav-state", UAV_STATE},
        {"--invalid-srv-offset", SRV_OFFSET}, {"--invalid-srv-range", SRV_RANGE},
        {"--invalid-uav-offset", UAV_OFFSET}, {"--invalid-uav-range", UAV_RANGE}
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
    void *dev = NULL, *heap = NULL, *resources[6] = {NULL}, *pipeline = NULL;
    volatile unsigned *data[6] = {NULL};
    if (D3D12XboxCreateDevice(NULL, &version, &IID_ID3D12Device, &dev) != S_OK || !dev) return 1;
    HeapDesc hd = {{D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, 6, D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, 0}, {0, 0}};
    if (SLOT(dev,0x70,CreateHeap)(dev, &hd, &IID_ID3D12DescriptorHeap, &heap) != S_OK || !heap) return 2;
    U64 handle = SLOT(heap,0x48,Handle)(heap);
    unsigned stride = SLOT(dev,0x78,Increment)(dev, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
    if (!stride) return 3;
    D3D12_HEAP_PROPERTIES hp = {D3D12_HEAP_TYPE_DEFAULT, 0, 0, 0, 0};
    D3D12_RESOURCE_DESC rd = {D3D12_RESOURCE_DIMENSION_BUFFER, 0, 128, 1, 1, 1, DXGI_FORMAT_UNKNOWN, {1, 0}, D3D12_TEXTURE_LAYOUT_ROW_MAJOR, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS};
    CreateResource make = SLOT(dev,0xd8,CreateResource);
    for (unsigned i = 0; i < 6; ++i) {
        int writable = i == 2 || i == 4;
        unsigned state = writable ? D3D12_RESOURCE_STATE_UNORDERED_ACCESS : D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE;
        if ((i == 5 && test == SRV_STATE) || (i == 4 && test == UAV_STATE)) state = D3D12_RESOURCE_STATE_COPY_DEST;
        if (make(dev, &hp, 0, &rd, state, NULL, &IID_ID3D12Resource, &resources[i]) != S_OK || !resources[i]) return 4;
        if (SLOT(resources[i],0x40,Map)(resources[i], 0, NULL, (void **)&data[i]) != S_OK || !data[i]) return 5;
        for (unsigned j = 0; j < 32; ++j) data[i][j] = 10000 * (i + 1) + j;
    }
    data[3][5] = 3; data[5][14] = 5;
    if (test != VALID && test != SHORT_VIEW) say("D3D12X invalid multi views armed\r\n");
    for (unsigned i = 0; i < 6; ++i) {
        U64 dst = handle + stride * i;
        if (i == 2 || i == 4) {
            D3D12_UNORDERED_ACCESS_VIEW_DESC view = {0};
            view.Format = DXGI_FORMAT_R32_TYPELESS; view.ViewDimension = D3D12_UAV_DIMENSION_BUFFER;
            view.Buffer.FirstElement = i == 2 ? 10 : 20; view.Buffer.NumElements = 4;
            view.Buffer.Flags = D3D12_BUFFER_UAV_FLAG_RAW;
            if (i == 4 && test == UAV_OFFSET) view.Buffer.FirstElement = 32;
            if (i == 4 && test == UAV_RANGE) view.Buffer.NumElements = 30;
            if (i == 4 && test == MISSING) continue;
            if (i == 4 && test == MISMATCH) {
                D3D12_SHADER_RESOURCE_VIEW_DESC wrong = {0};
                wrong.Format = DXGI_FORMAT_R32_TYPELESS; wrong.ViewDimension = D3D12_SRV_DIMENSION_BUFFER;
                wrong.Shader4ComponentMapping = D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
                wrong.Buffer.FirstElement = 20; wrong.Buffer.NumElements = 4; wrong.Buffer.Flags = D3D12_BUFFER_SRV_FLAG_RAW;
                SLOT(dev,0x90,CreateSrv)(dev, resources[i], &wrong, dst);
            } else {
                SLOT(dev,0x98,CreateUav)(dev, i == 4 && test == ALIAS ? resources[2] : resources[i], NULL, &view, dst);
            }
        } else {
            D3D12_SHADER_RESOURCE_VIEW_DESC view = {0};
            view.Format = DXGI_FORMAT_R32_TYPELESS; view.ViewDimension = D3D12_SRV_DIMENSION_BUFFER;
            view.Shader4ComponentMapping = D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
            view.Buffer.FirstElement = i == 3 ? 4 : i == 5 ? 12 : 0;
            view.Buffer.NumElements = i == 5 && test == SHORT_VIEW ? 2 : 4;
            view.Buffer.Flags = D3D12_BUFFER_SRV_FLAG_RAW;
            if (i == 5 && test == SRV_OFFSET) view.Buffer.FirstElement = 32;
            if (i == 5 && test == SRV_RANGE) view.Buffer.NumElements = 30;
            SLOT(dev,0x90,CreateSrv)(dev, resources[i], &view, dst);
        }
    }
    if (test != VALID && test != SHORT_VIEW) say("D3D12X invalid descriptors accepted\r\n");
    ComputeDesc pd = {NULL, {shader, sizeof(shader)}};
    if (SLOT(dev,0x58,CreatePipeline)(dev, &pd, &IID_ID3D12PipelineState, &pipeline) != S_OK || !pipeline) return 7;
    ID3D12CommandAllocator *allocator = NULL;
    ID3D12GraphicsCommandList *lists[2] = {NULL, NULL};
    ID3D12CommandQueue *queue = NULL;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev, D3D12_COMMAND_LIST_TYPE_COMPUTE, &IID_ID3D12CommandAllocator, (void **)&allocator) != S_OK) return 8;
    unsigned count = test == VALID || test == SHORT_VIEW ? 2 : 1;
    for (unsigned i = 0; i < count; ++i) {
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev, 0, D3D12_COMMAND_LIST_TYPE_COMPUTE, allocator, (ID3D12PipelineState *)pipeline, &IID_ID3D12GraphicsCommandList, (void **)&lists[i]) != S_OK) return 9;
        ID3D12GraphicsCommandList_SetPipelineState(lists[i], (ID3D12PipelineState *)pipeline);
        SLOT(lists[i],0x100,SetHeaps)(lists[i], 1, &heap);
        SLOT(lists[i],0x118,SetTable)(lists[i], test == BAD_SLOT ? 6 : 7, handle + 2 * stride);
        SLOT(lists[i],0x70,Dispatch)(lists[i], 1, 1, 1);
        HRESULT closed = ID3D12GraphicsCommandList_Close(lists[i]);
        if (test == MISSING || test == ALIAS) {
            if (closed == S_OK || data[2][10] != 30010 || data[4][21] != 50021) return 10;
            say("D3D12X invalid multi views rejected\r\n"); return 43;
        }
        if (closed != S_OK) return 11;
    }
    D3D12_COMMAND_QUEUE_DESC qd = {D3D12_COMMAND_LIST_TYPE_COMPUTE, 0, 0, 0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev, &qd, &IID_ID3D12CommandQueue, (void **)&queue) != S_OK) return 12;
    data[3][5] = 41; data[5][14] = 53;
    for (unsigned i = 0; i < 6; ++i) SLOT(resources[i],0x10,Release)(resources[i]);
    SLOT(heap,0x10,Release)(heap);
    ID3D12CommandList *submit[] = {(ID3D12CommandList *)lists[0]};
    if (test != VALID && test != SHORT_VIEW) say("D3D12X invalid dispatch armed\r\n");
    ID3D12CommandQueue_ExecuteCommandLists(queue, 1, submit);
    if (test != VALID && test != SHORT_VIEW) return 13;
    if (data[2][10] != 41 || data[4][21] != (test == SHORT_VIEW ? 0u : 53u)) {
        char message[128];
        snprintf(message, sizeof(message), "multi outputs: %u %u (case %u)\r\n", data[2][10], data[4][21], test);
        say(message); return 14;
    }
    for (unsigned i = 0; i < 32; ++i)
        if ((i != 10 && data[2][i] != 30000 + i) || (i != 21 && data[4][i] != 50000 + i) ||
            data[5][i] != (i == 14 ? 53u : 60000 + i)) return 15;
    void *gate = NULL, *done = NULL;
    HANDLE event = CreateEventW(NULL, FALSE, FALSE, NULL);
    if (!event || SLOT(dev,0x120,CreateFence)(dev,0,0,&IID_ID3D12Fence,&gate) != S_OK ||
        SLOT(dev,0x120,CreateFence)(dev,0,0,&IID_ID3D12Fence,&done) != S_OK ||
        SLOT(done,0x48,FenceEvent)(done,1,event) != S_OK || SLOT(queue,0x78,QueueFence)(queue,gate,1) != S_OK) return 16;
    submit[0] = (ID3D12CommandList *)lists[1];
    ID3D12CommandQueue_ExecuteCommandLists(queue, 1, submit);
    if (SLOT(queue,0x70,QueueFence)(queue,done,1) != S_OK || WaitForSingleObject(event,50) != WAIT_TIMEOUT ||
        data[2][10] != 41 || data[4][21] != (test == SHORT_VIEW ? 0u : 53u)) return 17;
    data[3][5] = 83; data[5][14] = 97;
    if (SLOT(gate,0x50,FenceSignal)(gate,1) != S_OK || WaitForSingleObject(event,10000) != WAIT_OBJECT_0 ||
        data[2][10] != 83 || data[4][21] != (test == SHORT_VIEW ? 0u : 97u) || tls_value != 29) return 18;
    for (unsigned i = 0; i < 32; ++i)
        if ((i != 10 && data[2][i] != 30000 + i) || (i != 21 && data[4][i] != 50000 + i) ||
            data[5][i] != (i == 14 ? 97u : 60000 + i)) return 19;
    CloseHandle(event); SLOT(gate,0x10,Release)(gate); SLOT(done,0x10,Release)(done);
    SLOT(queue,0x10,Release)(queue); SLOT(lists[1],0x10,Release)(lists[1]); SLOT(lists[0],0x10,Release)(lists[0]);
    SLOT(allocator,0x10,Release)(allocator); SLOT(pipeline,0x10,Release)(pipeline); SLOT(dev,0x10,Release)(dev);
    say("D3D12X multi views passed\r\n"); return 42;
}
