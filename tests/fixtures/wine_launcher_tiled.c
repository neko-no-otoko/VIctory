/* Ordinary Wine PE: sample independently specified GFX10 tiled RGBA texels. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <string.h>
#include <stddef.h>
#include "windows_d3d12x_uv_shaders.h"

__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef HRESULT (__stdcall *CreateResource)(void *, const void *, unsigned, const void *, unsigned, const void *, const IID *, void **);
typedef HRESULT (__stdcall *CreateObject)(void *, const void *, const IID *, void **);
typedef HRESULT (__stdcall *Map)(void *, unsigned, const void *, void **);
typedef U64 (__stdcall *Handle)(void *);
typedef void (__stdcall *View)(void *, void *, const void *, U64);
typedef void (__stdcall *Sampler)(void *, const void *, U64);
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
typedef struct { unsigned mip, x, y, offset; } Sample;
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),s,(DWORD)strlen(s),&n,0); }
/* Mesa addrlib GFX10 byte offsets, including non-tail blocks and mip tails;
   64KB_D vectors are also used by native_tiled64.rs. */
static const Sample s4[]={{0,1,1,0x2014},{0,63,60,0x5fcc},{1,1,1,0x1014},{2,0,0,0x800},{3,0,0,0x600},{6,0,0,0x300}};
static const Sample s64[]={{0,1,1,0x10014},{0,64,64,0x1c000},{0,127,124,0x1ffcc},{1,1,1,0x8014},{2,4,4,0x40c0},{3,7,7,0x20fc},{4,3,3,0x103c},{5,3,2,0x82c},{6,1,0,0x604},{7,0,0,0x500}};

int main(int argc, char **argv) {
    int invalid=0;
    if (argc==2) {
        if (!strcmp(argv[1],"--invalid-layout")) invalid=1;
        else if (!strcmp(argv[1],"--invalid-map")) invalid=2;
        else return 90;
    } else if (argc!=1) return 90;
    unsigned version=0x00020004; void *dev=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    ID3D12CommandAllocator *allocator=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 2;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 3;
    for (unsigned mode=0;mode<2;mode++) {
        const Sample *samples=mode?s64:s4; unsigned count=mode?sizeof(s64)/sizeof(*s64):sizeof(s4)/sizeof(*s4);
        unsigned width=mode?128:64, height=mode?125:61, levels=mode?8:7, size=mode?131072:24576;
        D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
        D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,72,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
        void *vb=0,*out=0,*target=0,*texture=0,*pipeline=0,*heaps[3]={0};
        if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 4;
        rd.Width=256; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK) return 5;
        rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=1; rd.Height=1; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
        if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 6;
        rd.Width=width; rd.Height=height; rd.MipLevels=levels; rd.Layout=(D3D12_TEXTURE_LAYOUT)(invalid==1?0x106:(mode?0x10a:0x105)); rd.Flags=0;
        if (invalid==1) say("D3D12X invalid tiled case armed\r\n");
        HRESULT hr=make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&texture);
        if (invalid==1) { if (FAILED(hr) && !texture) { say("D3D12X invalid tiled case rejected\r\n"); return 43; } return 91; }
        if (hr!=S_OK || !texture) return 7;
        void *v=0,*p=0,*t=0;
        if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK || SLOT(texture,0x40,Map)(texture,0,0,&t)!=S_OK || !t) return 8;
        if (invalid==2) {
            void *bad=0; say("D3D12X invalid tiled case armed\r\n");
            if (FAILED(SLOT(texture,0x40,Map)(texture,1,0,&bad)) && !bad) { say("D3D12X invalid tiled case rejected\r\n"); return 43; }
            return 92;
        }
        memset(t,0xcc,size);
        for (unsigned i=0;i<count;i++) {
            const Sample *s=&samples[i]; unsigned char *texel=(unsigned char *)t+s->offset;
            for (unsigned c=0;c<3;c++) texel[c]=(unsigned char)(10+c*20+s->mip*11+s->x*2+s->y);
            texel[3]=255;
        }
        for (unsigned i=0;i<3;i++) { HeapDesc hd={{i==0?2:(i==1?0:1),1,i==0?0:1,0},{0,0}}; if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 9; }
        U64 cpu[3],gpu[3]; for (unsigned i=0;i<3;i++) { cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]); }
        SLOT(dev,0xa0,View)(dev,target,0,cpu[0]);
        D3D12_INPUT_ELEMENT_DESC elements[2]={{"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},{"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
        union { U64 align; unsigned char b[224]; } stream={0};
        *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
        *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
        *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
        *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
        *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
        StreamDesc sd={sizeof(stream.b),stream.b};
        if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 10;
        D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24}; void *pair[2]={heaps[1],heaps[2]};
        for (unsigned i=0;i<count;i++) {
            const Sample *s=&samples[i]; unsigned mw=width>>s->mip, mh=height>>s->mip;
            if (!mw) mw=1; if (!mh) mh=1;
            float u=(s->x+.5f)/mw, w=(s->y+.5f)/mh;
            const float tri[18]={-1,-1,0,1,u,w, 3,-1,0,1,u,w, -1,3,0,1,u,w}; memcpy(v,tri,sizeof(tri)); memset(p,0x77,256);
            D3D12_SHADER_RESOURCE_VIEW_DESC srv={0}; srv.Format=DXGI_FORMAT_R8G8B8A8_UNORM; srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D; srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING; srv.Texture2D.MostDetailedMip=s->mip; srv.Texture2D.MipLevels=1;
            D3D12_SAMPLER_DESC sampler={0}; sampler.Filter=D3D12_FILTER_MIN_MAG_MIP_POINT; sampler.AddressU=sampler.AddressV=sampler.AddressW=D3D12_TEXTURE_ADDRESS_MODE_CLAMP; sampler.MaxAnisotropy=1; sampler.ComparisonFunc=D3D12_COMPARISON_FUNC_ALWAYS;
            SLOT(dev,0x90,View)(dev,texture,&srv,cpu[1]); SLOT(dev,0xb0,Sampler)(dev,&sampler,cpu[2]);
            ID3D12GraphicsCommandList *list=0;
            if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 11;
            if (i) { Barrier back={0,0,target,{0xffffffffULL | (0x800ULL<<32),4}}; SLOT(list,0xd0,BarrierFn)(list,1,&back); }
            float viewport[6]={0,0,1,1,0,1}; int scissor[4]={0,0,1,1};
            SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
            SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&cpu[0],0,0); SLOT(list,0x100,SetHeaps)(list,2,pair);
            SLOT(list,0x120,SetTable)(list,1,gpu[2]); SLOT(list,0x120,SetTable)(list,2,gpu[1]); SLOT(list,0x60,Draw)(list,3,1,0,0);
            Barrier barrier={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&barrier);
            D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT; dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=1; dst.PlacedFootprint.Footprint.Height=1; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256; src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
            SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
            if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 12;
            ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
            for (unsigned c=0;c<4;c++) { int expected=c==3?255:(unsigned char)(10+c*20+s->mip*11+s->x*2+s->y); int delta=(int)((volatile unsigned char *)p)[c]-expected; if (delta < -2 || delta > 2) return 20+mode; }
            for (unsigned b=4;b<256;b++) if (((volatile unsigned char *)p)[b]!=0x77) return 22;
            SLOT(list,0x10,Release)(list);
        }
        SLOT(vb,0x10,Release)(vb); SLOT(out,0x10,Release)(out); SLOT(target,0x10,Release)(target); SLOT(texture,0x10,Release)(texture); SLOT(pipeline,0x10,Release)(pipeline); for (unsigned i=0;i<3;i++) SLOT(heaps[i],0x10,Release)(heaps[i]);
    }
    SLOT(queue,0x10,Release)(queue); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev);
    say("D3D12X tiled sampling passed\r\n"); return 42;
}
