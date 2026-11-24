/* Normally launched Wine PE using the native synthetic root-CBV shader. */
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
typedef void (__stdcall *CreateUav)(void *, void *, void *, const void *, U64);
typedef void (__stdcall *SetHeaps)(void *, unsigned, void **);
typedef void (__stdcall *SetTable)(void *, unsigned, U64);
typedef void (__stdcall *SetView)(void *, unsigned, U64);
typedef void (__stdcall *Dispatch)(void *, unsigned, unsigned, unsigned);
typedef unsigned (__stdcall *Release)(void *);
typedef HRESULT (__stdcall *CreateFence)(void *, U64, unsigned, const IID *, void **);
typedef HRESULT (__stdcall *QueueFence)(void *, void *, U64);
typedef HRESULT (__stdcall *FenceSignal)(void *, U64);
typedef HRESULT (__stdcall *FenceEvent)(void *, U64, HANDLE);

static _Thread_local volatile int tls_value = 17;
static void say(const char *s) {
    DWORD written;
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), s, (DWORD)strlen(s), &written, NULL);
}

enum Case { VALID, SLOT_INVALID, MISSING, ADDRESS, ALIGNMENT, SHORT_BUFFER, ALIAS, SRV, UAV, BINDING, COPY_SOURCE, COPY_DEST };

int main(int argc, char **argv) {
    enum Case test = VALID;
    if (argc == 2) {
        if (!strcmp(argv[1], "--invalid-slot")) test = SLOT_INVALID;
        else if (!strcmp(argv[1], "--invalid-missing")) test = MISSING;
        else if (!strcmp(argv[1], "--invalid-address")) test = ADDRESS;
        else if (!strcmp(argv[1], "--invalid-alignment")) test = ALIGNMENT;
        else if (!strcmp(argv[1], "--invalid-short")) test = SHORT_BUFFER;
        else if (!strcmp(argv[1], "--invalid-alias")) test = ALIAS;
        else if (!strcmp(argv[1], "--invalid-srv")) test = SRV;
        else if (!strcmp(argv[1], "--invalid-uav")) test = UAV;
        else if (!strcmp(argv[1], "--invalid-binding")) test = BINDING;
        else if (!strcmp(argv[1], "--invalid-copy-source")) test = COPY_SOURCE;
        else if (!strcmp(argv[1], "--invalid-copy-dest")) test = COPY_DEST;
        else return 90;
    } else if (argc != 1) return 90;
    if (tls_value != 17) return 91;
    tls_value = 29;

    unsigned version = 0x00020004;
    void *dev = NULL, *heap = NULL, *cbv = NULL, *output = NULL, *pipeline = NULL;
    if (D3D12XboxCreateDevice(NULL, &version, &IID_ID3D12Device, &dev) != S_OK || !dev) return 1;
    HeapDesc hd = {{D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, 1, D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, 0}, {0, 0}};
    if (SLOT(dev,0x70,CreateHeap)(dev, &hd, &IID_ID3D12DescriptorHeap, &heap) != S_OK || !heap) return 2;
    U64 handle = SLOT(heap,0x48,Handle)(heap);
    D3D12_HEAP_PROPERTIES hp = {D3D12_HEAP_TYPE_DEFAULT, 0, 0, 0, 0};
    D3D12_RESOURCE_DESC rd = {D3D12_RESOURCE_DIMENSION_BUFFER, 0, 128, 1, 1, 1, DXGI_FORMAT_UNKNOWN, {1, 0}, D3D12_TEXTURE_LAYOUT_ROW_MAJOR, 0};
    CreateResource make = SLOT(dev,0xd8,CreateResource);
    unsigned state = test == COPY_SOURCE ? D3D12_RESOURCE_STATE_COPY_SOURCE :
        test == COPY_DEST ? D3D12_RESOURCE_STATE_COPY_DEST : D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER;
    if (make(dev, &hp, 0, &rd, state, NULL, &IID_ID3D12Resource, &cbv) != S_OK || !cbv) return 3;
    rd.Flags = D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;
    if (make(dev, &hp, 0, &rd, 0, NULL, &IID_ID3D12Resource, &output) != S_OK || !output) return 4;
    volatile unsigned *constants = NULL, *result = NULL;
    if (SLOT(cbv,0x40,Map)(cbv, 0, NULL, (void **)&constants) != S_OK || !constants) return 5;
    if (SLOT(output,0x40,Map)(output, 0, NULL, (void **)&result) != S_OK || !result) return 6;
    constants[4] = 999; constants[8] = 3; constants[20] = 5;
    result[0] = 7; result[1] = 1;
    D3D12_UNORDERED_ACCESS_VIEW_DESC uav = {0};
    uav.Format = DXGI_FORMAT_R32_TYPELESS;
    uav.ViewDimension = D3D12_UAV_DIMENSION_BUFFER;
    uav.Buffer.NumElements = 32;
    uav.Buffer.Flags = D3D12_BUFFER_UAV_FLAG_RAW;
    SLOT(dev,0x98,CreateUav)(dev, output, NULL, &uav, handle);
    ComputeDesc pd = {NULL, {shader, sizeof(shader)}};
    if (SLOT(dev,0x58,CreatePipeline)(dev, &pd, &IID_ID3D12PipelineState, &pipeline) != S_OK || !pipeline) return 7;
    ID3D12CommandAllocator *allocator = NULL;
    ID3D12GraphicsCommandList *lists[2] = {NULL, NULL};
    ID3D12CommandQueue *queue = NULL;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev, D3D12_COMMAND_LIST_TYPE_COMPUTE, &IID_ID3D12CommandAllocator, (void **)&allocator) != S_OK) return 8;
    U64 address = SLOT(cbv,0x58,Handle)(cbv);
    unsigned count = test == VALID ? 2 : 1;
    for (unsigned i = 0; i < count; ++i) {
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev, 0, D3D12_COMMAND_LIST_TYPE_COMPUTE, allocator, (ID3D12PipelineState *)pipeline, &IID_ID3D12GraphicsCommandList, (void **)&lists[i]) != S_OK) return 9;
        ID3D12GraphicsCommandList_SetPipelineState(lists[i], (ID3D12PipelineState *)pipeline);
        SLOT(lists[i],0x100,SetHeaps)(lists[i], 1, &heap);
        SLOT(lists[i],0x118,SetTable)(lists[i], 1, handle);
        U64 at = address + (i ? 64 : 16);
        if (test == ADDRESS) at = address + 128;
        else if (test == ALIGNMENT) at = address + 17;
        else if (test == SHORT_BUFFER) at = address + 112;
        else if (test == ALIAS) at = SLOT(output,0x58,Handle)(output);
        if (test != VALID) say("D3D12X invalid CBV case armed\r\n");
        if (test == SRV) SLOT(lists[i],0x158,SetView)(lists[i], 2, at);
        else if (test == UAV) SLOT(lists[i],0x168,SetView)(lists[i], 2, at);
        else if (test != MISSING) SLOT(lists[i],0x148,SetView)(lists[i], test == SLOT_INVALID ? 64 : test == BINDING ? 3 : 2, at);
        SLOT(lists[i],0x70,Dispatch)(lists[i], 1, 1, 1);
        HRESULT closed = ID3D12GraphicsCommandList_Close(lists[i]);
        if (test == SLOT_INVALID || test == ADDRESS || test == ALIGNMENT || test == ALIAS) {
            if (closed == S_OK || result[0] != 7) return 10;
            say("D3D12X invalid CBV case rejected\r\n");
            return 43;
        }
        if (closed != S_OK) return 11;
    }
    D3D12_COMMAND_QUEUE_DESC qd = {D3D12_COMMAND_LIST_TYPE_COMPUTE, 0, 0, 0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev, &qd, &IID_ID3D12CommandQueue, (void **)&queue) != S_OK) return 12;
    /* Both lists retain the CBV after the guest releases it. The first write
     * happens after recording; the second happens between submissions. */
    constants[8] = 41;
    SLOT(cbv,0x10,Release)(cbv);
    SLOT(heap,0x10,Release)(heap);
    ID3D12CommandList *submit[] = {(ID3D12CommandList *)lists[0]};
    ID3D12CommandQueue_ExecuteCommandLists(queue, 1, submit);
    if (test != VALID) return 13; /* Native validation must reject. */
    if (result[0] != 42 || result[1] != 1) return 14;
    void *gate = NULL, *done = NULL;
    HANDLE event = CreateEventW(NULL, FALSE, FALSE, NULL);
    if (!event || SLOT(dev,0x120,CreateFence)(dev,0,0,&IID_ID3D12Fence,&gate) != S_OK ||
        SLOT(dev,0x120,CreateFence)(dev,0,0,&IID_ID3D12Fence,&done) != S_OK ||
        SLOT(done,0x48,FenceEvent)(done,1,event) != S_OK ||
        SLOT(queue,0x78,QueueFence)(queue,gate,1) != S_OK) return 15;
    submit[0] = (ID3D12CommandList *)lists[1];
    ID3D12CommandQueue_ExecuteCommandLists(queue, 1, submit);
    if (SLOT(queue,0x70,QueueFence)(queue,done,1) != S_OK ||
        WaitForSingleObject(event, 50) != WAIT_TIMEOUT || result[0] != 42) return 16;
    constants[20] = 83; /* The deferred dispatch must observe this late write. */
    if (SLOT(gate,0x50,FenceSignal)(gate,1) != S_OK ||
        WaitForSingleObject(event, 10000) != WAIT_OBJECT_0 ||
        result[0] != 84 || result[1] != 1 || tls_value != 29) return 17;
    CloseHandle(event);
    SLOT(gate,0x10,Release)(gate);
    SLOT(done,0x10,Release)(done);
    SLOT(queue,0x10,Release)(queue);
    SLOT(lists[1],0x10,Release)(lists[1]);
    SLOT(lists[0],0x10,Release)(lists[0]);
    SLOT(allocator,0x10,Release)(allocator);
    SLOT(pipeline,0x10,Release)(pipeline);
    SLOT(output,0x10,Release)(output);
    SLOT(dev,0x10,Release)(dev);
    say("D3D12X root CBV passed\r\n");
    return 42;
}
