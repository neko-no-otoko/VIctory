/* Synthetic D3D12.X graphics guest with compiler TLS. The shader containers
 * are self-authored as in the native_graphics integration test. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include "windows_d3d12x_graphics_shaders.h"
__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
static void print(const char *s, DWORD n) { DWORD w; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), s, n, &w, 0); }
#define SAY(s) print(s, sizeof(s)-1)
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef HRESULT (__stdcall *CreateResource)(void *, const void *, unsigned, const void *, unsigned, const void *, const IID *, void **);
typedef HRESULT (__stdcall *CreateObject)(void *, const void *, const IID *, void **);
typedef HRESULT (__stdcall *Map)(void *, unsigned, const void *, void **);
typedef U64 (__stdcall *Handle)(void *);
typedef void (__stdcall *Rtv)(void *, void *, const void *, U64);
typedef void (__stdcall *Draw)(void *, unsigned, unsigned, unsigned, unsigned);
typedef void (__stdcall *SetConstants)(void *, unsigned, unsigned, const unsigned *, unsigned);
typedef void (__stdcall *Topology)(void *, unsigned);
typedef void (__stdcall *SetArray)(void *, unsigned, const void *);
typedef void (__stdcall *Vertices)(void *, unsigned, unsigned, const void *);
typedef void (__stdcall *Targets)(void *, unsigned, const U64 *, int, const U64 *);
typedef void (__stdcall *Clear)(void *, U64, const float *, unsigned, const void *);
typedef void (__stdcall *Copy)(void *, const void *, unsigned, unsigned, unsigned, const void *, const void *);
typedef unsigned (__stdcall *BarrierFn)(void *, unsigned, const void *);
typedef unsigned (__stdcall *Release)(void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind, flags; void *resource; U64 data[2]; } Barrier;

static _Thread_local int tls_value = 17;
static volatile LONG tls_attach, tls_detach, tls_bad, ready;
static HANDLE gate;
static void NTAPI tls_callback(PVOID image, DWORD reason, PVOID reserved) {
    (void)image; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        if (tls_value != 17) InterlockedIncrement(&tls_bad);
        if (reason == DLL_THREAD_ATTACH) InterlockedIncrement(&tls_attach);
    } else if (reason == DLL_THREAD_DETACH) InterlockedIncrement(&tls_detach);
}
__attribute__((used, section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK tls_callback_slot = tls_callback;
static DWORD WINAPI tls_worker(void *arg) {
    int value = (int)(INT_PTR)arg;
    if (tls_value != 17) return 1;
    tls_value = value;
    InterlockedIncrement(&ready);
    if (WaitForSingleObject(gate, 5000) != WAIT_OBJECT_0) return 2;
    return tls_value == value ? 0 : 3;
}
unsigned mainCRTStartup(void *unused) {
    (void)unused;
    HANDLE threads[2]; DWORD codes[2];
    if (tls_value != 17 || tls_bad) { SAY("TLS initial state failed\r\n"); return 20; }
    tls_value = 99;
    gate = CreateEventW(NULL, TRUE, FALSE, NULL);
    if (!gate) return 21;
    threads[0] = CreateThread(NULL, 0, tls_worker, (void *)(INT_PTR)111, 0, NULL);
    threads[1] = CreateThread(NULL, 0, tls_worker, (void *)(INT_PTR)222, 0, NULL);
    if (!threads[0] || !threads[1]) return 22;
    ULONGLONG deadline = GetTickCount64() + 5000;
    while (InterlockedCompareExchange(&ready, 0, 0) != 2 && GetTickCount64() < deadline) Sleep(1);
    if (ready != 2 || tls_value != 99 || !SetEvent(gate)) return 23;
    if (WaitForMultipleObjects(2, threads, TRUE, 5000) != WAIT_OBJECT_0) return 24;
    if (!GetExitCodeThread(threads[0], &codes[0]) || !GetExitCodeThread(threads[1], &codes[1])) return 25;
    CloseHandle(threads[0]); CloseHandle(threads[1]); CloseHandle(gate);
    if (codes[0] || codes[1] || tls_attach < 2 || tls_detach < 2 || tls_bad || tls_value != 99) {
        SAY("TLS thread state failed\r\n"); return 26;
    }
    SAY("TLS compiler state and thread callbacks passed\r\n");

    unsigned version=0x00020004; void *dev=0, *vb=0, *readback=0, *texture=0, *heap=0, *pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,48,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    CreateResource make=SLOT(dev,0xd8,CreateResource);
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=1024;
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&readback)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=5; rd.Height=4; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&texture)!=S_OK) return 4;
    void *v=0,*out=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(readback,0x40,Map)(readback,0,0,&out)!=S_OK) return 5;
    const float vertices[12]={-1,-1,0,1, 0,-1,0,1, -0.5,1,0,1};
    for (unsigned i=0;i<12;i++) ((float *)v)[i]=vertices[i];
    for (unsigned i=0;i<1024;i++) ((unsigned char *)out)[i]=0x77;
    HeapDesc hd={{D3D12_DESCRIPTOR_HEAP_TYPE_RTV,1,0,0},{0,0}};
    if (SLOT(dev,0x70,CreateObject)(dev,&hd,&IID_ID3D12DescriptorHeap,&heap)!=S_OK) return 6;
    U64 handle=SLOT(heap,0x48,Handle)(heap);
    SLOT(dev,0xa0,Rtv)(dev,texture,0,handle);
    D3D12_INPUT_ELEMENT_DESC element={"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0};
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=&element; *(unsigned *)(stream.b+168)=1;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    StreamDesc sd={sizeof(stream.b),stream.b};
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 7;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 8;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 9;
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),48,16};
    float viewport[6]={0,0,5,4,0,1}; int scissor[4]={0,0,5,4}; float background[4]={0.25,0.5,1,0};
    SLOT(list,0x1a0,Clear)(list,handle,background,0,0);
#ifdef INVALID_GRAPHICS_TOPOLOGY
    SLOT(list,0xa0,Topology)(list,2);
#else
    SLOT(list,0xa0,Topology)(list,4);
#endif
    SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
    SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&handle,0,0);
    unsigned constants[3]={0,0,0x3f400000};
    SLOT(list,0x140,SetConstants)(list,3,3,constants,0); SLOT(list,0x60,Draw)(list,3,1,0,0);
    Barrier barrier={0,0,texture,{0xffffffffULL | (4ULL<<32),0x800}};
    SLOT(list,0xd0,BarrierFn)(list,1,&barrier);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=readback; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=5; dst.PlacedFootprint.Footprint.Height=4; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=texture; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
    SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) { SAY("D3D12X graphics command list rejected\r\n"); return 10; }
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 11;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    volatile unsigned char *p=out; unsigned a=2*256+1*4,c=2*4;
    if (p[a]!=191 || p[a+1]!=0 || p[a+2]!=0 || p[a+3]!=255) return 12;
    if (p[c]!=64 || p[c+1]!=128 || p[c+2]!=255 || p[c+3]!=0) return 13;
    for (unsigned row=0;row<4;row++) for (unsigned col=20;col<256;col++) if (p[row*256+col]!=0x77) return 14;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator); SLOT(pipeline,0x10,Release)(pipeline); SLOT(heap,0x10,Release)(heap); SLOT(texture,0x10,Release)(texture); SLOT(readback,0x10,Release)(readback); SLOT(vb,0x10,Release)(vb); SLOT(dev,0x10,Release)(dev);
    SAY("D3D12X synthetic graphics pixels and padding passed\r\n"); return 42;
}
