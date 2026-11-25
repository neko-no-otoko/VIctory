/* Self-authored console shaders and real Wine D3D12.X MRT readback. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <stdio.h>
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

int main(int argc,char **argv) {
    const char *mode=argc==1?"ok":argc==2?argv[1]:"invalid";
    int count=!strcmp(mode,"--count"), format=!strcmp(mode,"--format"), unsupported=!strcmp(mode,"--unsupported");
    int invalid=count || format || unsupported;
    if (strcmp(mode,"ok") && !invalid) return 90;
    unsigned version=0x00020004; void *dev=0,*vb=0,*out=0,*target[2]={0},*heap=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,72,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=4096; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM;
    rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target[0])!=S_OK) return 4;
    rd.Format=DXGI_FORMAT_B8G8R8A8_UNORM;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target[1])!=S_OK) return 5;
    void *v=0,*p=0; if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK) return 6;
    const float tri[18]={-1,-1,0,1,.5f,.125f, 3,-1,0,1,.5f,.125f, -1,3,0,1,.5f,.125f};
    /* Position.xy is not constant; COLOR1 provides the other two channels. */
    memcpy(v,tri,sizeof(tri)); memset(p,0x77,4096);
    HeapDesc hd={{D3D12_DESCRIPTOR_HEAP_TYPE_RTV,2,0,0},{0,0}};
    if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heap)!=S_OK) return 7;
    U64 rtv[2]; rtv[0]=SLOT(heap,0x48,Handle)(heap); rtv[1]=rtv[0]+SLOT(dev,0x78,Increment)(dev,2);
    for (unsigned i=0;i<2;i++) SLOT(dev,0xa0,View)(dev,target[i],0,rtv[i]);
    D3D12_INPUT_ELEMENT_DESC elements[2]={{"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},{"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
    union { U64 align; unsigned char b[560]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15;
    *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+192)=format?28:87; *(unsigned *)(stream.b+220)=2;
    *(unsigned *)(stream.b+224)=8; *(unsigned *)(stream.b+232)=1; /* independent blend */
    *(unsigned *)(stream.b+236)=1;
    for (unsigned i=0;i<6;i++) *(unsigned *)(stream.b+244+4*i)=(unsigned[]){2,2,1,2,2,1}[i];
    stream.b[272]=5; /* target 0: additive, red and blue only */
    for (unsigned i=0;i<6;i++) *(unsigned *)(stream.b+284+4*i)=(unsigned[]){2,1,1,2,1,1}[i];
    stream.b[312]=10; /* target 1: replace, green and alpha only */
    if (unsupported) *(unsigned *)(stream.b+280)=1; /* logic ops unsupported */
    StreamDesc sd={sizeof(stream.b),stream.b};
    if (invalid) say("D3D12X MRT invalid armed\r\n");
    HRESULT hr=SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline);
    if (invalid && FAILED(hr) && !pipeline) { say("D3D12X MRT invalid rejected\r\n"); return 43; }
    if (hr!=S_OK || !pipeline) return 8;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 9;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 10;
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24}; float viewport[6]={0,0,8,8,0,1}; int scissor[4]={0,0,8,8};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
    SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,count?1:2,rtv,0,0);
    const float clear0[4]={.125f,.25f,.5f,.25f}, clear1[4]={.25f,.125f,.125f,.75f};
    SLOT(list,0x1a0,Clear)(list,rtv[0],clear0,0,0); SLOT(list,0x1a0,Clear)(list,rtv[1],clear1,0,0);
    SLOT(list,0x60,Draw)(list,3,1,0,0);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
    for (unsigned i=0;i<2;i++) {
        Barrier b={0,0,target[i],{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&b);
        dst.PlacedFootprint.Offset=2048*i; dst.PlacedFootprint.Footprint.Format=i?DXGI_FORMAT_B8G8R8A8_UNORM:DXGI_FORMAT_R8G8B8A8_UNORM;
        src.pResource=target[i]; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    }
    hr=ID3D12GraphicsCommandList_Close(list);
    if (invalid && FAILED(hr)) { say("D3D12X MRT invalid rejected\r\n"); return 43; }
    if (hr!=S_OK) return 11;
    D3D12_COMMAND_QUEUE_DESC qd={0}; if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 12;
    /* Recorded commands must own the render targets, descriptors and pipeline. */
    SLOT(pipeline,0x10,Release)(pipeline); SLOT(heap,0x10,Release)(heap);
    for (unsigned i=0;i<2;i++) SLOT(target[i],0x10,Release)(target[i]);
    SLOT(vb,0x10,Release)(vb);
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    if (invalid) return 91; /* An accepted invalid submission is not a rejection. */
    volatile unsigned char *pixels=p;
    /* At pixel (3,3), interpolated position.xy is (-.125,.125).
       MRT0 = (position.xy, .5, .125), MRT1 = (.125,.5,.125,-.125).
       MRT0 adds the clamped source to its clear; masked lanes retain clears.
       UNORM clamps negative alpha to zero. */
    const int expected[2][4]={{32,64,255,64},{32,128,64,0}};
    for (unsigned i=0;i<2;i++) for (unsigned c=0;c<4;c++) {
        unsigned actual=pixels[i*2048+3*256+3*4+c]; int delta=(int)actual-expected[i][c];
        if (delta < -3 || delta > 3) { char msg[128]; snprintf(msg,sizeof(msg),"MRT%u channel %u: %u expected %d\r\n",i,c,actual,expected[i][c]); say(msg); return 20+i; }
    }
    if (pixels[32]!=0x77 || pixels[2080]!=0x77) return 22;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator);
    SLOT(out,0x10,Release)(out); SLOT(dev,0x10,Release)(dev);
    say("D3D12X MRT passed\r\n"); return 42;
}
