/* Synthetic Win64 PE exercising indexed draws through translated shaders. */
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
    float viewport[6]={0,0,5,4,0,1}; int scissor[4]={0,0,5,4};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
    SLOT(list,0x180,Vertices)(list,0,1,view); SLOT(list,0x190,Targets)(list,1,&handle,0,0);
    SLOT(list,0x130,SetConstant)(list,3,0x3e800000,2);
}

unsigned mainCRTStartup(void) {
    if (*relocated != 7) return 99;
    unsigned version=0x00020004; void *dev=0,*vb=0,*ib=0,*readback=0,*texture=0,*heap=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,96,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    CreateResource make=SLOT(dev,0xd8,CreateResource);
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=32; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&ib)!=S_OK) return 3;
    rd.Width=1024; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&readback)!=S_OK) return 4;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=5; rd.Height=4; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&texture)!=S_OK) return 5;
    void *v=0,*indices=0,*out=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(ib,0x40,Map)(ib,0,0,&indices)!=S_OK || SLOT(readback,0x40,Map)(readback,0,0,&out)!=S_OK) return 6;
    const float vertices[24]={-1,-1,0,1, 0,-1,0,1, -0.5,1,0,1, 0,-1,0,1, 1,-1,0,1, 0.5,1,0,1};
    for (unsigned i=0;i<24;i++) ((float *)v)[i]=vertices[i];
    for (unsigned i=0;i<32;i++) ((unsigned char *)indices)[i]=0xff;
    ((unsigned short *)indices)[3]=0; ((unsigned short *)indices)[4]=1; ((unsigned short *)indices)[5]=2;
    ((unsigned *)indices)[5]=4; ((unsigned *)indices)[6]=5; ((unsigned *)indices)[7]=6;
    for (unsigned i=0;i<1024;i++) ((unsigned char *)out)[i]=0x77;
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
#ifdef TRAP
    /* Slot 18 (CopyTiles) is deliberately unsupported, unlike indexed slot 13. */
    ((void (__stdcall *)(void *))(*(void ***)list)[18])(list);
#endif
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),96,16};
    U64 address=SLOT(ib,0x58,Handle)(ib);
    D3D12_INDEX_BUFFER_VIEW i16={address+4,8,DXGI_FORMAT_R16_UINT}, i32={address+16,16,DXGI_FORMAT_R32_UINT};
    IndexBuffer set_index=SLOT(list,0x178,IndexBuffer); DrawIndexed draw=SLOT(list,0x68,DrawIndexed);
    /* Each invalid command is tested on a fresh list: Close must report failure. */
    for (unsigned test=0;test<6;test++) {
        ID3D12GraphicsCommandList *bad=0;
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&bad)!=S_OK) return 11;
        bind_graphics(bad,&view,handle);
        D3D12_INDEX_BUFFER_VIEW bad_view=i16;
        if (test==1) bad_view.Format=DXGI_FORMAT_R8_UINT;
        if (test==5) bad_view.BufferLocation++;
        set_index(bad,test==4?0:&bad_view);
        draw(bad,3,test==2?2:1,test==0?2:1,0,test==3?1:0);
        if (ID3D12GraphicsCommandList_Close(bad)!=E_NOTIMPL) return 12+test;
        SLOT(bad,0x10,Release)(bad);
    }
    float background[4]={0.25,0.5,1,0}; SLOT(list,0x1a0,Clear)(list,handle,background,0,0);
    bind_graphics(list,&view,handle);
    set_index(list,&i16); draw(list,3,1,1,0,0);
    SLOT(list,0x130,SetConstant)(list,3,0x3f400000,2);
    set_index(list,&i32); draw(list,3,1,1,-1,0);
    Barrier barrier={0,0,texture,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&barrier);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=readback; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=5; dst.PlacedFootprint.Footprint.Height=4; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=texture; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 16;
    if (((volatile unsigned char *)out)[2*256+4]!=0x77) return 17;
#ifdef BAD_INDEX
    /* Index contents are read at submission; this exceeds the vertex view. */
    ((unsigned short *)indices)[3]=99;
#endif
#ifdef BAD_BASE
    /* The second draw has base -1, so index zero underflows. */
    ((unsigned *)indices)[5]=0;
#endif
    /* The recorded indexed draws must keep the index resource alive. */
    if (SLOT(ib,0x10,Release)(ib)==0) return 18;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 19;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    volatile unsigned char *p=out; unsigned a=2*256+4,b=2*256+12,c=8;
    if (p[a]!=64 || p[a+1]!=0 || p[a+2]!=0 || p[a+3]!=255) return 20;
    if (p[b]!=191 || p[b+1]!=0 || p[b+2]!=0 || p[b+3]!=255) return 21;
    if (p[c]!=64 || p[c+1]!=128 || p[c+2]!=255 || p[c+3]!=0 || p[20]!=0x77) return 22;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator); SLOT(pipeline,0x10,Release)(pipeline); SLOT(heap,0x10,Release)(heap); SLOT(texture,0x10,Release)(texture); SLOT(readback,0x10,Release)(readback); SLOT(vb,0x10,Release)(vb); SLOT(dev,0x10,Release)(dev);
    return 42;
}
