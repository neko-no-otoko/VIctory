/* Synthetic PE: sample recorded SRV/sampler tables via translated shaders. */
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
typedef void (__stdcall *Sampler)(void *, const unsigned *, U64);
typedef void (__stdcall *Draw)(void *, unsigned, unsigned, unsigned, unsigned);
typedef void (__stdcall *SetConstant)(void *, unsigned, unsigned, unsigned);
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
static int marker=7; static int *volatile relocated=&marker;
unsigned mainCRTStartup(void) {
    if (*relocated != 7) return 99;
    unsigned version=0x00020004; void *dev=0,*vb=0,*readback=0,*target=0,*first=0,*second=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,144,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    CreateResource make=SLOT(dev,0xd8,CreateResource);
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=1024; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&readback)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=9; rd.Height=4; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 4;
    rd.Width=2; rd.Height=1; rd.Flags=0;
    if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&first)!=S_OK) return 5;
    rd.Width=3; if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&second)!=S_OK) return 6;
    void *v=0,*out=0,*target_data=0,*a=0,*b=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(readback,0x40,Map)(readback,0,0,&out)!=S_OK || SLOT(target,0x40,Map)(target,0,0,&target_data)!=S_OK || SLOT(first,0x40,Map)(first,0,0,&a)!=S_OK || SLOT(second,0x40,Map)(second,0,0,&b)!=S_OK) return 7;
    const float vertices[36]={-1,-1,0,1,-1.0f/3,-1,0,1,-2.0f/3,1,0,1,-1.0f/3,-1,0,1,1.0f/3,-1,0,1,0,1,0,1,1.0f/3,-1,0,1,1,-1,0,1,2.0f/3,1,0,1};
    for(unsigned i=0;i<36;i++) ((float *)v)[i]=vertices[i];
    for(unsigned i=0;i<1024;i++) { ((unsigned char *)out)[i]=0x77; ((unsigned char *)target_data)[i]=0x30; }
    const unsigned char ta[8]={255,0,0,255,0,0,255,255},tb[12]={0,0,255,255,255,255,255,255,0,0,0,255};
    for(unsigned i=0;i<8;i++) ((unsigned char *)a)[i]=ta[i]; for(unsigned i=0;i<12;i++) ((unsigned char *)b)[i]=tb[i];
    void *heaps[5]={0}; CreateObject create_heap=SLOT(dev,0x70,CreateObject);
    for(unsigned i=0;i<5;i++) { HeapDesc hd={{i==0?2:(i<3?0:1),1,i==0?0:1,0},{0,0}}; if(create_heap(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 8; }
    U64 cpu[5],gpu[5]; for(unsigned i=0;i<5;i++) { cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]); }
    SLOT(dev,0xa0,View)(dev,target,0,cpu[0]); SLOT(dev,0x90,View)(dev,first,0,cpu[1]); SLOT(dev,0x90,View)(dev,second,0,cpu[2]);
    const unsigned point[13]={0,3,3,3,0,1,8,0,0,0,0,0,0},linear[13]={0x15,3,3,3,0,1,8,0,0,0,0,0,0};
    SLOT(dev,0xb0,Sampler)(dev,point,cpu[3]); SLOT(dev,0xb0,Sampler)(dev,linear,cpu[4]);
    D3D12_INPUT_ELEMENT_DESC element={"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0};
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=&element; *(unsigned *)(stream.b+168)=1;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    StreamDesc sd={sizeof(stream.b),stream.b}; if(SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 9;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if(ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 10;
    if(ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 11;
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),144,16}; float viewport[6]={0,0,9,4,0,1}; int scissor[4]={0,0,9,4};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
    SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&cpu[0],0,0);
    SetHeaps bind=SLOT(list,0x100,SetHeaps); SetTable table=SLOT(list,0x120,SetTable); SetConstant constant=SLOT(list,0x130,SetConstant); Draw draw=SLOT(list,0x60,Draw);
    void *pair[2]={heaps[1],heaps[3]}; bind(list,2,pair); table(list,1,gpu[3]); table(list,2,gpu[1]); constant(list,0,0x3ec00000,0); draw(list,3,1,0,0);
    pair[1]=heaps[4]; bind(list,2,pair); table(list,1,gpu[4]); table(list,2,gpu[1]); draw(list,3,1,3,0);
    pair[0]=heaps[2]; bind(list,2,pair); table(list,1,gpu[4]); table(list,2,gpu[2]); constant(list,0,0x3eaaaaab,0); draw(list,3,1,6,0);
    Barrier barrier={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&barrier);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=readback; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=9; dst.PlacedFootprint.Footprint.Height=4; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    if(ID3D12GraphicsCommandList_Close(list)!=S_OK) return 12;
    if(((volatile unsigned char *)out)[2*256+4]!=0x77) return 13;
    ((unsigned char *)b)[0]=0; ((unsigned char *)b)[1]=255; ((unsigned char *)b)[2]=0; ((unsigned char *)b)[3]=255;
    /* Recorded commands must retain descriptors, textures, buffers and pipeline. */
    SLOT(vb,0x10,Release)(vb); SLOT(readback,0x10,Release)(readback); SLOT(target,0x10,Release)(target); SLOT(first,0x10,Release)(first); SLOT(second,0x10,Release)(second); SLOT(pipeline,0x10,Release)(pipeline);
    for(unsigned i=0;i<5;i++) if(SLOT(heaps[i],0x10,Release)(heaps[i])==0 && i!=0) return 14;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if(ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 15;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    volatile unsigned char *p=out; unsigned x=2*256+4,y=2*256+16,z=2*256+28;
    if(p[x]!=255 || p[x+1]!=0 || p[x+2]!=0 || p[x+3]!=255) return 16;
    if(p[y]<190 || p[y]>192 || p[y+1]!=0 || p[y+2]<63 || p[y+2]>65 || p[y+3]!=255) return 17;
    if(p[z]<127 || p[z]>128 || p[z+1]!=255 || p[z+2]<127 || p[z+2]>128 || p[z+3]!=255) return 18;
    if(p[0]!=0x30 || p[36]!=0x77) return 19;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev); return 42;
}
