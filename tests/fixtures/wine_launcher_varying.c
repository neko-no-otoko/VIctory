/* Normally launched Wine guest: multiple independently linked varyings. */
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
typedef HRESULT (__stdcall *CreateResource)(void *, const void *, unsigned, const void *, unsigned, const void *, const IID *, void **);
typedef HRESULT (__stdcall *CreateObject)(void *, const void *, const IID *, void **);
typedef HRESULT (__stdcall *Map)(void *, unsigned, const void *, void **);
typedef U64 (__stdcall *Handle)(void *);
typedef void (__stdcall *View)(void *, void *, const void *, U64);
typedef void (__stdcall *Draw)(void *, unsigned, unsigned, unsigned, unsigned);
typedef void (__stdcall *Topology)(void *, unsigned);
typedef void (__stdcall *SetArray)(void *, unsigned, const void *);
typedef void (__stdcall *Vertices)(void *, unsigned, unsigned, const void *);
typedef void (__stdcall *Targets)(void *, unsigned, const U64 *, int, const U64 *);
typedef void (__stdcall *Copy)(void *, const void *, unsigned, unsigned, unsigned, const void *, const void *);
typedef unsigned (__stdcall *BarrierFn)(void *, unsigned, const void *);
typedef unsigned (__stdcall *Release)(void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind, flags; void *resource; U64 data[2]; } Barrier;
static _Thread_local int tls_value = 17;
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),s,(DWORD)strlen(s),&n,0); }

int main(int argc, char **argv) {
    int mismatch = argc == 2 && !strcmp(argv[1], "--mismatch");
    int flat = argc == 2 && !strcmp(argv[1], "--flat");
    int reordered = argc == 2 && !strcmp(argv[1], "--reordered");
    int wide = argc == 2 && !strcmp(argv[1], "--wide");
    if (argc != 1 && !mismatch && !flat && !reordered && !wide) return 90;
    if (tls_value != 17) return 91;
    tls_value = 29;
    unsigned version=0x00020004; void *dev=0,*vb=0,*out=0,*target=0,*heap=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,72,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=2048; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM;
    rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 4;
    void *v=0,*p=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK) return 5;
    /* Clip-space position.xy is TEXCOORD2; distinct w exercises perspective.
       COLOR1 is independently sourced from the vertex buffer. */
    const float tri[18]={-1,-1,0,1, .25f,.75f, 6,-2,0,2, .25f,.75f, -4,12,0,4, .25f,.75f};
    memcpy(v,tri,sizeof(tri)); memset(p,0x77,2048);
    if (wide) for (unsigned i=0;i<3;i++) ((float *)v)[6*i+2]=.5f;
    if (flat) {
        /* D3D nointerpolation uses the first vertex of the triangle. */
        ((float *)v)[10]=.875f; ((float *)v)[11]=.125f;
        ((float *)v)[16]=.625f; ((float *)v)[17]=.375f;
    }
    HeapDesc hd={{D3D12_DESCRIPTOR_HEAP_TYPE_RTV,1,0,0},{0,0}};
    if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heap)!=S_OK) return 6;
    U64 rtv=SLOT(heap,0x48,Handle)(heap); SLOT(dev,0xa0,View)(dev,target,0,rtv);
    D3D12_INPUT_ELEMENT_DESC elements[2]={
        {"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},
        {"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}
    };
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=wide?vs_wide:vs; *(U64 *)(stream.b+16)=wide?sizeof(vs_wide):sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=mismatch?ps_mismatch:flat?ps_flat:reordered?ps_reordered:wide?ps_wide:ps;
    *(U64 *)(stream.b+40)=mismatch?sizeof(ps_mismatch):flat?sizeof(ps_flat):reordered?sizeof(ps_reordered):wide?sizeof(ps_wide):sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    StreamDesc sd={sizeof(stream.b),stream.b};
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK || !pipeline) return 7;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 8;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 9;
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24};
    float viewport[6]={0,0,8,8,0,1}; int scissor[4]={0,0,8,8};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
    SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&rtv,0,0);
    SLOT(list,0x60,Draw)(list,3,1,0,0);
    Barrier barrier={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&barrier);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 11;
    if (((volatile unsigned char *)p)[3*256+16]!=0x77) return 12;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 13;
    if (mismatch) say("D3D12X varying mismatch armed\r\n");
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    volatile unsigned char *pixels=p;
    /* Perspective interpolation gives approximately (.19,.19) and
       (.72,1.19), while COLOR1 remains (.25,.75). */
    const unsigned offsets[2]={3*256+4*4,1*256+5*4};
    const int expected[2][4]={{48,48,64,191},{184,255,64,191}};
    const int wide_expected[4]={128,255,64,191};
    for (unsigned i=0;i<2;i++) for (unsigned c=0;c<4;c++) {
        int want=wide?wide_expected[c]:expected[i][c];
        int delta=(int)pixels[offsets[i]+c]-want;
        if (delta < -4 || delta > 4) {
            char message[128]; snprintf(message,sizeof(message),"varying pixel %u channel %u: %u expected %d\r\n",i,c,pixels[offsets[i]+c],want); say(message); return 20+i;
        }
    }
    if (pixels[32]!=0x77 || tls_value!=29) return 22;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator);
    SLOT(pipeline,0x10,Release)(pipeline); SLOT(heap,0x10,Release)(heap); SLOT(target,0x10,Release)(target);
    SLOT(out,0x10,Release)(out); SLOT(vb,0x10,Release)(vb); SLOT(dev,0x10,Release)(dev);
    say(flat ? "D3D12X flat varying passed\r\n" : reordered ? "D3D12X reordered varyings passed\r\n" : wide ? "D3D12X wide varying passed\r\n" : "D3D12X multiple varyings passed\r\n"); return 42;
}
