/* Ordinary Wine PE: sampler mip selection, SRV ranges and independent samplers. */
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
typedef HRESULT (__stdcall *CreateResource)(void *,const void *,unsigned,const void *,unsigned,const void *,const IID *,void **);
typedef HRESULT (__stdcall *CreateObject)(void *,const void *,const IID *,void **);
typedef HRESULT (__stdcall *Map)(void *,unsigned,const void *,void **);
typedef U64 (__stdcall *Handle)(void *);
typedef unsigned (__stdcall *Increment)(void *,unsigned);
typedef void (__stdcall *View)(void *,void *,const void *,U64);
typedef void (__stdcall *Sampler)(void *,const unsigned *,U64);
typedef void (__stdcall *Draw)(void *,unsigned,unsigned,unsigned,unsigned);
typedef void (__stdcall *SetTable)(void *,unsigned,U64);
typedef void (__stdcall *SetHeaps)(void *,unsigned,void **);
typedef void (__stdcall *Topology)(void *,unsigned);
typedef void (__stdcall *SetArray)(void *,unsigned,const void *);
typedef void (__stdcall *Vertices)(void *,unsigned,unsigned,const void *);
typedef void (__stdcall *Targets)(void *,unsigned,const U64 *,int,const U64 *);
typedef void (__stdcall *Copy)(void *,const void *,unsigned,unsigned,unsigned,const void *,const void *);
typedef unsigned (__stdcall *BarrierFn)(void *,unsigned,const void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind,flags; void *resource; U64 data[2]; } Barrier;
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),s,(DWORD)strlen(s),&n,0); }
static unsigned bits(float f) { unsigned u; memcpy(&u,&f,4); return u; }

int main(int argc,char **argv) {
    if (argc!=2) return 90;
    const char *which=argv[1];
    int invalid=!strncmp(which,"--invalid-",10) || !strcmp(which,"--bias-reject");
    int probe=!strcmp(which,"--explicit-clamp-probe");
    unsigned shader=0, filter[2]={0,0}, first[2]={0,0}, count[2]={3,3};
    float minlod[2]={0,0}, maxlod[2]={2,2}, bias[2]={0,0};
    int high_gradient=0, expected[4]={240,16,240,16};
    if (!strcmp(which,"--point-low")) { shader=1; }
    else if (!strcmp(which,"--point-high")) { shader=2; expected[0]=expected[2]=16; expected[1]=expected[3]=240; }
    else if (!strcmp(which,"--linear")) { shader=3; filter[0]=1; filter[1]=0x15; expected[0]=expected[1]=expected[2]=expected[3]=128; }
    else if (!strcmp(which,"--independent")) { shader=1; filter[0]=1; filter[1]=0x14; expected[0]=184; expected[1]=72; }
    else if (!strcmp(which,"--view")) { shader=4; first[0]=1; count[0]=2; count[1]=1; expected[0]=48; expected[1]=48; }
    else if (!strcmp(which,"--min")) { minlod[0]=minlod[1]=1; expected[0]=expected[2]=16; expected[1]=expected[3]=240; }
    else if (!strcmp(which,"--independent-clamps")) { minlod[0]=1; expected[0]=16; expected[1]=240; }
    else if (!strcmp(which,"--max")) { high_gradient=1; maxlod[0]=maxlod[1]=1; expected[0]=expected[2]=16; expected[1]=expected[3]=240; }
    else if (!strcmp(which,"--min-fraction")) { filter[0]=filter[1]=1; minlod[0]=minlod[1]=0.5f; expected[0]=expected[1]=expected[2]=expected[3]=128; }
    else if (!strcmp(which,"--max-fraction")) { filter[0]=filter[1]=1; high_gradient=1; maxlod[0]=maxlod[1]=1.5f; expected[0]=expected[2]=32; expected[1]=expected[3]=144; }
    else if (probe) { shader=1; minlod[0]=minlod[1]=1; }
    else if (!strcmp(which,"--bias-reject")) { bias[0]=bias[1]=1; }
    else if (!invalid) return 90;
    if (invalid) say("D3D12X invalid sampler LOD armed\r\n");
    if (!strcmp(which,"--invalid-negative-min")) minlod[0]=-1;
    else if (!strcmp(which,"--invalid-negative-max")) maxlod[0]=-1;
    else if (!strcmp(which,"--invalid-reversed")) { minlod[0]=2; maxlod[0]=1; }
    else if (!strcmp(which,"--invalid-nan")) { unsigned u=0x7fc00000; memcpy(&minlod[0],&u,4); }
    else if (!strcmp(which,"--invalid-infinity")) { unsigned u=0x7f800000; memcpy(&maxlod[0],&u,4); }
    else if (!strcmp(which,"--invalid-bias-step")) bias[0]=0.01f;
    else if (!strcmp(which,"--invalid-bias-low")) bias[0]=-16.015625f;
    else if (!strcmp(which,"--invalid-bias-high")) bias[0]=16;
    else if (!strcmp(which,"--invalid-filter")) filter[0]=4;
    else if (invalid && strcmp(which,"--bias-reject")) return 90;
    unsigned version=0x00020004; void *dev=0,*vb=0,*out=0,*upload=0,*target=0,*texture=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,72,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=4096; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK || make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&upload)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 4;
    rd.MipLevels=3; rd.Flags=0; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&texture)!=S_OK) return 5;
    void *v=0,*p=0,*t=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK || SLOT(upload,0x40,Map)(upload,0,0,&t)!=S_OK) return 6;
    const float tri_low[18]={-1,-1,0,1, .25f,.25f, 3,-1,0,1, .25f,.25f, -1,3,0,1, .25f,.25f};
    const float tri_high[18]={-1,-1,0,1, 0,0, 3,-1,0,1, 8,0, -1,3,0,1, 0,8};
    memcpy(v,high_gradient?tri_high:tri_low,72); memset(p,0x77,4096); memset(t,0,4096);
    const unsigned sizes[3]={8,4,2}, offsets[3]={0,2048,3072};
    const unsigned char colors[3][4]={{240,16,0,255},{16,240,0,255},{48,48,0,255}};
    for (unsigned m=0;m<3;m++) for (unsigned y=0;y<sizes[m];y++) for (unsigned x=0;x<sizes[m];x++) memcpy((char *)t+offsets[m]+y*256+x*4,colors[m],4);
    void *heaps[3]={0}; U64 cpu[3],gpu[3];
    for (unsigned i=0;i<3;i++) {
        HeapDesc hd={{i==0?2:i==1?0:1,i==0?1:2,i==0?0:1,0},{0,0}};
        if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 7;
        cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]);
    }
    unsigned ts=SLOT(dev,0x78,Increment)(dev,0), ss=SLOT(dev,0x78,Increment)(dev,1); if (!ts || !ss) return 8;
    SLOT(dev,0xa0,View)(dev,target,0,cpu[0]);
    for (unsigned i=0;i<2;i++) {
        D3D12_SHADER_RESOURCE_VIEW_DESC srv={0}; srv.Format=DXGI_FORMAT_R8G8B8A8_UNORM; srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D; srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
        srv.Texture2D.MostDetailedMip=first[i]; srv.Texture2D.MipLevels=count[i]; SLOT(dev,0x90,View)(dev,texture,&srv,cpu[1]+i*ts);
        unsigned desc[13]={0,3,3,3,0,1,8,0,0,0,0,0,0}; desc[0]=filter[i]; desc[4]=bits(bias[i]); desc[11]=bits(minlod[i]); desc[12]=bits(maxlod[i]); SLOT(dev,0xb0,Sampler)(dev,desc,cpu[2]+i*ss);
    }
    D3D12_INPUT_ELEMENT_DESC elements[2]={{"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},{"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
    const unsigned char *shaders[]={ps_implicit,ps_quarter,ps_three_quarter,ps_half,ps_one};
    const size_t lengths[]={sizeof(ps_implicit),sizeof(ps_quarter),sizeof(ps_three_quarter),sizeof(ps_half),sizeof(ps_one)};
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=shaders[shader]; *(U64 *)(stream.b+40)=lengths[shader];
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    StreamDesc sd={sizeof(stream.b),stream.b}; if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 9;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK || ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 10;
    for (unsigned m=0;m<3;m++) {
        D3D12_TEXTURE_COPY_LOCATION src={0},dst={0}; src.pResource=upload; src.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT; src.PlacedFootprint.Offset=offsets[m]; src.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; src.PlacedFootprint.Footprint.Width=sizes[m]; src.PlacedFootprint.Footprint.Height=sizes[m]; src.PlacedFootprint.Footprint.Depth=1; src.PlacedFootprint.Footprint.RowPitch=256;
        dst.pResource=texture; dst.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; dst.SubresourceIndex=m; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    }
    Barrier b={0,0,texture,{0xffffffffULL | (0x400ULL<<32),0x80}}; SLOT(list,0xd0,BarrierFn)(list,1,&b);
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24}; float viewport[6]={0,0,8,8,0,1}; int scissor[4]={0,0,8,8};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor); SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&cpu[0],0,0);
    void *pair[2]={heaps[1],heaps[2]}; SLOT(list,0x100,SetHeaps)(list,2,pair);
    SLOT(list,0x120,SetTable)(list,1,gpu[2]); SLOT(list,0x120,SetTable)(list,2,gpu[1]); SLOT(list,0x120,SetTable)(list,3,gpu[2]+ss); SLOT(list,0x120,SetTable)(list,4,gpu[1]+ts); SLOT(list,0x60,Draw)(list,3,1,0,0);
    Barrier rb={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&rb);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT; dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256; src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 11;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 12;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    if (invalid) return 93;
    volatile unsigned char *pixels=p; char msg[160];
    snprintf(msg,sizeof(msg),"D3D12X sampler LOD result %u %u %u %u\r\n",pixels[3*256+3*4],pixels[3*256+3*4+1],pixels[3*256+3*4+2],pixels[3*256+3*4+3]); say(msg);
    if (!probe) for (unsigned c=0;c<4;c++) { int got=pixels[3*256+3*4+c], delta=got-expected[c]; if (delta < -2 || delta > 2) { snprintf(msg,sizeof(msg),"LOD %s channel %u got %d expected %d\r\n",which,c,got,expected[c]); say(msg); return 20+c; } }
    say("D3D12X sampler LOD passed\r\n"); return 42;
}
