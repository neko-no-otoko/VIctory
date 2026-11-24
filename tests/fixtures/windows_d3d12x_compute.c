/* Self-authored synthetic console shader and D3D12.X compute guest. The
 * compressed FSSM container represents the shader in metal_tests.rs: load
 * UAV word 1, add root constant s0, and store UAV word 0. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
static const unsigned char shader[] = {
  0x44,0x58,0x42,0x43,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
  0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x01,0x00,0x00,0x00,
  0xee,0x00,0x00,0x00,0x01,0x00,0x00,0x00,0x24,0x00,0x00,0x00,
  0x44,0x58,0x49,0x4c,0xc2,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
  0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
  0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x58,0x44,0x41,0x54,
  0x01,0x00,0x00,0x00,0x24,0x00,0x00,0x00,0x0d,0x00,0x00,0x00,
  0x30,0x00,0x00,0x00,0x92,0x00,0x00,0x00,0x58,0x42,0x43,0x48,
  0x03,0x00,0x00,0x00,0x7a,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
  0x70,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x78,0x9c,0x73,0x0b,
  0x0e,0xf6,0x65,0x64,0x64,0x60,0x48,0x00,0x62,0x10,0x60,0x83,
  0x50,0x0c,0xef,0xf7,0xad,0xbd,0x47,0x0a,0x66,0x40,0x03,0xb8,
  0xc4,0x33,0xa0,0xf4,0x1f,0x28,0xad,0x81,0x24,0xa7,0x80,0xae,
  0x18,0x08,0x54,0xa0,0xee,0xf2,0x81,0xd2,0x4c,0x40,0xcc,0x88,
  0x45,0x1d,0x07,0x16,0x31,0x6c,0x80,0x0b,0xaa,0x1f,0x86,0x91,
  0x41,0x03,0x73,0xeb,0x3e,0x26,0x26,0x8e,0x2f,0x40,0xe6,0x2f,
  0x16,0x06,0x83,0x07,0x40,0xdb,0x1a,0x0a,0x18,0x7a,0xf6,0x03,
  0xf9,0x5e,0x0c,0x0c,0x05,0x60,0x3e,0x03,0x43,0xe3,0x7e,0x98,
  0x7a,0x64,0xfd,0xcc,0x68,0x62,0x2c,0x40,0x06,0x33,0x33,0x03,
  0x0a,0x60,0x01,0x0a,0x02,0x00,0xfd,0xdf,0x3b,0x6f
};
static void print(const char *s, DWORD n) { DWORD w; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), s, n, &w, 0); }
#define SAY(s) print(s, sizeof(s)-1)
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { void *root; struct { const void *code; U64 size; } stage; } ComputeDesc;
typedef HRESULT (__stdcall *CreateHeap)(void *, const void *, const IID *, void **);
typedef U64 (__stdcall *Handle)(void *);
typedef HRESULT (__stdcall *CreateResource)(void *, const void *, unsigned, const void *, unsigned, const void *, const IID *, void **);
typedef HRESULT (__stdcall *Map)(void *, unsigned, const void *, void **);
typedef void (__stdcall *CreateUav)(void *, void *, void *, const void *, U64);
typedef HRESULT (__stdcall *CreatePipeline)(void *, const void *, const IID *, void **);
typedef void (__stdcall *SetHeaps)(void *, unsigned, void **);
typedef void (__stdcall *SetTable)(void *, unsigned, U64);
typedef void (__stdcall *SetConstants)(void *, unsigned, unsigned, const unsigned *, unsigned);
typedef void (__stdcall *Dispatch)(void *, unsigned, unsigned, unsigned);
unsigned mainCRTStartup(void *unused) {
    (void)unused; unsigned params=0x00020004; ID3D12Device *dev=0;
    if (D3D12XboxCreateDevice(0,&params,&IID_ID3D12Device,(void **)&dev)!=S_OK || !dev) { SAY("device failed\r\n"); return 1; }
    HeapDesc hd = {{D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,1,D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,0},{0,0}};
    void *heap=0;
    if (SLOT(dev,0x70,CreateHeap)(dev,&hd,&IID_ID3D12DescriptorHeap,&heap)!=S_OK || !heap) { SAY("heap failed\r\n"); return 2; }
    U64 handle = SLOT(heap,0x48,Handle)(heap);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,64,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS};
    void *resource=0;
    if (SLOT(dev,0xd8,CreateResource)(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&resource)!=S_OK || !resource) { SAY("resource failed\r\n"); return 3; }
    void *memory=0;
    if (SLOT(resource,0x40,Map)(resource,0,0,&memory)!=S_OK || !memory) { SAY("map failed\r\n"); return 4; }
    ((volatile unsigned *)memory)[0]=7; ((volatile unsigned *)memory)[1]=1;
    D3D12_UNORDERED_ACCESS_VIEW_DESC uav = {0};
    uav.Format = DXGI_FORMAT_R32_TYPELESS;
    uav.ViewDimension = D3D12_UAV_DIMENSION_BUFFER;
    uav.Buffer.NumElements = 16;
    uav.Buffer.Flags = D3D12_BUFFER_UAV_FLAG_RAW;
    SLOT(dev,0x98,CreateUav)(dev,resource,0,&uav,handle);
    ComputeDesc pd={0,{shader,sizeof(shader)}};
    void *pipeline=0;
    if (SLOT(dev,0x58,CreatePipeline)(dev,&pd,&IID_ID3D12PipelineState,&pipeline)!=S_OK || !pipeline) { SAY("pipeline failed\r\n"); return 5; }
    ID3D12CommandAllocator *allocator=0;
    if (ID3D12Device_CreateCommandAllocator(dev,D3D12_COMMAND_LIST_TYPE_COMPUTE,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) { SAY("allocator failed\r\n"); return 6; }
    ID3D12GraphicsCommandList *list=0;
    if (ID3D12Device_CreateCommandList(dev,0,D3D12_COMMAND_LIST_TYPE_COMPUTE,allocator,(ID3D12PipelineState *)pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) { SAY("list failed\r\n"); return 7; }
    ID3D12GraphicsCommandList_SetPipelineState(list, (ID3D12PipelineState *)pipeline);
    SLOT(list,0x100,SetHeaps)(list,1,&heap);
    SLOT(list,0x118,SetTable)(list,1,handle);
    unsigned constants[4]={41,0,0,0};
    SLOT(list,0x138,SetConstants)(list,0,4,constants,0);
    SLOT(list,0x70,Dispatch)(list,1,1,1);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) { SAY("close failed\r\n"); return 8; }
    D3D12_COMMAND_QUEUE_DESC qd={D3D12_COMMAND_LIST_TYPE_COMPUTE,0,0,0}; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandQueue(dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) { SAY("queue failed\r\n"); return 9; }
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list};
    ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    if (((volatile unsigned *)memory)[0]!=42) {
        if (((volatile unsigned *)memory)[0]==7) SAY("compute readback unchanged 7\r\n");
        else SAY("compute readback unexpected value\r\n");
        return 10;
    }
    SAY("D3D12X synthetic compute readback 42\r\n"); return 42;
}
