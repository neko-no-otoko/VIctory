/* Normal Wine guest: D32_FLOAT_S8X24 depth/stencil with separate plane readback. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <stdio.h>
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
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),s,(DWORD)strlen(s),&n,0); }
static void face(unsigned char *b,unsigned fail,unsigned depth_fail,unsigned pass,unsigned compare) {
    unsigned *p=(unsigned *)b; p[0]=fail;p[1]=depth_fail;p[2]=pass;p[3]=compare;
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
    int invalid_op=argc==2 && !strcmp(argv[1],"--invalid-op");
    int invalid_format=argc==2 && !strcmp(argv[1],"--invalid-format");
    if (argc>2 || (argc==2 && !invalid_op && !invalid_format)) return 90;
    unsigned version=0x00020004;void *dev=0,*vb=0,*out=0,*color=0,*depth=0,*rtvh=0,*dsvh=0,*pipes[3]={0};
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource);CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,192,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=6144;if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D;rd.Width=8;rd.Height=8;rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN;
    rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM;rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&color)!=S_OK) return 4;
    rd.Format=invalid_format?DXGI_FORMAT_D24_UNORM_S8_UINT:DXGI_FORMAT_D32_FLOAT_S8X24_UINT;
    rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL;
    HRESULT hr=make(dev,&hp,0,&rd,0x10,0,&IID_ID3D12Resource,&depth);
    if (invalid_format) { if (FAILED(hr) && !depth) {say("invalid stencil format rejected\r\n");return 43;} return 91; }
    if (hr!=S_OK) return 5;
    void *v=0,*o=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&o)!=S_OK) return 6;
    /* D3D's default front face is clockwise in render-target coordinates. */
    for (unsigned i=0;i<3;i++) { float z=(float[]){0.5f,0.25f,0.9f}[i];float tri[12]={-1,-1,z,1,-1,3,z,1,3,-1,z,1};memcpy((float *)v+i*12,tri,sizeof(tri)); }
    const float reverse[12]={-1,-1,0.5f,1,3,-1,0.5f,1,-1,3,0.5f,1};
    memcpy((float *)v+36,reverse,sizeof(reverse));
    memset(o,0x77,6144);
    HeapDesc hd={{D3D12_DESCRIPTOR_HEAP_TYPE_RTV,1,0,0},{0,0}};
    if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&rtvh)!=S_OK) return 7;
    hd.desc.Type=D3D12_DESCRIPTOR_HEAP_TYPE_DSV;if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&dsvh)!=S_OK) return 8;
    U64 rtv=SLOT(rtvh,0x48,Handle)(rtvh),dsv=SLOT(dsvh,0x48,Handle)(dsvh);
    SLOT(dev,0xa0,View)(dev,color,0,rtv);SLOT(dev,0xa8,View)(dev,depth,0,dsv);
    D3D12_INPUT_ELEMENT_DESC element={"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0};
    union { U64 align;unsigned char b[232]; } stream={0};
    *(unsigned *)(stream.b+0)=1;*(const void **)(stream.b+8)=vs;*(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2;*(const void **)(stream.b+32)=ps;*(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10;*(unsigned *)(stream.b+52)=3;*(unsigned *)(stream.b+56)=1;*(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11;*(unsigned *)(stream.b+100)=1;*(unsigned *)(stream.b+104)=1;*(unsigned *)(stream.b+108)=2;
    *(unsigned *)(stream.b+112)=1;stream.b[116]=0xf0;stream.b[117]=0x0f;
    face(stream.b+120,1,1,invalid_op?9:3,3);face(stream.b+136,1,1,2,3);
    *(unsigned *)(stream.b+152)=12;*(const void **)(stream.b+160)=&element;*(unsigned *)(stream.b+168)=1;
    *(unsigned *)(stream.b+176)=14;*(unsigned *)(stream.b+180)=3;
    *(unsigned *)(stream.b+184)=15;*(unsigned *)(stream.b+188)=28;*(unsigned *)(stream.b+220)=1;
    *(unsigned *)(stream.b+224)=16;*(unsigned *)(stream.b+228)=20;
    StreamDesc sd={sizeof(stream.b),stream.b};hr=SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipes[0]);
    if (invalid_op) {if (FAILED(hr) && !pipes[0]) {say("invalid stencil op rejected\r\n");return 43;}return 92;}
    if (hr!=S_OK) return 9;
    stream.b[116]=0xff;stream.b[117]=0;face(stream.b+120,1,1,1,3);face(stream.b+136,1,1,1,3);
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipes[1])!=S_OK) return 10;
    stream.b[117]=0xff;face(stream.b+120,1,4,1,3);face(stream.b+136,1,4,1,3);
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipes[2])!=S_OK) return 11;
    ID3D12CommandAllocator *allocator=0;ID3D12GraphicsCommandList *list=0;ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 12;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipes[0],&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 13;
    const float black[4]={0,0,0,0};SLOT(list,0x1a0,ClearColor)(list,rtv,black,0,0);
    SLOT(list,0x198,ClearDepth)(list,dsv,3,0.5f,0x11,0,0);
    SLOT(list,0x198,ClearDepth)(list,dsv,1,0.75f,0,0,0);
    SLOT(list,0x198,ClearDepth)(list,dsv,2,0.0f,0xa0,0,0);
    float viewport[6]={0,0,8,8,0,1};int left[4]={0,0,4,4},lower[4]={0,4,4,8},full[4]={0,0,8,8};
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),192,16};
    SLOT(list,0xa0,Topology)(list,4);SLOT(list,0xa8,Array)(list,1,viewport);SLOT(list,0xb0,Array)(list,1,left);
    SLOT(list,0x180,Vertices)(list,0,1,&view);SLOT(list,0x190,Targets)(list,1,&rtv,0,&dsv);
    SLOT(list,0x130,Constant)(list,3,0,0);SLOT(list,0x130,Constant)(list,3,0,1);SLOT(list,0x130,Constant)(list,3,0x3e800000,2);
    SLOT(list,0xc0,StencilRef)(list,0xa5);SLOT(list,0x60,Draw)(list,3,1,0,0);
    SLOT(list,0xb0,Array)(list,1,lower);SLOT(list,0x60,Draw)(list,3,1,9,0);
    SLOT(list,0xc8,SetPipeline)(list,pipes[1]);SLOT(list,0xb0,Array)(list,1,full);
    SLOT(list,0xc0,StencilRef)(list,0xa4);SLOT(list,0x130,Constant)(list,3,0x3f800000,2);
    SLOT(list,0x60,Draw)(list,3,1,3,0); /* must fail with the changed reference */
    SLOT(list,0xc0,StencilRef)(list,0xa5);
    SLOT(list,0x130,Constant)(list,3,0x3f400000,2);SLOT(list,0x60,Draw)(list,3,1,3,0);
    SLOT(list,0xc8,SetPipeline)(list,pipes[2]);SLOT(list,0x130,Constant)(list,3,0x3f800000,2);SLOT(list,0x60,Draw)(list,3,1,6,0);
    Barrier cb={0,0,color,{0xffffffffULL | (4ULL<<32),0x800}};SLOT(list,0xd0,BarrierFn)(list,1,&cb);
    copy_plane(list,color,out,0,DXGI_FORMAT_R8G8B8A8_UNORM,0);
    Barrier db={0,0,depth,{0xffffffffULL | (0x10ULL<<32),0x800}};SLOT(list,0xd0,BarrierFn)(list,1,&db);
    copy_plane(list,depth,out,2048,DXGI_FORMAT_R32_FLOAT,0);copy_plane(list,depth,out,4096,DXGI_FORMAT_R8_UINT,1);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 14;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 15;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list};ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    volatile unsigned char *p=o;
    for (unsigned y=0;y<8;y++) for (unsigned x=0;x<8;x++) {
        unsigned i=y*256+x*4;float z;memcpy(&z,(const void *)(p+2048+i),4);
        unsigned want=x<4?(y<4?191:64):0;
        if (p[i]!=want || p[i+1]!=0 || p[i+2]!=0 || p[i+3]!=(x<4?255:0)) {
            printf("color mismatch (%u,%u): %u,%u,%u,%u expected red %u\n",x,y,p[i],p[i+1],p[i+2],p[i+3],want);return 20;
        }
        if (z!=(x<4?(y<4?0.25f:0.5f):0.75f)) {printf("depth mismatch (%u,%u): %f\n",x,y,z);return 21;}
        if (p[4096+y*256+x]!=((x<4 && y<4)?0xa6:0xa0)) {printf("stencil mismatch (%u,%u): %02x\n",x,y,p[4096+y*256+x]);return 22;}
    }
    for (unsigned y=0;y<8;y++) for (unsigned x=32;x<256;x++) if (p[y*256+x]!=0x77 || p[2048+y*256+x]!=0x77) return 23;
    for (unsigned y=0;y<8;y++) for (unsigned x=8;x<256;x++) if (p[4096+y*256+x]!=0x77) return 24;
    SLOT(queue,0x10,Release)(queue);SLOT(list,0x10,Release)(list);SLOT(allocator,0x10,Release)(allocator);
    for (unsigned i=0;i<3;i++) SLOT(pipes[i],0x10,Release)(pipes[i]);
    SLOT(rtvh,0x10,Release)(rtvh);SLOT(dsvh,0x10,Release)(dsvh);SLOT(color,0x10,Release)(color);SLOT(depth,0x10,Release)(depth);SLOT(vb,0x10,Release)(vb);SLOT(out,0x10,Release)(out);SLOT(dev,0x10,Release)(dev);
    say("Wine Metal depth stencil passed\r\n");return 42;
}
