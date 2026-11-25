/* Real Wine D3D12.X primitive topology rendering and readback. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "shaders.h"

__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *,const unsigned *,const IID *,void **);
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef HRESULT (__stdcall *CreateResource)(void *,const void *,unsigned,const void *,unsigned,const void *,const IID *,void **);
typedef HRESULT (__stdcall *CreateObject)(void *,const void *,const IID *,void **);
typedef HRESULT (__stdcall *Map)(void *,unsigned,const void *,void **);
typedef U64 (__stdcall *Handle)(void *);
typedef unsigned (__stdcall *Increment)(void *,unsigned);
typedef unsigned (__stdcall *Release)(void *);
typedef void (__stdcall *View)(void *,void *,const void *,U64);
typedef void (__stdcall *Draw)(void *,unsigned,unsigned,unsigned,unsigned);
typedef void (__stdcall *DrawIndexed)(void *,unsigned,unsigned,unsigned,int,unsigned);
typedef void (__stdcall *IndexBuffer)(void *,const D3D12_INDEX_BUFFER_VIEW *);
typedef void (__stdcall *Topology)(void *,unsigned);
typedef void (__stdcall *SetArray)(void *,unsigned,const void *);
typedef void (__stdcall *Vertices)(void *,unsigned,unsigned,const void *);
typedef void (__stdcall *Targets)(void *,unsigned,const U64 *,int,const U64 *);
typedef void (__stdcall *Clear)(void *,U64,const float *,unsigned,const void *);
typedef void (__stdcall *Copy)(void *,const void *,unsigned,unsigned,unsigned,const void *,const void *);
typedef unsigned (__stdcall *BarrierFn)(void *,unsigned,const void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind,flags; void *resource; U64 data[2]; } Barrier;
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),s,(DWORD)strlen(s),&n,0); }

/* Scan a small neighborhood to avoid depending on line/point edge conventions. */
static int covered(volatile unsigned char *p,int x,int y) {
    for (int j=y-1;j<=y+1;j++) for (int i=x-1;i<=x+1;i++)
        if (p[j*256+i*4+2]>64) return 1;
    return 0;
}

int main(int argc,char **argv) {
    if (argc!=4) return 90;
    unsigned topology=(unsigned)strtoul(argv[1],0,10), indexed=(unsigned)strtoul(argv[2],0,10);
    int restart16=!strcmp(argv[3],"restart16"), restart32=!strcmp(argv[3],"restart32");
    int invalid=!strcmp(argv[3],"invalid") || restart16 || restart32;
    if (strcmp(argv[3],"ok") && !invalid) return 90;
    unsigned version=0x00020004; void *dev=0,*vb=0,*ib=0,*out=0,*target[2]={0},*heap=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,96,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=16; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&ib)!=S_OK) return 3;
    rd.Width=8192; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK) return 4;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=16; rd.Height=16; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM;
    rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target[0])!=S_OK) return 5;
    rd.Format=DXGI_FORMAT_B8G8R8A8_UNORM;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target[1])!=S_OK) return 6;
    void *v=0,*indices=0,*p=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(ib,0x40,Map)(ib,0,0,&indices)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK) return 7;
    const float vertices[24]={-.75f,.75f,0,1,.5f,.125f, .75f,.75f,0,1,.5f,.125f, -.75f,-.75f,0,1,.5f,.125f, .75f,-.75f,0,1,.5f,.125f};
    if (indexed) {
        for (unsigned i=0;i<4;i++) memcpy((float *)v+i*6,vertices+(3-i)*6,6*sizeof(float));
    } else memcpy(v,vertices,sizeof(vertices));
    memset(p,0x77,8192);
    if (restart32) { unsigned a[4]={3,2,0xffffffffu,0}; memcpy(indices,a,sizeof(a)); }
    else { unsigned short a[4]={3,2,restart16?0xffffu:1,0}; memcpy(indices,a,sizeof(a)); }
    HeapDesc hd={{D3D12_DESCRIPTOR_HEAP_TYPE_RTV,2,0,0},{0,0}};
    if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heap)!=S_OK) return 8;
    U64 rtv[2]; rtv[0]=SLOT(heap,0x48,Handle)(heap); rtv[1]=rtv[0]+SLOT(dev,0x78,Increment)(dev,2);
    for (unsigned i=0;i<2;i++) SLOT(dev,0xa0,View)(dev,target[i],0,rtv[i]);
    D3D12_INPUT_ELEMENT_DESC elements[2]={{"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},{"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
    union { U64 align; unsigned char b[560]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=topology==1?1:topology<=3?2:3; *(unsigned *)(stream.b+184)=15;
    *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+192)=87; *(unsigned *)(stream.b+220)=2;
    *(unsigned *)(stream.b+224)=8; *(unsigned *)(stream.b+232)=1; *(unsigned *)(stream.b+236)=1;
    for (unsigned i=0;i<6;i++) *(unsigned *)(stream.b+244+4*i)=(unsigned[]){2,2,1,2,2,1}[i];
    stream.b[272]=5;
    for (unsigned i=0;i<6;i++) *(unsigned *)(stream.b+284+4*i)=(unsigned[]){2,1,1,2,1,1}[i];
    stream.b[312]=10;
    StreamDesc sd={sizeof(stream.b),stream.b};
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK || !pipeline) return 9;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 10;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 11;
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),96,24};
    D3D12_INDEX_BUFFER_VIEW index={SLOT(ib,0x58,Handle)(ib),restart32?16:8,restart32?DXGI_FORMAT_R32_UINT:DXGI_FORMAT_R16_UINT};
    float viewport[6]={0,0,16,16,0,1}; int scissor[4]={0,0,16,16};
    if (invalid) { char msg[128]; snprintf(msg,sizeof(msg),"D3D12X topology %u indexed %u %s armed\r\n",topology,indexed,argv[3]); say(msg); }
    SLOT(list,0xa0,Topology)(list,topology); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
    SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,2,rtv,0,0);
    const float clear[4]={0,0,0,0};
    SLOT(list,0x1a0,Clear)(list,rtv[0],clear,0,0); SLOT(list,0x1a0,Clear)(list,rtv[1],clear,0,0);
    unsigned draw_count=topology==4?3:4;
    if (indexed) { SLOT(list,0x178,IndexBuffer)(list,&index); SLOT(list,0x68,DrawIndexed)(list,draw_count,1,0,0,0); }
    else SLOT(list,0x60,Draw)(list,draw_count,1,0,0);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Width=16; dst.PlacedFootprint.Footprint.Height=16; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
    for (unsigned i=0;i<2;i++) {
        Barrier b={0,0,target[i],{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&b);
        dst.PlacedFootprint.Offset=4096*i; dst.PlacedFootprint.Footprint.Format=i?DXGI_FORMAT_B8G8R8A8_UNORM:DXGI_FORMAT_R8G8B8A8_UNORM;
        src.pResource=target[i]; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    }
    HRESULT hr=ID3D12GraphicsCommandList_Close(list);
    if (invalid && FAILED(hr)) { say("D3D12X topology rejected\r\n"); return 43; }
    if (hr!=S_OK) return 12;
    D3D12_COMMAND_QUEUE_DESC qd={0}; if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 13;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    if (invalid) return 91;
    volatile unsigned char *pixels=p;
    /* Top and bottom horizontal midpoints, diagonal midpoint, and interiors of
       the first and second triangles distinguish all five primitive modes. */
    const int xy[5][2]={{8,2},{8,14},{8,8},{4,4},{11,11}};
    const int expected[5][5]={{0,0,0,0,0},{1,1,0,0,0},{1,1,1,0,0},{1,0,1,1,0},{1,1,1,1,1}};
    for (unsigned i=0;i<5;i++) {
        int actual=covered(pixels,xy[i][0],xy[i][1]);
        if (actual!=expected[topology-1][i]) { char msg[128]; snprintf(msg,sizeof(msg),"topology %u region %u got %d expected %d\r\n",topology,i,actual,expected[topology-1][i]); say(msg); return 20; }
    }
    if (topology==1 && (!covered(pixels,2,2) || !covered(pixels,14,2) || !covered(pixels,2,14) || !covered(pixels,14,14))) return 21;
    if (pixels[64]!=0x77) return 22;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator);
    SLOT(pipeline,0x10,Release)(pipeline); SLOT(heap,0x10,Release)(heap);
    for (unsigned i=0;i<2;i++) SLOT(target[i],0x10,Release)(target[i]);
    SLOT(vb,0x10,Release)(vb); SLOT(ib,0x10,Release)(ib); SLOT(out,0x10,Release)(out); SLOT(dev,0x10,Release)(dev);
    char msg[96]; snprintf(msg,sizeof(msg),"D3D12X topology %u indexed %u passed\r\n",topology,indexed); say(msg); return 42;
}
