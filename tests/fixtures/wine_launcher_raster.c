/* Real Wine D3D12.X rasterizer and two-sided stencil readback. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "windows_d3d12x_graphics_shaders.h"

__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *,const unsigned *,const IID *,void **);
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef HRESULT (__stdcall *CreateResource)(void *,const void *,unsigned,const void *,unsigned,const void *,const IID *,void **);
typedef HRESULT (__stdcall *CreateObject)(void *,const void *,const IID *,void **);
typedef HRESULT (__stdcall *Map)(void *,unsigned,const void *,void **);
typedef U64 (__stdcall *Handle)(void *);
typedef void (__stdcall *View)(void *,void *,const void *,U64);
typedef void (__stdcall *Draw)(void *,unsigned,unsigned,unsigned,unsigned);
typedef void (__stdcall *Constant)(void *,unsigned,unsigned,unsigned);
typedef void (__stdcall *Topology)(void *,unsigned);
typedef void (__stdcall *Array)(void *,unsigned,const void *);
typedef void (__stdcall *Vertices)(void *,unsigned,unsigned,const void *);
typedef void (__stdcall *Targets)(void *,unsigned,const U64 *,int,const U64 *);
typedef void (__stdcall *ClearDepth)(void *,U64,unsigned,float,unsigned char,unsigned,const void *);
typedef void (__stdcall *ClearColor)(void *,U64,const float *,unsigned,const void *);
typedef void (__stdcall *SetPipeline)(void *,void *);
typedef void (__stdcall *StencilRef)(void *,unsigned);
typedef void (__stdcall *Copy)(void *,const void *,unsigned,unsigned,unsigned,const void *,const void *);
typedef unsigned (__stdcall *BarrierFn)(void *,unsigned,const void *);
typedef unsigned (__stdcall *Release)(void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind,flags; void *resource; U64 data[2]; } Barrier;
static void face(unsigned char *b,unsigned pass) {
    unsigned *p=(unsigned *)b; p[0]=1;p[1]=1;p[2]=pass;p[3]=8; /* ALWAYS */
}
static void copy_plane(ID3D12GraphicsCommandList *l,void *target,void *out,U64 offset,unsigned format,unsigned plane) {
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0};
    dst.pResource=out;dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Offset=offset;dst.PlacedFootprint.Footprint.Format=format;
    dst.PlacedFootprint.Footprint.Width=8;dst.PlacedFootprint.Footprint.Height=8;
    dst.PlacedFootprint.Footprint.Depth=1;dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=target;src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;src.SubresourceIndex=plane;
    SLOT(l,0x80,Copy)(l,&dst,0,0,0,&src,0);
}
int main(int argc,char **argv) {
    if (argc!=4) return 90;
    unsigned cull=(unsigned)strtoul(argv[1],0,10), ccw=(unsigned)strtoul(argv[2],0,10);
    int stencil=!strcmp(argv[3],"stencil"), valid=!strcmp(argv[3],"color") || stencil;
    int wire=!strcmp(argv[3],"wireframe"), bad_cull=!strcmp(argv[3],"cull"), bias=!strcmp(argv[3],"bias"), clamp=!strcmp(argv[3],"clamp"), slope=!strcmp(argv[3],"slope");
    if (cull<1 || cull>3 || ccw>1 || (!valid && !wire && !bad_cull && !bias && !clamp && !slope)) return 90;
    unsigned version=0x00020004;void *dev=0,*vb=0,*out=0,*color=0,*depth=0,*rtvh=0,*dsvh=0,*pipes[2]={0};
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource);CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,96,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=4096;if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D;rd.Width=8;rd.Height=8;rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN;
    rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM;rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&color)!=S_OK) return 4;
    rd.Format=DXGI_FORMAT_D32_FLOAT_S8X24_UINT;rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL;
    if (make(dev,&hp,0,&rd,0x10,0,&IID_ID3D12Resource,&depth)!=S_OK) return 5;
    void *v=0,*o=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&o)!=S_OK) return 6;
    /* First triangle is counterclockwise in D3D render-target coordinates; second is its reverse. */
    const float vertices[24]={-1,-1,0.5f,1,3,-1,0.5f,1,-1,3,0.5f,1, -1,-1,0.5f,1,-1,3,0.5f,1,3,-1,0.5f,1};
    memcpy(v,vertices,sizeof(vertices));memset(o,0x77,4096);
    HeapDesc hd={{D3D12_DESCRIPTOR_HEAP_TYPE_RTV,1,0,0},{0,0}};
    if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&rtvh)!=S_OK) return 7;
    hd.desc.Type=D3D12_DESCRIPTOR_HEAP_TYPE_DSV;if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&dsvh)!=S_OK) return 8;
    U64 rtv=SLOT(rtvh,0x48,Handle)(rtvh),dsv=SLOT(dsvh,0x48,Handle)(dsvh);
    SLOT(dev,0xa0,View)(dev,color,0,rtv);SLOT(dev,0xa8,View)(dev,depth,0,dsv);
    D3D12_INPUT_ELEMENT_DESC element={"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0};
    union { U64 align;unsigned char b[232]; } stream={0};
    *(unsigned *)(stream.b+0)=1;*(const void **)(stream.b+8)=vs;*(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2;*(const void **)(stream.b+32)=ps;*(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10;*(unsigned *)(stream.b+52)=wire?2:3;*(unsigned *)(stream.b+56)=bad_cull?4:cull;
    *(unsigned *)(stream.b+60)=ccw;*(unsigned *)(stream.b+64)=bias?1:0;
    *(float *)(stream.b+68)=clamp?1.0f:0.0f;*(float *)(stream.b+72)=slope?1.0f:0.0f;*(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11;*(unsigned *)(stream.b+100)=1;*(unsigned *)(stream.b+108)=8;
    *(unsigned *)(stream.b+112)=stencil;stream.b[116]=0xff;stream.b[117]=0xff;
    face(stream.b+120,3);face(stream.b+136,4); /* front REPLACE, back INCR_SAT */
    *(unsigned *)(stream.b+152)=12;*(const void **)(stream.b+160)=&element;*(unsigned *)(stream.b+168)=1;
    *(unsigned *)(stream.b+176)=14;*(unsigned *)(stream.b+180)=3;
    *(unsigned *)(stream.b+184)=15;*(unsigned *)(stream.b+188)=28;*(unsigned *)(stream.b+220)=1;
    *(unsigned *)(stream.b+224)=16;*(unsigned *)(stream.b+228)=20;
    StreamDesc sd={sizeof(stream.b),stream.b};HRESULT hr=SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipes[0]);
    if (!valid) {if (FAILED(hr) && !pipes[0]) {printf("raster %s rejected\n",argv[3]);return 43;}return 91;}
    if (hr!=S_OK || !pipes[0]) return 9;
    /* Start with a different rasterizer and switch PSOs before the tested draws. */
    *(unsigned *)(stream.b+56)=cull==1?3:1;*(unsigned *)(stream.b+60)=!ccw;
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipes[1])!=S_OK) return 10;
    ID3D12CommandAllocator *allocator=0;ID3D12GraphicsCommandList *list=0;ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 11;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipes[1],&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 12;
    const float black[4]={0,0,0,0};SLOT(list,0x1a0,ClearColor)(list,rtv,black,0,0);SLOT(list,0x198,ClearDepth)(list,dsv,3,1.0f,0x10,0,0);
    float viewport[6]={0,0,8,8,0,1};int left[4]={0,0,4,8},right[4]={4,0,8,8};
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),96,16};
    SLOT(list,0xa0,Topology)(list,4);SLOT(list,0xa8,Array)(list,1,viewport);
    SLOT(list,0x180,Vertices)(list,0,1,&view);SLOT(list,0x190,Targets)(list,1,&rtv,0,&dsv);
    SLOT(list,0x130,Constant)(list,3,0,0);SLOT(list,0x130,Constant)(list,3,0,1);SLOT(list,0x130,Constant)(list,3,0x3f800000,2);
    SLOT(list,0xc0,StencilRef)(list,0x55);
    SLOT(list,0xb0,Array)(list,1,left);SLOT(list,0x60,Draw)(list,3,1,0,0);
    SLOT(list,0x1a0,ClearColor)(list,rtv,black,0,0);SLOT(list,0x198,ClearDepth)(list,dsv,3,1.0f,0x10,0,0);
    SLOT(list,0xc8,SetPipeline)(list,pipes[0]);
    SLOT(list,0xb0,Array)(list,1,left);SLOT(list,0x60,Draw)(list,3,1,0,0);
    SLOT(list,0xb0,Array)(list,1,right);SLOT(list,0x60,Draw)(list,3,1,3,0);
    Barrier cb={0,0,color,{0xffffffffULL | (4ULL<<32),0x800}};SLOT(list,0xd0,BarrierFn)(list,1,&cb);
    copy_plane(list,color,out,0,DXGI_FORMAT_R8G8B8A8_UNORM,0);
    Barrier db={0,0,depth,{0xffffffffULL | (0x10ULL<<32),0x800}};SLOT(list,0xd0,BarrierFn)(list,1,&db);
    copy_plane(list,depth,out,2048,DXGI_FORMAT_R8_UINT,1);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 13;
    D3D12_COMMAND_QUEUE_DESC qd={0};if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 14;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list};ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    volatile unsigned char *p=o;
    for (unsigned y=0;y<8;y++) for (unsigned x=0;x<8;x++) {
        int front=(x<4) == (int)ccw;int visible=cull==1 || (cull==2 && !front) || (cull==3 && front);
        unsigned i=y*256+x*4, want=visible?255:0;
        if (p[i]!=want || p[i+1]!=0 || p[i+2]!=0 || p[i+3]!=want) {printf("color cull %u ccw %u (%u,%u): %u,%u,%u,%u expected %u\n",cull,ccw,x,y,p[i],p[i+1],p[i+2],p[i+3],want);return 20;}
        unsigned sw=stencil && visible?(front?0x55:0x11):0x10;
        if (p[2048+y*256+x]!=sw) {printf("stencil cull %u ccw %u (%u,%u): %02x expected %02x\n",cull,ccw,x,y,p[2048+y*256+x],sw);return 21;}
    }
    SLOT(queue,0x10,Release)(queue);SLOT(list,0x10,Release)(list);SLOT(allocator,0x10,Release)(allocator);
    for (unsigned i=0;i<2;i++) SLOT(pipes[i],0x10,Release)(pipes[i]);
    SLOT(rtvh,0x10,Release)(rtvh);SLOT(dsvh,0x10,Release)(dsvh);SLOT(color,0x10,Release)(color);SLOT(depth,0x10,Release)(depth);SLOT(vb,0x10,Release)(vb);SLOT(out,0x10,Release)(out);SLOT(dev,0x10,Release)(dev);
    printf("raster cull %u ccw %u %s passed\n",cull,ccw,argv[3]);return 42;
}
