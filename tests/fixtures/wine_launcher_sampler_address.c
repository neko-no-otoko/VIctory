/* Ordinary Wine PE: independent sampler addressing and descriptor snapshots. */
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
typedef unsigned (__stdcall *Increment)(void *, unsigned);
typedef void (__stdcall *View)(void *, void *, const void *, U64);
typedef void (__stdcall *Sampler)(void *, const unsigned *, U64);
typedef void (__stdcall *Draw)(void *, unsigned, unsigned, unsigned, unsigned);
typedef void (__stdcall *SetTable)(void *, unsigned, U64);
typedef void (__stdcall *SetHeaps)(void *, unsigned, void **);
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

enum Case { VALID, BORDER, COMPARISON, ANISOTROPY, CHANGED_SAMPLER };
int main(int argc, char **argv) {
    enum Case test=VALID;
    if (argc==2) {
        if (!strcmp(argv[1],"--border")) test=BORDER;
        else if (!strcmp(argv[1],"--comparison")) test=COMPARISON;
        else if (!strcmp(argv[1],"--anisotropy")) test=ANISOTROPY;
        else if (!strcmp(argv[1],"--changed-sampler")) test=CHANGED_SAMPLER;
        else return 90;
    } else if (argc!=1) return 90;
    if (tls_value!=17) return 91;
    tls_value=29;
    unsigned version=0x00020004; void *dev=0,*vb=0,*out=0,*upload=0,*target=0,*texture[2]={0},*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,72,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=4096; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK || make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&upload)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 4;
    rd.Width=2; rd.Height=2; rd.Flags=0;
    for (unsigned i=0;i<2;i++) {
        rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM;
        if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&texture[i])!=S_OK) return 5;
    }
    void *v=0,*p=0,*t=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK || SLOT(upload,0x40,Map)(upload,0,0,&t)!=S_OK) return 6;
    const float tri[18]={-1,-1,0,1, 1.125f,-.625f, 3,-1,0,1, 1.125f,-.625f, -1,3,0,1, 1.125f,-.625f};
    memcpy(v,tri,sizeof(tri)); memset(p,0x77,4096); memset(t,0,4096);
    /* Independent red and green axes expose U and V separately. */
    const unsigned char texels[4][4]={
        {20,40,0,255},{100,40,0,255},{20,120,0,255},{100,120,0,255}
    };
    for (unsigned i=0;i<2;i++) for (unsigned y=0;y<2;y++) memcpy((char *)t+i*1024+y*256,texels+2*y,8);
    void *heaps[3]={0}; U64 cpu[3],gpu[3];
    for (unsigned i=0;i<3;i++) {
        HeapDesc hd={{i==0?2:i==1?0:1,i==0?1:i==1?2:6,i==0?0:1,0},{0,0}};
        if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 7;
        cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]);
    }
    unsigned ts=SLOT(dev,0x78,Increment)(dev,0), ss=SLOT(dev,0x78,Increment)(dev,1);
    if (!ts || !ss) return 8;
    SLOT(dev,0xa0,View)(dev,target,0,cpu[0]);
    D3D12_SHADER_RESOURCE_VIEW_DESC srv={0}; srv.Format=DXGI_FORMAT_R8G8B8A8_UNORM; srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D;
    srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING; srv.Texture2D.MipLevels=1;
    for (unsigned i=0;i<2;i++) {
        srv.Format=DXGI_FORMAT_R8G8B8A8_UNORM;
        SLOT(dev,0x90,View)(dev,texture[i],&srv,cpu[1]+i*ts);
    }
    /* D3D address values: wrap=1, mirror=2, clamp=3. W varies too. */
    unsigned samplers[6][13]={
        {0,1,1,2,0,1,8,0,0,0,0,0,0}, {0,2,2,1,0,1,8,0,0,0,0,0,0},
        {0,3,3,3,0,1,8,0,0,0,0,0,0}, {0x15,1,2,3,0,1,8,0,0,0,0,0,0},
        {0x15,2,1,2,0,1,8,0,0,0,0,0,0}, {0x15,3,3,1,0,1,8,0,0,0,0,0,0}
    };
    if (test==BORDER || test==COMPARISON || test==ANISOTROPY) {
        say("D3D12X invalid sampler armed\r\n");
        if (test==BORDER) samplers[0][1]=4;
        if (test==COMPARISON) samplers[0][0]=0x80;
        if (test==ANISOTROPY) samplers[0][0]=0x55;
    }
    for (unsigned i=0;i<6;i++) SLOT(dev,0xb0,Sampler)(dev,samplers[i],cpu[2]+i*ss);
    D3D12_INPUT_ELEMENT_DESC elements[2]={{"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},{"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    StreamDesc sd={sizeof(stream.b),stream.b}; if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 9;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK || ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 10;
    for (unsigned i=0;i<2;i++) {
        D3D12_TEXTURE_COPY_LOCATION src={0},dst={0}; src.pResource=upload; src.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
        src.PlacedFootprint.Offset=i*1024; src.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; src.PlacedFootprint.Footprint.Width=2; src.PlacedFootprint.Footprint.Height=2; src.PlacedFootprint.Footprint.Depth=1; src.PlacedFootprint.Footprint.RowPitch=256;
        dst.pResource=texture[i]; dst.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
        Barrier b={0,0,texture[i],{0xffffffffULL | (0x400ULL<<32),0x80}}; SLOT(list,0xd0,BarrierFn)(list,1,&b);
    }
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24}; float viewport[6]={0,0,8,8,0,1};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&cpu[0],0,0);
    void *pair[2]={heaps[1],heaps[2]}; SLOT(list,0x100,SetHeaps)(list,2,pair);
    if (test==CHANGED_SAMPLER) say("D3D12X invalid sampler armed\r\n");
    const unsigned first[4]={0,1,2,4}, second[4]={3,4,5,0};
    for (unsigned i=0;i<4;i++) {
        int scissor[4]={(int)(i*2),0,(int)(i*2+2),8}; SLOT(list,0xb0,SetArray)(list,1,scissor);
        SLOT(list,0x120,SetTable)(list,1,gpu[2]+first[i]*ss);
        SLOT(list,0x120,SetTable)(list,2,gpu[1]);
        SLOT(list,0x120,SetTable)(list,3,gpu[2]+second[i]*ss);
        SLOT(list,0x120,SetTable)(list,4,gpu[1]+ts);
        SLOT(list,0x60,Draw)(list,3,1,0,0);
    }
    Barrier rb={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&rb);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT; dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256; src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    HRESULT hr=ID3D12GraphicsCommandList_Close(list);
    if (hr!=S_OK) return 11;
    if (test==CHANGED_SAMPLER) SLOT(dev,0xb0,Sampler)(dev,samplers[2],cpu[2]);
    /* Command lists retain descriptors and resources after application releases. */
    SLOT(upload,0x10,Release)(upload); SLOT(vb,0x10,Release)(vb); SLOT(out,0x10,Release)(out); SLOT(target,0x10,Release)(target);
    for (unsigned i=0;i<2;i++) SLOT(texture[i],0x10,Release)(texture[i]);
    SLOT(pipeline,0x10,Release)(pipeline); for (unsigned i=0;i<3;i++) SLOT(heaps[i],0x10,Release)(heaps[i]);
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 12;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    if (test!=VALID) return 93;
    const int expected[4][4]={{20,40,40,100},{100,120,100,60},{100,40,100,40},{100,60,20,40}};
    volatile unsigned char *pixels=p;
    for (unsigned i=0;i<4;i++) for (unsigned c=0;c<4;c++) {
        int got=pixels[3*256+(i*2+1)*4+c], delta=got-expected[i][c];
        if (delta < -2 || delta > 2) { char msg[100]; snprintf(msg,sizeof(msg),"sampler address pixel %u channel %u got %d expected %d\r\n",i,c,got,expected[i][c]); say(msg); return 20+i; }
    }
    for (unsigned y=0;y<8;y++) for (unsigned x=32;x<256;x++) if (pixels[y*256+x]!=0x77) return 24;
    if (tls_value!=29) return 25;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev);
    say("D3D12X sampler address passed\r\n"); return 42;
}
