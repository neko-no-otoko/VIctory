/* Synthetic console compute fixture. shaders.h is generated from RDNA2 and
 * FSSM metadata by native_cbv.rs; no title or pretranslated shader is used. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
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
static int marker=7; static int *volatile relocated=&marker;
unsigned mainCRTStartup(void) {
    if (*relocated != 7) return 99;
    unsigned version=0x00020004; void *dev=0, *heap=0, *cbv=0, *output=0, *pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    HeapDesc hd={{D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,1,D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,0},{0,0}};
    if (SLOT(dev,0x70,CreateHeap)(dev,&hd,&IID_ID3D12DescriptorHeap,&heap)!=S_OK || !heap) return 2;
    U64 handle=SLOT(heap,0x48,Handle)(heap);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,64,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    CreateResource make=SLOT(dev,0xd8,CreateResource);
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&cbv)!=S_OK || !cbv) return 3;
    rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&output)!=S_OK || !output) return 4;
    volatile unsigned *constants=0, *result=0;
    if (SLOT(cbv,0x40,Map)(cbv,0,0,(void **)&constants)!=S_OK || !constants) return 5;
    if (SLOT(output,0x40,Map)(output,0,0,(void **)&result)!=S_OK || !result) return 6;
    constants[0]=91; constants[4]=3; constants[8]=5; result[0]=7; result[1]=1;
    SLOT(dev,0x98,CreateUav)(dev,output,0,0,handle);
    ComputeDesc pd={0,{shader,sizeof(shader)}};
    if (SLOT(dev,0x58,CreatePipeline)(dev,&pd,&IID_ID3D12PipelineState,&pipeline)!=S_OK || !pipeline) return 7;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,D3D12_COMMAND_LIST_TYPE_COMPUTE,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 8;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,D3D12_COMMAND_LIST_TYPE_COMPUTE,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 9;
    ID3D12GraphicsCommandList_SetPipelineState(list,(ID3D12PipelineState *)pipeline);
    SLOT(list,0x100,SetHeaps)(list,1,&heap);
    SLOT(list,0x118,SetTable)(list,2,handle);
    U64 address=SLOT(cbv,0x58,Handle)(cbv);
#if CASE == 2
    address+=17; /* misaligned */
#elif CASE == 3
    address+=64; /* exactly one past the buffer */
#elif CASE == 7
    address+=48; /* only 16 bytes remain; the CBV load needs 32 */
#elif CASE == 8
    address=SLOT(output,0x58,Handle)(output); /* aliases the output UAV */
#else
    address+=16;
#endif
#if CASE == 4
    SLOT(list,0x148,SetView)(list,64,address); /* slot outside 0..63 */
#elif CASE == 5
    SLOT(list,0x158,SetView)(list,1,address); /* unsupported compute SRV */
#elif CASE == 6
    SLOT(list,0x168,SetView)(list,1,address); /* unsupported compute UAV */
#elif CASE != 1
    SLOT(list,0x148,SetView)(list,1,address);
#endif
    SLOT(list,0x70,Dispatch)(list,1,1,1);
    HRESULT close=ID3D12GraphicsCommandList_Close(list);
#if (CASE >= 1 && CASE <= 4) || CASE == 7 || CASE == 8
    if (close != E_NOTIMPL || result[0] != 7) return 10;
#else
    if (close != S_OK) return 11;
    /* Writes after command recording must be observed at execution, and the
     * recorded state must retain both objects after their guest releases. */
    constants[8]=41;
    SLOT(cbv,0x10,Release)(cbv); cbv=0;
    SLOT(heap,0x10,Release)(heap); heap=0;
    D3D12_COMMAND_QUEUE_DESC qd={D3D12_COMMAND_LIST_TYPE_COMPUTE,0,0,0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 12;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    if (result[0]!=42 || result[1]!=1) return 13;
    SLOT(queue,0x10,Release)(queue);
#endif
    SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator); SLOT(pipeline,0x10,Release)(pipeline);
    if (heap) SLOT(heap,0x10,Release)(heap);
    if (cbv) SLOT(cbv,0x10,Release)(cbv);
    SLOT(output,0x10,Release)(output); SLOT(dev,0x10,Release)(dev);
    return 42;
}
