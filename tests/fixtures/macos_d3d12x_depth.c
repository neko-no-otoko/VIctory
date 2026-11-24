/* Win64 PE: translated fullscreen triangles against a linear D32_FLOAT DSV. */
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
typedef void (__stdcall *DrawIndexed)(void *, unsigned, unsigned, unsigned, int, unsigned);
typedef void (__stdcall *IndexBuffer)(void *, const D3D12_INDEX_BUFFER_VIEW *);
typedef void (__stdcall *Constant)(void *, unsigned, unsigned, unsigned);
typedef void (__stdcall *Topology)(void *, unsigned);
typedef void (__stdcall *Array)(void *, unsigned, const void *);
typedef void (__stdcall *Vertices)(void *, unsigned, unsigned, const void *);
typedef void (__stdcall *Targets)(void *, unsigned, const U64 *, int, const U64 *);
typedef void (__stdcall *ClearDepth)(void *, U64, unsigned, float, unsigned char, unsigned, const void *);
typedef void (__stdcall *ClearColor)(void *, U64, const float *, unsigned, const void *);
typedef void (__stdcall *SetPipeline)(void *, void *);
typedef void (__stdcall *Copy)(void *, const void *, unsigned, unsigned, unsigned, const void *, const void *);
typedef unsigned (__stdcall *BarrierFn)(void *, unsigned, const void *);
typedef unsigned (__stdcall *Release)(void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind, flags; void *resource; U64 data[2]; } Barrier;
static int marker=7; static int *volatile relocated=&marker;

/* Stream offsets, including 8-byte padding: VS 0, PS 24, rasterizer 48,
   depth-stencil 96 (enable +4, write mask +8, compare +12, stencil +16),
   input 152, topology 176, RTV formats 184, DSV format 224. */
static void stream_init(unsigned char *b, const D3D12_INPUT_ELEMENT_DESC *element) {
    for (unsigned i=0;i<232;i++) b[i]=0;
    *(unsigned *)(b+0)=1; *(const void **)(b+8)=vs; *(U64 *)(b+16)=sizeof(vs);
    *(unsigned *)(b+24)=2; *(const void **)(b+32)=ps; *(U64 *)(b+40)=sizeof(ps);
    *(unsigned *)(b+48)=10; *(unsigned *)(b+52)=3; *(unsigned *)(b+56)=1; *(unsigned *)(b+76)=1;
    *(unsigned *)(b+96)=11; *(unsigned *)(b+100)=1; *(unsigned *)(b+104)=1; *(unsigned *)(b+108)=2;
    *(unsigned *)(b+152)=12; *(const void **)(b+160)=element; *(unsigned *)(b+168)=1;
    *(unsigned *)(b+176)=14; *(unsigned *)(b+180)=3;
    *(unsigned *)(b+184)=15; *(unsigned *)(b+188)=28; *(unsigned *)(b+220)=1;
    *(unsigned *)(b+224)=16; *(unsigned *)(b+228)=40;
}
static void bind_graphics(ID3D12GraphicsCommandList *l, const D3D12_VERTEX_BUFFER_VIEW *v, U64 rtv, U64 dsv) {
    float viewport[6]={0,0,8,8,0,1}; int scissor[4]={0,0,8,8};
    SLOT(l,0xa0,Topology)(l,4); SLOT(l,0xa8,Array)(l,1,viewport); SLOT(l,0xb0,Array)(l,1,scissor);
    SLOT(l,0x180,Vertices)(l,0,1,v); SLOT(l,0x190,Targets)(l,1,&rtv,0,&dsv);
    SLOT(l,0x130,Constant)(l,3,0x3e800000,2);
}
static void draw(ID3D12GraphicsCommandList *l, unsigned first, unsigned color, int left, int right) {
    int scissor[4]={left,0,right,8};
    SLOT(l,0xb0,Array)(l,1,scissor); SLOT(l,0x130,Constant)(l,3,color,2);
    SLOT(l,0x60,Draw)(l,3,1,first,0);
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
static int pixels(volatile unsigned char *p, volatile float *d, int phase) {
    for (unsigned y=0;y<8;y++) for (unsigned x=0;x<8;x++) {
        unsigned red; float depth;
        if (!phase) {
            red=x<2?64:x<4?191:x<6?255:128;
            depth=(x<2 || x>=6)?0.5f:0.25f;
        } else {
            red=x<4?128:x<6?255:128;
            depth=x<4?0.75f:x>=6?0.125f:0.875f;
        }
        unsigned at=y*256+x*4;
        if (p[at]!=red || p[at+1]!=0 || p[at+2]!=0 || p[at+3]!=255) return 0;
        if (d[y*64+x]!=depth) return 0;
    }
    return 1;
}
unsigned mainCRTStartup(void) {
    if (*relocated!=7) return 99;
    unsigned version=0x00020004; void *dev=0,*vb=0,*ib=0,*out=0,*target=0,*depth=0,*small=0,*rtvheap=0,*dsvheap=0,*smallheap=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,192,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=6; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&ib)!=S_OK) return 33;
    rd.Width=2048; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&out)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 4;
    rd.Format=DXGI_FORMAT_D32_FLOAT; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL;
    if (make(dev,&hp,0,&rd,0x10,0,&IID_ID3D12Resource,&depth)!=S_OK) return 5;
    rd.Width=4; if (make(dev,&hp,0,&rd,0x10,0,&IID_ID3D12Resource,&small)!=S_OK) return 6;
    void *v=0,*idx=0,*o=0,*d=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(ib,0x40,Map)(ib,0,0,&idx)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&o)!=S_OK || SLOT(depth,0x40,Map)(depth,0,0,&d)!=S_OK) return 7;
    const float z[4]={0.5f,0.75f,0.25f,0.125f};
    for (unsigned i=0;i<4;i++) {
        const float tri[12]={-1,-1,z[i],1, 3,-1,z[i],1, -1,3,z[i],1};
        for (unsigned j=0;j<12;j++) ((float *)v)[i*12+j]=tri[j];
    }
    ((unsigned short *)idx)[0]=6; ((unsigned short *)idx)[1]=7; ((unsigned short *)idx)[2]=8;
    for (unsigned i=0;i<2048;i++) { ((unsigned char *)o)[i]=0x77; ((unsigned char *)d)[i]=0; }
    HeapDesc hd={{2,1,0,0},{0,0}};
    if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&rtvheap)!=S_OK) return 8;
    hd.desc.Type=3;
    if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&dsvheap)!=S_OK || create(dev,&hd,&IID_ID3D12DescriptorHeap,&smallheap)!=S_OK) return 9;
    U64 rtv=SLOT(rtvheap,0x48,Handle)(rtvheap), dsv=SLOT(dsvheap,0x48,Handle)(dsvheap), small_dsv=SLOT(smallheap,0x48,Handle)(smallheap);
    SLOT(dev,0xa0,View)(dev,target,0,rtv); SLOT(dev,0xa8,View)(dev,depth,0,dsv); SLOT(dev,0xa8,View)(dev,small,0,small_dsv);
    D3D12_INPUT_ELEMENT_DESC element={"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0};
    union { U64 align; unsigned char b[232]; } stream;
    stream_init(stream.b,&element); StreamDesc sd={sizeof(stream.b),stream.b}; void *write_pipeline=0,*no_write=0;
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&write_pipeline)!=S_OK) return 10;
    *(unsigned *)(stream.b+104)=0;
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&no_write)!=S_OK) return 11;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *first=0,*second=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 12;
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),192,16};
    D3D12_INDEX_BUFFER_VIEW index={SLOT(ib,0x58,Handle)(ib),6,DXGI_FORMAT_R16_UINT};
    /* Unsupported pipeline fields must poison the list, not silently disable depth. */
    for (unsigned test=0;test<4;test++) {
        void *bad_pipeline=0; ID3D12GraphicsCommandList *bad=0;
        stream_init(stream.b,&element);
        if (test==0) *(unsigned *)(stream.b+228)=45; /* D24S8 */
        if (test==1) *(unsigned *)(stream.b+112)=1; /* stencil enabled */
        if (test==2) *(unsigned *)(stream.b+104)=2; /* invalid write mask */
        if (test==3) *(unsigned *)(stream.b+108)=9; /* invalid comparison */
        if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&bad_pipeline)!=S_OK) return 13;
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,bad_pipeline,&IID_ID3D12GraphicsCommandList,(void **)&bad)!=S_OK) return 14;
        bind_graphics(bad,&view,rtv,dsv); SLOT(bad,0x60,Draw)(bad,3,1,0,0);
        if (ID3D12GraphicsCommandList_Close(bad)!=E_NOTIMPL) return 15+test;
        SLOT(bad,0x10,Release)(bad); SLOT(bad_pipeline,0x10,Release)(bad_pipeline);
    }
    for (unsigned test=0;test<11;test++) {
        ID3D12GraphicsCommandList *bad=0;
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,write_pipeline,&IID_ID3D12GraphicsCommandList,(void **)&bad)!=S_OK) return 19;
        if (test==5) { bind_graphics(bad,&view,rtv,small_dsv); SLOT(bad,0x60,Draw)(bad,3,1,0,0); }
        else if (test==6) {
            Barrier barrier={0,0,depth,{0xffffffffULL | (0x10ULL<<32),0x20}};
            SLOT(bad,0xd0,BarrierFn)(bad,1,&barrier);
        } else if (test>=8) {
            D3D12_DEPTH_STENCIL_VIEW_DESC bad_desc={0};
            bad_desc.Format=test==10?DXGI_FORMAT_D24_UNORM_S8_UINT:DXGI_FORMAT_D32_FLOAT;
            bad_desc.ViewDimension=D3D12_DSV_DIMENSION_TEXTURE2D;
            bad_desc.Flags=test==8?D3D12_DSV_FLAG_READ_ONLY_DEPTH:test==9?D3D12_DSV_FLAG_READ_ONLY_STENCIL:0;
            SLOT(dev,0xa8,View)(dev,small,&bad_desc,small_dsv);
            SLOT(bad,0x198,ClearDepth)(bad,small_dsv,1,1.0f,0,0,0);
        } else {
            union { unsigned u; float f; } nan; nan.u=0x7fc00000;
            unsigned flags=test==0?0:test==1?2:test==2?3:1;
            float value=test==3?-0.1f:test==4?nan.f:test==7?1.1f:1.0f;
            SLOT(bad,0x198,ClearDepth)(bad,dsv,flags,value,0,0,0);
        }
        if (ID3D12GraphicsCommandList_Close(bad)!=E_NOTIMPL) return 20+test;
        SLOT(bad,0x10,Release)(bad);
    }
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,write_pipeline,&IID_ID3D12GraphicsCommandList,(void **)&first)!=S_OK || ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,write_pipeline,&IID_ID3D12GraphicsCommandList,(void **)&second)!=S_OK) return 26;
    const float black[4]={0,0,0,0}; SLOT(first,0x1a0,ClearColor)(first,rtv,black,0,0); SLOT(first,0x198,ClearDepth)(first,dsv,1,1.0f,0,0,0);
    bind_graphics(first,&view,rtv,dsv);
    draw(first,0,0x3e800000,0,8); /* .5 writes, red .25 */
    draw(first,3,0x3f000000,0,8); /* .75 fails LESS */
    int indexed_scissor[4]={2,0,4,8}; SLOT(first,0xb0,Array)(first,1,indexed_scissor);
    SLOT(first,0x130,Constant)(first,3,0x3f400000,2); SLOT(first,0x178,IndexBuffer)(first,&index);
    SLOT(first,0x68,DrawIndexed)(first,3,1,0,0,0); /* indexed .25 passes */
    SLOT(first,0xc8,SetPipeline)(first,no_write);
    draw(first,9,0x3f000000,4,8); /* .125 passes without writing */
    SLOT(first,0xc8,SetPipeline)(first,write_pipeline);
    draw(first,6,0x3f800000,4,6); /* .25 must pass against .5 */
    copy_color(first,target,out);
    if (ID3D12GraphicsCommandList_Close(first)!=S_OK) return 27;
    SLOT(second,0x198,ClearDepth)(second,dsv,1,0.875f,0,0,0);
    int clear_rect[4]={6,0,9,8}; SLOT(second,0x198,ClearDepth)(second,dsv,1,0.125f,0,1,clear_rect);
    clear_rect[0]=0; clear_rect[2]=8; /* recorded clear owns its rectangle */
    bind_graphics(second,&view,rtv,dsv); draw(second,3,0x3f000000,0,4);
    copy_color(second,target,out);
    if (ID3D12GraphicsCommandList_Close(second)!=S_OK) return 28;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 29;
    /* Exercise all eight D3D comparison functions against less/equal/greater
       fragments, keeping depth writes disabled so each sees the same clear. */
    const unsigned pass[8][3]={{0,0,0},{1,0,0},{0,1,0},{1,1,0},{0,0,1},{1,0,1},{0,1,1},{1,1,1}};
    for (unsigned function=1;function<=8;function++) {
        void *compare_pipeline=0; ID3D12GraphicsCommandList *compare_list=0;
        stream_init(stream.b,&element); *(unsigned *)(stream.b+104)=0; *(unsigned *)(stream.b+108)=function;
        if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&compare_pipeline)!=S_OK) return 34;
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,compare_pipeline,&IID_ID3D12GraphicsCommandList,(void **)&compare_list)!=S_OK) return 35;
        SLOT(compare_list,0x1a0,ClearColor)(compare_list,rtv,black,0,0);
        SLOT(compare_list,0x198,ClearDepth)(compare_list,dsv,1,0.5f,0,0,0);
        bind_graphics(compare_list,&view,rtv,dsv);
        draw(compare_list,6,0x3f800000,0,2);
        draw(compare_list,0,0x3f800000,2,5);
        draw(compare_list,3,0x3f800000,5,8);
        copy_color(compare_list,target,out);
        if (ID3D12GraphicsCommandList_Close(compare_list)!=S_OK) return 36;
        ID3D12CommandList *one[]={(ID3D12CommandList *)compare_list};
        ID3D12CommandQueue_ExecuteCommandLists(queue,1,one);
        volatile unsigned char *color=o; volatile float *depth_pixels=d;
        if (color[4*256+4] != 255*pass[function-1][0] ||
            color[4*256+12] != 255*pass[function-1][1] ||
            color[4*256+24] != 255*pass[function-1][2] ||
            depth_pixels[4*64+1] != 0.5f || depth_pixels[4*64+3] != 0.5f || depth_pixels[4*64+6] != 0.5f) return 100+function;
        SLOT(compare_list,0x10,Release)(compare_list); SLOT(compare_pipeline,0x10,Release)(compare_pipeline);
    }
    /* Recordings own their resource snapshots, not the DSV heap or caller references. */
    SLOT(dsvheap,0x10,Release)(dsvheap); SLOT(depth,0x10,Release)(depth);
    SLOT(rtvheap,0x10,Release)(rtvheap); SLOT(target,0x10,Release)(target); SLOT(vb,0x10,Release)(vb); SLOT(ib,0x10,Release)(ib); SLOT(out,0x10,Release)(out);
    SLOT(write_pipeline,0x10,Release)(write_pipeline); SLOT(no_write,0x10,Release)(no_write);
    ID3D12CommandList *lists[]={(ID3D12CommandList *)first}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    if (!pixels(o,d,0)) return 30;
    lists[0]=(ID3D12CommandList *)second; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    if (!pixels(o,d,1)) return 31;
    if (((volatile unsigned char *)o)[32]!=0x77) return 32;
    SLOT(smallheap,0x10,Release)(smallheap); SLOT(small,0x10,Release)(small);
    SLOT(queue,0x10,Release)(queue); SLOT(first,0x10,Release)(first); SLOT(second,0x10,Release)(second); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev);
    return 42;
}
