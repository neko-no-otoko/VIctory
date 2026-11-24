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

/* mode: 0 legacy, 1 legacy+CBV, 2 paired, 3 multi. case: valid,
   out-of-view read, out-of-view store, dynamic read, dynamic store. */
int main(int argc, char **argv) {
    if (argc != 3) return 90;
    int mode = atoi(argv[1]), test = atoi(argv[2]);
    if (mode < 0 || mode > 3 || test < 0 || test > 4 || tls_value != 17) return 91;
    tls_value = 29;
    unsigned version = 0x00020004;
    void *dev = NULL, *heap = NULL, *resources[4] = {0}, *cbv = NULL, *pipeline = NULL;
    volatile unsigned *data[4] = {0}, *cbv_data = NULL;
    unsigned count = mode < 2 ? 1 : mode == 2 ? 2 : 4;
    if (D3D12XboxCreateDevice(NULL, &version, &IID_ID3D12Device, &dev) != S_OK || !dev) return 1;
    HeapDesc hd = {{D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, count, D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, 0}, {0, 0}};
    if (SLOT(dev,0x70,CreateHeap)(dev, &hd, &IID_ID3D12DescriptorHeap, &heap) != S_OK || !heap) return 2;
    U64 handle = SLOT(heap,0x48,Handle)(heap);
    unsigned stride = SLOT(dev,0x78,Increment)(dev, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
    if (!stride) return 3;
    D3D12_HEAP_PROPERTIES hp = {D3D12_HEAP_TYPE_DEFAULT, 0, 0, 0, 0};
    D3D12_RESOURCE_DESC rd = {D3D12_RESOURCE_DIMENSION_BUFFER, 0, mode < 2 ? 16 : 128, 1, 1, 1, DXGI_FORMAT_UNKNOWN, {1, 0}, D3D12_TEXTURE_LAYOUT_ROW_MAJOR, D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS};
    CreateResource make = SLOT(dev,0xd8,CreateResource);
    for (unsigned i = 0; i < count; ++i) {
        int writable = mode < 2 || (mode == 2 ? i == 1 : i % 2 == 0);
        unsigned state = writable ? D3D12_RESOURCE_STATE_UNORDERED_ACCESS : D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE;
        if (make(dev, &hp, 0, &rd, state, NULL, &IID_ID3D12Resource, &resources[i]) != S_OK || !resources[i]) return 4;
        if (SLOT(resources[i],0x40,Map)(resources[i], 0, NULL, (void **)&data[i]) != S_OK || !data[i]) return 5;
        for (unsigned j = 0; j < (mode < 2 ? 4u : 32u); ++j) data[i][j] = 10000 * (i + 1) + j;
        if (writable) {
            D3D12_UNORDERED_ACCESS_VIEW_DESC view = {0};
            view.Format = DXGI_FORMAT_R32_TYPELESS; view.ViewDimension = D3D12_UAV_DIMENSION_BUFFER;
            view.Buffer.FirstElement = mode < 2 ? 0 : 10; view.Buffer.NumElements = 4; view.Buffer.Flags = D3D12_BUFFER_UAV_FLAG_RAW;
            SLOT(dev,0x98,CreateUav)(dev, resources[i], NULL, &view, handle + i * stride);
        } else {
            D3D12_SHADER_RESOURCE_VIEW_DESC view = {0};
            view.Format = DXGI_FORMAT_R32_TYPELESS; view.ViewDimension = D3D12_SRV_DIMENSION_BUFFER;
            view.Shader4ComponentMapping = D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
            view.Buffer.FirstElement = 4; view.Buffer.NumElements = 4; view.Buffer.Flags = D3D12_BUFFER_SRV_FLAG_RAW;
            SLOT(dev,0x90,CreateSrv)(dev, resources[i], &view, handle + i * stride);
        }
    }
    if (mode == 1) {
        rd.Width = 256; rd.Flags = D3D12_RESOURCE_FLAG_NONE;
        if (make(dev, &hp, 0, &rd, D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER, NULL, &IID_ID3D12Resource, &cbv) != S_OK || !cbv ||
            SLOT(cbv,0x40,Map)(cbv, 0, NULL, (void **)&cbv_data) != S_OK || !cbv_data) return 6;
        cbv_data[4] = 7;
    }
    ComputeDesc pd = {NULL, {shaders[mode][test].bytes, shaders[mode][test].size}};
    if (SLOT(dev,0x58,CreatePipeline)(dev, &pd, &IID_ID3D12PipelineState, &pipeline) != S_OK || !pipeline) return 7;
    ID3D12CommandAllocator *allocator = NULL;
    ID3D12GraphicsCommandList *list = NULL;
    ID3D12CommandQueue *queue = NULL;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev, D3D12_COMMAND_LIST_TYPE_COMPUTE, &IID_ID3D12CommandAllocator, (void **)&allocator) != S_OK ||
        ID3D12Device_CreateCommandList((ID3D12Device *)dev, 0, D3D12_COMMAND_LIST_TYPE_COMPUTE, allocator, (ID3D12PipelineState *)pipeline, &IID_ID3D12GraphicsCommandList, (void **)&list) != S_OK) return 8;
    ID3D12GraphicsCommandList_SetPipelineState(list, (ID3D12PipelineState *)pipeline);
    SLOT(list,0x100,SetHeaps)(list, 1, &heap);
    SLOT(list,0x118,SetTable)(list, mode == 3 ? 7 : 1, handle);
    if (mode == 1) SLOT(list,0x148,SetCbv)(list, 2, SLOT(cbv,0x58,Address)(cbv));
    SLOT(list,0x70,Dispatch)(list, 1, 1, 1);
    if (ID3D12GraphicsCommandList_Close(list) != S_OK) return 9;
    D3D12_COMMAND_QUEUE_DESC qd = {D3D12_COMMAND_LIST_TYPE_COMPUTE, 0, 0, 0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev, &qd, &IID_ID3D12CommandQueue, (void **)&queue) != S_OK) return 10;
    for (unsigned i = 0; i < count; ++i)
        for (unsigned j = 0; j < (mode < 2 ? 4u : 32u); ++j)
            if (data[i][j] != 10000 * (i + 1) + j) return 11;
    say("D3D12X bounds dispatch armed\r\n");
    ID3D12CommandList *submit[] = {(ID3D12CommandList *)list};
    ID3D12CommandQueue_ExecuteCommandLists(queue, 1, submit);
    if (test != 0) { say("D3D12X unsafe dispatch returned\r\n"); return 12; }
    for (unsigned i = 0; i < count; ++i) {
        for (unsigned j = 0; j < (mode < 2 ? 4u : 32u); ++j) {
            unsigned expected = 10000 * (i + 1) + j;
            if (mode < 2 && i == 0 && j == 0) expected = 10001 + (mode == 1 ? 7 : 0);
            if (mode == 2 && i == 1 && j == 10) expected = 10005;
            if (mode == 3 && i == 0 && j == 10) expected = 20005;
            if (mode == 3 && i == 2 && j == 11) expected = 40006;
            if (data[i][j] != expected) return 13;
        }
    }
    if (tls_value != 29) return 14;
    say("D3D12X bounds valid passed\r\n"); return 42;
}
