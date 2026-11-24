/* Normally launched TLS PE: view bounds must be checked before Metal dispatch. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <stdio.h>
#include <stdlib.h>
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
typedef U64 (__stdcall *Address)(void *);
typedef unsigned (__stdcall *Increment)(void *, unsigned);
typedef void (__stdcall *CreateSrv)(void *, void *, const void *, U64);
typedef void (__stdcall *CreateUav)(void *, void *, void *, const void *, U64);
typedef void (__stdcall *SetHeaps)(void *, unsigned, void **);
typedef void (__stdcall *SetTable)(void *, unsigned, U64);
typedef void (__stdcall *SetCbv)(void *, unsigned, U64);
typedef void (__stdcall *Dispatch)(void *, unsigned, unsigned, unsigned);
static _Thread_local volatile int tls_value = 17;
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), s, (DWORD)strlen(s), &n, NULL); }

/* The two read views and two write views have unequal lengths and nonzero
   offsets into larger allocations. Every untouched word is a sentinel. */
typedef HRESULT (__stdcall *CreateFence)(void *, U64, unsigned, const IID *, void **);
typedef HRESULT (__stdcall *FenceEvent)(void *, U64, HANDLE);
typedef HRESULT (__stdcall *QueueSignal)(void *, void *, U64);
int main(int argc, char **argv) {
    if (argc != 3) return 90;
    int mode = atoi(argv[1]), test = atoi(argv[2]);
    if (mode != 3 || test < 0 || test > 12 || tls_value != 17) return 91;
    tls_value = 29;
    unsigned version = 0x00020004;
    void *dev = NULL, *heap = NULL, *resources[4] = {0}, *pipeline = NULL;
    volatile unsigned *data[4] = {0};
    if (D3D12XboxCreateDevice(NULL, &version, &IID_ID3D12Device, &dev) != S_OK || !dev) return 1;
    HeapDesc hd = {{D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, 4, D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, 0}, {0, 0}};
    if (SLOT(dev,0x70,CreateHeap)(dev, &hd, &IID_ID3D12DescriptorHeap, &heap) != S_OK || !heap) return 2;
    U64 handle = SLOT(heap,0x48,Handle)(heap);
    unsigned stride = SLOT(dev,0x78,Increment)(dev, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
    if (!stride) return 3;
    D3D12_HEAP_PROPERTIES hp = {D3D12_HEAP_TYPE_DEFAULT, 0, 0, 0, 0};
    D3D12_RESOURCE_DESC rd = {D3D12_RESOURCE_DIMENSION_BUFFER, 0, 128, 1, 1, 1, DXGI_FORMAT_UNKNOWN, {1, 0}, D3D12_TEXTURE_LAYOUT_ROW_MAJOR, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS};
    CreateResource make = SLOT(dev,0xd8,CreateResource);
    for (unsigned i = 0; i < 4; ++i) {
        int writable = i % 2 == 0;
        unsigned state = writable ? D3D12_RESOURCE_STATE_UNORDERED_ACCESS : D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE;
        if (make(dev, &hp, 0, &rd, state, NULL, &IID_ID3D12Resource, &resources[i]) != S_OK || !resources[i]) return 4;
        if (SLOT(resources[i],0x40,Map)(resources[i], 0, NULL, (void **)&data[i]) != S_OK || !data[i]) return 5;
        for (unsigned j = 0; j < 32; ++j) data[i][j] = 10000 * (i + 1) + j;
        if (writable) {
            D3D12_UNORDERED_ACCESS_VIEW_DESC view = {0};
            view.Format = DXGI_FORMAT_R32_TYPELESS; view.ViewDimension = D3D12_UAV_DIMENSION_BUFFER;
            view.Buffer.FirstElement = 10; view.Buffer.NumElements = i == 0 ? 4 : 2; view.Buffer.Flags = D3D12_BUFFER_UAV_FLAG_RAW;
            SLOT(dev,0x98,CreateUav)(dev, resources[i], NULL, &view, handle + i * stride);
        } else {
            D3D12_SHADER_RESOURCE_VIEW_DESC view = {0};
            view.Format = DXGI_FORMAT_R32_TYPELESS; view.ViewDimension = D3D12_SRV_DIMENSION_BUFFER;
            view.Shader4ComponentMapping = D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
            view.Buffer.FirstElement = 4; view.Buffer.NumElements = i == 1 ? 4 : 3; view.Buffer.Flags = D3D12_BUFFER_SRV_FLAG_RAW;
            SLOT(dev,0x90,CreateSrv)(dev, resources[i], &view, handle + i * stride);
        }
    }
    unsigned first = 8, second = 8;
    if (test == 1) { first = 16; second = 12; }
    if (test == 10) first = 0xfffffffcu;
    if (test == 5 || test == 11) first = second = 0xfffffffcu;
    if (test == 8) { first = 16; second = 8; }
    data[1][4] = first; data[3][4] = second;
    ComputeDesc pd = {NULL, {shaders[mode][test].bytes, shaders[mode][test].size}};
    if (SLOT(dev,0x58,CreatePipeline)(dev, &pd, &IID_ID3D12PipelineState, &pipeline) != S_OK || !pipeline) return 7;
    ID3D12CommandAllocator *allocator = NULL;
    ID3D12GraphicsCommandList *list = NULL;
    ID3D12CommandQueue *queue = NULL;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev, D3D12_COMMAND_LIST_TYPE_COMPUTE, &IID_ID3D12CommandAllocator, (void **)&allocator) != S_OK ||
        ID3D12Device_CreateCommandList((ID3D12Device *)dev, 0, D3D12_COMMAND_LIST_TYPE_COMPUTE, allocator, (ID3D12PipelineState *)pipeline, &IID_ID3D12GraphicsCommandList, (void **)&list) != S_OK) return 8;
    ID3D12GraphicsCommandList_SetPipelineState(list, (ID3D12PipelineState *)pipeline);
    SLOT(list,0x100,SetHeaps)(list, 1, &heap);
    SLOT(list,0x118,SetTable)(list, 7, handle);
    SLOT(list,0x70,Dispatch)(list, 1, 1, 1);
    if (ID3D12GraphicsCommandList_Close(list) != S_OK) return 9;
    D3D12_COMMAND_QUEUE_DESC qd = {D3D12_COMMAND_LIST_TYPE_COMPUTE, 0, 0, 0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev, &qd, &IID_ID3D12CommandQueue, (void **)&queue) != S_OK || !queue) return 10;
    void *fence = NULL;
    HANDLE event = CreateEventW(NULL, FALSE, FALSE, NULL);
    if (!event || SLOT(dev,0x120,CreateFence)(dev, 0, 0, &IID_ID3D12Fence, &fence) != S_OK || !fence ||
        SLOT(fence,0x48,FenceEvent)(fence, 1, event) != S_OK) return 11;
    say("D3D12X bounds dispatch armed\r\n");
    ID3D12CommandList *submit[] = {(ID3D12CommandList *)list};
    ID3D12CommandQueue_ExecuteCommandLists(queue, 1, submit);
    if (test == 9) { say("D3D12X indexed dispatch returned\r\n"); return 12; }
    if (SLOT(queue,0x70,QueueSignal)(queue, fence, 1) != S_OK || WaitForSingleObject(event, 10000) != WAIT_OBJECT_0) return 15;
    CloseHandle(event);
    for (unsigned i = 0; i < 4; ++i) {
        for (unsigned j = 0; j < 32; ++j) {
            unsigned expected = 10000 * (i + 1) + j;
            if (i == 1 && j == 4) expected = first;
            if (i == 3 && j == 4) expected = second;
            if (test != 7 && test != 8 && test != 10 && test != 11 && test != 12 && i == 0 && j == 10)
                expected = (test == 0 || test == 3) ? 20006 : 0;
            if (test != 7 && test != 8 && test != 10 && test != 11 && test != 12 && i == 2 && j == 10)
                expected = (test == 0 || test == 3) ? 40006 : 0;
            if ((test == 7 || test == 10) && i == 0 && j == 12) expected = test == 7 ? 20007 : 0;
            if ((test == 7 || test == 10) && i == 0 && j == 13) expected = 0;
            if ((test == 7 || test == 10) && i == 2 && j == 11) expected = test == 7 ? 20007 : 0;
            if (data[i][j] != expected) {
                char msg[160];
                snprintf(msg, sizeof(msg), "case %d resource %u word %u: got %u expected %u\r\n", test, i, j, data[i][j], expected);
                say(msg); return 13;
            }
        }
    }
    if (tls_value != 29) return 14;
    say("D3D12X bounds dynamic passed\r\n"); return 42;
}
