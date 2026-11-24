/* Win64 PE: ordered RGBA8 blending and channel masks through translated shaders. */
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
typedef void (__stdcall *View)(void *, void *, const void *, U64);
typedef void (__stdcall *Draw)(void *, unsigned, unsigned, unsigned, unsigned);
typedef void (__stdcall *Constant)(void *, unsigned, unsigned, unsigned);
typedef void (__stdcall *Topology)(void *, unsigned);
typedef void (__stdcall *Array)(void *, unsigned, const void *);
typedef void (__stdcall *Vertices)(void *, unsigned, unsigned, const void *);
typedef void (__stdcall *Targets)(void *, unsigned, const U64 *, int, const U64 *);
typedef void (__stdcall *ClearColor)(void *, U64, const float *, unsigned, const void *);
typedef void (__stdcall *SetPipeline)(void *, void *);
typedef void (__stdcall *Copy)(void *, const void *, unsigned, unsigned, unsigned, const void *, const void *);
typedef unsigned (__stdcall *BarrierFn)(void *, unsigned, const void *);
typedef unsigned (__stdcall *Release)(void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind, flags; void *resource; U64 data[2]; } Barrier;
static int marker=7; static int *volatile relocated=&marker;
/* The freestanding guest has no CRT; Clang may lower zero initialization here. */
void *memset(void *dst, int value, size_t len) {
    volatile unsigned char *p=dst;
    for (size_t i=0;i<len;i++) p[i]=(unsigned char)value;
    return dst;
}

/* Blend begins at 168; the complete subobject, including its type, is 336 bytes. */
static void stream_init(unsigned char *b, const D3D12_INPUT_ELEMENT_DESC *element) {
    for (unsigned i=0;i<560;i++) b[i]=0;
    *(unsigned *)(b+0)=1; *(const void **)(b+8)=vs; *(U64 *)(b+16)=sizeof(vs);
    *(unsigned *)(b+24)=2; *(const void **)(b+32)=ps; *(U64 *)(b+40)=sizeof(ps);
    *(unsigned *)(b+48)=10; *(unsigned *)(b+52)=3; *(unsigned *)(b+56)=1; *(unsigned *)(b+76)=1;
    *(unsigned *)(b+96)=12; *(const void **)(b+104)=element; *(unsigned *)(b+112)=1;
    *(unsigned *)(b+120)=14; *(unsigned *)(b+124)=3;
    *(unsigned *)(b+128)=15; *(unsigned *)(b+132)=28; *(unsigned *)(b+164)=1;
    *(unsigned *)(b+168)=8;
    *(unsigned *)(b+180)=1; /* blend enable */
    *(unsigned *)(b+188)=5; *(unsigned *)(b+192)=6; *(unsigned *)(b+196)=1;
    *(unsigned *)(b+200)=2; *(unsigned *)(b+204)=6; *(unsigned *)(b+208)=1;
    b[216]=15;
    *(unsigned *)(b+504)=11; /* depth and stencil disabled */
}
static void setup_graphics(ID3D12GraphicsCommandList *l, const D3D12_VERTEX_BUFFER_VIEW *v, U64 rtv) {
    float viewport[6]={0,0,8,8,0,1}; int scissor[4]={0,0,8,8};
    SLOT(l,0xa0,Topology)(l,4); SLOT(l,0xa8,Array)(l,1,viewport); SLOT(l,0xb0,Array)(l,1,scissor);
    SLOT(l,0x180,Vertices)(l,0,1,v); SLOT(l,0x190,Targets)(l,1,&rtv,0,0);
}
static void draw_color(ID3D12GraphicsCommandList *l, int left, int top, int right, int bottom, const unsigned *color) {
    int scissor[4]={left,top,right,bottom};
    SLOT(l,0xb0,Array)(l,1,scissor);
    for (unsigned i=0;i<4;i++) SLOT(l,0x130,Constant)(l,3,color[i],i);
    SLOT(l,0x60,Draw)(l,3,1,0,0);
}
static void copy_color(ID3D12GraphicsCommandList *l, void *target, void *out) {
    Barrier barrier={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}};
    SLOT(l,0xd0,BarrierFn)(l,1,&barrier);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(l,0x80,Copy)(l,&dst,0,0,0,&src,0);
    barrier.data[0]=0xffffffffULL | (0x800ULL<<32); barrier.data[1]=4;
    SLOT(l,0xd0,BarrierFn)(l,1,&barrier);
}
static int pixels(volatile unsigned char *p) {
    /* Allow UNORM conversion rounding at each draw. */
    const int expected[8][4]={{80,143,16,207},{143,80,16,207},{80,143,16,112},
        {255,64,64,64},{64,128,64,64},{64,64,191,64},{64,64,64,128},{64,64,64,64}};
    const int opaque[4]={255,128,191,128};
    for (unsigned y=0;y<8;y++) {
        for (unsigned x=0;x<8;x++) for (unsigned c=0;c<4;c++) {
            int want=(x==0 && y<2)?opaque[c]:expected[x][c];
            int diff=(int)p[y*256+x*4+c]-want;
            if (diff < -2 || diff > 2) return 0;
        }
        for (unsigned x=32;x<256;x++) if (p[y*256+x]!=0x77) return 0;
    }
    return 1;
}
unsigned mainCRTStartup(void) {
    if (*relocated!=7) return 99;
    unsigned version=0x00020004; void *dev=0,*vb=0,*out=0,*target=0,*heap=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,48,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=2048; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&out)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 4;
    void *v=0,*o=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&o)!=S_OK) return 5;
    const float tri[12]={-1,-1,0,1, 3,-1,0,1, -1,3,0,1};
    for (unsigned i=0;i<12;i++) ((float *)v)[i]=tri[i];
    for (unsigned i=0;i<2048;i++) ((unsigned char *)o)[i]=0x77;
    HeapDesc hd={{2,1,0,0},{0,0}};
    if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heap)!=S_OK) return 6;
    U64 rtv=SLOT(heap,0x48,Handle)(heap); SLOT(dev,0xa0,View)(dev,target,0,rtv);
    D3D12_INPUT_ELEMENT_DESC element={"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0};
    union { U64 align; unsigned char b[560]; } stream;
    StreamDesc sd={sizeof(stream.b),stream.b}; void *pipelines[8]={0};
    stream_init(stream.b,&element);
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipelines[0])!=S_OK) return 7;
    *(unsigned *)(stream.b+200)=5;
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipelines[1])!=S_OK) return 8;
    /* Disabled blending ignores factor/op values; independent blending is off,
       so even nonsensical RT1 fields must not affect RT0. */
    *(unsigned *)(stream.b+180)=0;
    for (unsigned off=188;off<=212;off+=4) *(unsigned *)(stream.b+off)=0xffffffffu;
    for (unsigned off=220;off<504;off++) stream.b[off]=0xff;
    for (unsigned i=0;i<5;i++) {
        stream.b[216]=i==4?0:(1u<<i);
        if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipelines[2+i])!=S_OK) return 9;
    }
    stream_init(stream.b,&element); *(unsigned *)(stream.b+168)=11; sd.size=224;
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipelines[7])!=S_OK) return 10;
    sd.size=sizeof(stream.b);
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 11;
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),48,16};
    const unsigned red[4]={0x3f800000,0,0,0x3f000000};
    const unsigned green[4]={0,0x3f800000,0,0x3f000000};
    const unsigned mixed[4]={0x3f800000,0x3f000000,0x3f400000,0x3f000000};
    /* Unsupported blend state is reported by Close after a draw captures it. */
    for (unsigned test=0;test<12;test++) {
        void *bad_pipeline=0; ID3D12GraphicsCommandList *bad=0;
        stream_init(stream.b,&element);
        if (test==0) *(unsigned *)(stream.b+172)=1; /* alpha-to-coverage */
        if (test==1) *(unsigned *)(stream.b+176)=1; /* independent */
        if (test==2) *(unsigned *)(stream.b+184)=1; /* logic enable */
        if (test==3) *(unsigned *)(stream.b+188)=2;
        if (test==4) *(unsigned *)(stream.b+192)=2;
        if (test==5) *(unsigned *)(stream.b+196)=2;
        if (test==6) *(unsigned *)(stream.b+200)=6;
        if (test==7) *(unsigned *)(stream.b+204)=2;
        if (test==8) *(unsigned *)(stream.b+208)=2;
        if (test==9) stream.b[216]=16;
        if (test==10) *(unsigned *)(stream.b+180)=2;
        if (test==11) *(unsigned *)(stream.b+184)=2;
        if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&bad_pipeline)!=S_OK) return 12;
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,bad_pipeline,&IID_ID3D12GraphicsCommandList,(void **)&bad)!=S_OK) return 13;
        setup_graphics(bad,&view,rtv); draw_color(bad,0,0,8,8,red);
        if (ID3D12GraphicsCommandList_Close(bad)!=E_NOTIMPL) return 100+test;
        SLOT(bad,0x10,Release)(bad); SLOT(bad_pipeline,0x10,Release)(bad_pipeline);
    }
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipelines[0],&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 14;
    const float clear[4]={0.25f,0.25f,0.25f,0.25f};
    SLOT(list,0x1a0,ClearColor)(list,rtv,clear,0,0); setup_graphics(list,&view,rtv);
    draw_color(list,0,0,1,8,red); draw_color(list,0,0,1,8,green);
    draw_color(list,1,0,2,8,green); draw_color(list,1,0,2,8,red);
    SLOT(list,0xc8,SetPipeline)(list,pipelines[1]);
    draw_color(list,2,0,3,8,red); draw_color(list,2,0,3,8,green);
    for (unsigned i=0;i<5;i++) {
        SLOT(list,0xc8,SetPipeline)(list,pipelines[2+i]); draw_color(list,3+i,0,4+i,8,mixed);
    }
    SLOT(list,0xc8,SetPipeline)(list,pipelines[7]); draw_color(list,0,0,1,2,mixed);
    copy_color(list,target,out);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 15;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 16;
    /* All pipeline changes and referenced resources survive release before execution. */
    for (unsigned i=0;i<8;i++) SLOT(pipelines[i],0x10,Release)(pipelines[i]);
    SLOT(heap,0x10,Release)(heap); SLOT(target,0x10,Release)(target); SLOT(vb,0x10,Release)(vb); SLOT(out,0x10,Release)(out);
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    if (!pixels(o)) return 17;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev);
    return 42;
}
