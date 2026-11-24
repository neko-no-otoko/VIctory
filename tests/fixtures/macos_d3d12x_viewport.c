/* Synthetic Win64 PE exercising viewport/scissor snapshots and clipping. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include "shaders.h"
__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef HRESULT (__stdcall *CreateResource)(void *, const void *, unsigned, const void *, unsigned, const void *, const IID *, void **);
typedef HRESULT (__stdcall *CreateObject)(void *, const void *, const IID *, void **);
typedef HRESULT (__stdcall *Map)(void *, unsigned, const void *, void **);
typedef U64 (__stdcall *Handle)(void *);
typedef void (__stdcall *Rtv)(void *, void *, const void *, U64);
typedef void (__stdcall *Draw)(void *, unsigned, unsigned, unsigned, unsigned);
typedef void (__stdcall *DrawIndexed)(void *, unsigned, unsigned, unsigned, int, unsigned);
typedef void (__stdcall *IndexBuffer)(void *, const D3D12_INDEX_BUFFER_VIEW *);
typedef void (__stdcall *SetConstant)(void *, unsigned, unsigned, unsigned);
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
static int marker=7; static int *volatile relocated=&marker;

static void bind_graphics(ID3D12GraphicsCommandList *list, const D3D12_VERTEX_BUFFER_VIEW *view, U64 handle) {
    SLOT(list,0xa0,Topology)(list,4);
    SLOT(list,0x180,Vertices)(list,0,1,view);
    SLOT(list,0x190,Targets)(list,1,&handle,0,0);
    SLOT(list,0x130,SetConstant)(list,3,0x3e800000,2);
}
static float bits(unsigned value) { union { unsigned u; float f; } v; v.u=value; return v.f; }
static int pixel(volatile unsigned char *p, unsigned x, unsigned y, unsigned r, unsigned g, unsigned b, unsigned a) {
    unsigned i=y*256+x*4;
    return p[i]==r && p[i+1]==g && p[i+2]==b && p[i+3]==a;
}
unsigned mainCRTStartup(void) {
    if (*relocated != 7) return 99;
    unsigned version=0x00020004; void *dev=0,*vb=0,*ib=0,*readback=0,*texture=0,*heap=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,48,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    CreateResource make=SLOT(dev,0xd8,CreateResource);
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=12; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&ib)!=S_OK) return 3;
    rd.Width=2048; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&readback)!=S_OK) return 4;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&texture)!=S_OK) return 5;
    void *v=0,*indices=0,*out=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(ib,0x40,Map)(ib,0,0,&indices)!=S_OK || SLOT(readback,0x40,Map)(readback,0,0,&out)!=S_OK) return 6;
    const float vertices[12]={-1,-1,0,1, 3,-1,0,1, -1,3,0,1};
    for (unsigned i=0;i<12;i++) ((float *)v)[i]=vertices[i];
    for (unsigned i=0;i<6;i++) ((unsigned short *)indices)[i]=(unsigned short)(i%3);
    for (unsigned i=0;i<2048;i++) ((unsigned char *)out)[i]=0x77;
    HeapDesc hd={{D3D12_DESCRIPTOR_HEAP_TYPE_RTV,1,0,0},{0,0}};
    if (SLOT(dev,0x70,CreateObject)(dev,&hd,&IID_ID3D12DescriptorHeap,&heap)!=S_OK) return 7;
    U64 handle=SLOT(heap,0x48,Handle)(heap); SLOT(dev,0xa0,Rtv)(dev,texture,0,handle);
    D3D12_INPUT_ELEMENT_DESC element={"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0};
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=&element; *(unsigned *)(stream.b+168)=1;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    StreamDesc sd={sizeof(stream.b),stream.b};
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 8;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 9;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 10;
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),48,16};
    D3D12_INDEX_BUFFER_VIEW index={SLOT(ib,0x58,Handle)(ib),6,DXGI_FORMAT_R16_UINT};
    /* Each invalid state is recorded on its own list and must fail Close. */
    for (unsigned test=0;test<18;test++) {
        ID3D12GraphicsCommandList *bad=0;
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&bad)!=S_OK) return 11;
        bind_graphics(bad,&view,handle);
        float vp[6]={0,0,8,8,0,1}; int sc[4]={0,0,8,8};
        switch (test) {
        case 0: vp[0]=bits(0x7fc00000); break;
        case 1: vp[1]=bits(0x7f800000); break;
        case 2: vp[2]=bits(0xff800000); break;
        case 3: vp[2]=0; break;
        case 4: vp[3]=0; break;
        case 5: vp[2]=-1; break;
        case 6: vp[3]=-1; break;
        case 7: vp[4]=0.75f; vp[5]=0.25f; break;
        case 8: vp[4]=-0.1f; break;
        case 9: vp[5]=1.1f; break;
        case 10: vp[4]=bits(0x7fc00000); break;
        case 11: vp[5]=bits(0x7f800000); break;
        case 12: sc[0]=5; sc[2]=4; break;
        case 13: sc[1]=5; sc[3]=4; break;
        case 14: vp[0]=-32769; break;
        case 15: vp[1]=-32769; break;
        case 16: vp[0]=32760; vp[2]=8; break;
        case 17: vp[1]=32760; vp[3]=8; break;
        }
        SLOT(bad,0xa8,SetArray)(bad,1,vp); SLOT(bad,0xb0,SetArray)(bad,1,sc);
        SLOT(bad,0x60,Draw)(bad,3,1,0,0);
        if (ID3D12GraphicsCommandList_Close(bad)!=E_NOTIMPL) return 20+test;
        SLOT(bad,0x10,Release)(bad);
    }
    float background[4]={0.25f,0.5f,1,0}; SLOT(list,0x1a0,Clear)(list,handle,background,0,0);
    bind_graphics(list,&view,handle); SLOT(list,0x178,IndexBuffer)(list,&index);
    float vp[6]={1,1,3,3,0.2f,0.8f}; int sc[4]={2,2,4,4};
    SLOT(list,0xa8,SetArray)(list,1,vp); SLOT(list,0xb0,SetArray)(list,1,sc);
    /* Reuse and mutate the caller's arrays before issuing each draw. */
    vp[0]=4; vp[1]=0; vp[2]=4; vp[3]=4; vp[4]=0; vp[5]=1;
    sc[0]=5; sc[1]=-2; sc[2]=10; sc[3]=3;
    SLOT(list,0x60,Draw)(list,3,1,0,0);
    SLOT(list,0xa8,SetArray)(list,1,vp); SLOT(list,0xb0,SetArray)(list,1,sc);
    SLOT(list,0x130,SetConstant)(list,3,0x3f000000,2);
    SLOT(list,0x68,DrawIndexed)(list,3,1,0,0,0);
    vp[0]=-2; vp[1]=4; vp[2]=4; vp[3]=4; vp[4]=0.5f; vp[5]=0.5f;
    sc[0]=-5; sc[1]=5; sc[2]=3; sc[3]=10;
    SLOT(list,0xa8,SetArray)(list,1,vp); SLOT(list,0xb0,SetArray)(list,1,sc);
    SLOT(list,0x130,SetConstant)(list,3,0x3f400000,2);
    SLOT(list,0x60,Draw)(list,3,1,0,0);
    vp[0]=6; vp[1]=6; vp[2]=4; vp[3]=4; vp[4]=0.1f; vp[5]=0.9f;
    sc[0]=6; sc[1]=6; sc[2]=12; sc[3]=12;
    SLOT(list,0xa8,SetArray)(list,1,vp); SLOT(list,0xb0,SetArray)(list,1,sc);
    SLOT(list,0x130,SetConstant)(list,3,0x3f800000,2);
    SLOT(list,0x68,DrawIndexed)(list,3,1,0,0,0);
    /* Both explicitly empty and entirely off-target scissors preserve pixels. */
    vp[0]=0; vp[1]=0; vp[2]=8; vp[3]=8; vp[4]=0; vp[5]=1;
    sc[0]=3; sc[1]=0; sc[2]=3; sc[3]=8;
    SLOT(list,0xa8,SetArray)(list,1,vp); SLOT(list,0xb0,SetArray)(list,1,sc);
    SLOT(list,0x60,Draw)(list,3,1,0,0);
    sc[0]=10; sc[1]=10; sc[2]=12; sc[3]=12;
    D3D12_INDEX_BUFFER_VIEW clipped_index={index.BufferLocation+6,6,DXGI_FORMAT_R16_UINT};
    SLOT(list,0x178,IndexBuffer)(list,&clipped_index);
    SLOT(list,0xb0,SetArray)(list,1,sc); SLOT(list,0x68,DrawIndexed)(list,3,1,0,0,0);
    /* Change state again after the draws, and overwrite the stack arrays. */
    sc[0]=0; sc[1]=0; sc[2]=8; sc[3]=8;
    SLOT(list,0xb0,SetArray)(list,1,sc);
    for (unsigned i=0;i<6;i++) vp[i]=0;
    for (unsigned i=0;i<4;i++) sc[i]=0;
    Barrier barrier={0,0,texture,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&barrier);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=readback; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=texture; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 34;
#ifdef BAD_CLIPPED_INDEX
    /* Even the fully clipped indexed draw must validate its vertex range. */
    ((unsigned short *)indices)[3]=99;
#endif
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 35;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    volatile unsigned char *p=out;
    for (unsigned y=0;y<8;y++) for (unsigned x=0;x<8;x++) {
        unsigned r=64,g=128,b=255,a=0;
        if (x>=2 && x<4 && y>=2 && y<4) { r=64; g=0; b=0; a=255; }
        if (x>=5 && y<3) { r=128; g=0; b=0; a=255; }
        if (x<2 && y>=5) { r=191; g=0; b=0; a=255; }
        if (x>=6 && y>=6) { r=255; g=0; b=0; a=255; }
        if (!pixel(p,x,y,r,g,b,a)) return 36;
    }
    if (p[32]!=0x77) return 37;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator); SLOT(pipeline,0x10,Release)(pipeline); SLOT(heap,0x10,Release)(heap); SLOT(texture,0x10,Release)(texture); SLOT(readback,0x10,Release)(readback); SLOT(ib,0x10,Release)(ib); SLOT(vb,0x10,Release)(vb); SLOT(dev,0x10,Release)(dev);
    return 42;
}
