/* Ordinary Wine PE: typed RGBA/BGRA sampling, rendering, blending and copies. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <stdlib.h>
#include <string.h>
#include "windows_d3d12x_uv_shaders.h"

__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef HRESULT (__stdcall *CreateResource)(void *,const void *,unsigned,const void *,unsigned,const void *,const IID *,void **);
typedef HRESULT (__stdcall *CreateObject)(void *,const void *,const IID *,void **);
typedef HRESULT (__stdcall *Map)(void *,unsigned,const void *,void **);
typedef U64 (__stdcall *Handle)(void *);
typedef void (__stdcall *View)(void *,void *,const void *,U64);
typedef void (__stdcall *Sampler)(void *,const unsigned *,U64);
typedef void (__stdcall *Draw)(void *,unsigned,unsigned,unsigned,unsigned);
typedef void (__stdcall *SetTable)(void *,unsigned,U64);
typedef void (__stdcall *SetHeaps)(void *,unsigned,void **);
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
    if (argc!=3) return 90;
    unsigned format=(unsigned)atoi(argv[1]);
    if (format!=28 && format!=29 && format!=87 && format!=91) return 90;
    int bgra=format==87 || format==91, srgb=format==29 || format==91;
    int blend=!strcmp(argv[2],"blend") || !strcmp(argv[2],"blend-default");
    int sample=!strcmp(argv[2],"sample") || !strcmp(argv[2],"sample-default");
    int defaults=strstr(argv[2],"-default")!=0;
    int bad_resource=!strcmp(argv[2],"resource-typeless");
    int bad_srv=!strcmp(argv[2],"srv-mismatch") || !strcmp(argv[2],"srv-order") || !strcmp(argv[2],"srv-typeless");
    int bad_rtv=!strcmp(argv[2],"rtv-mismatch") || !strcmp(argv[2],"rtv-order") || !strcmp(argv[2],"rtv-typeless");
    int bad_pipeline=!strcmp(argv[2],"pipeline-mismatch") || !strcmp(argv[2],"pipeline-order");
    if (!sample && !blend && !bad_resource && !bad_srv && !bad_rtv && !bad_pipeline) return 90;
    unsigned typeless=bgra?90:27, other=srgb?(bgra?87:28):(bgra?91:29);
    unsigned target_format=sample?28:format;
    if (strstr(argv[2],"-order")) other=bgra?(srgb?29:28):(srgb?91:87);
    unsigned version=0x00020004; void *dev=0,*vb=0,*out=0,*upload=0,*target=0,*texture=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_TEXTURE2D,0,8,8,1,1,(DXGI_FORMAT)(bad_resource?typeless:format),{1,0},D3D12_TEXTURE_LAYOUT_UNKNOWN,0};
    if (bad_resource) say("D3D12X invalid format case armed\r\n");
    HRESULT hr=make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&texture);
    if (bad_resource) {
        if (FAILED(hr) && !texture) { say("D3D12X invalid format case rejected\r\n"); return 43; }
        return 91;
    }
    if (hr!=S_OK || !texture) return 2;
    rd.Format=(DXGI_FORMAT)target_format; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_BUFFER; rd.Width=72; rd.Height=1; rd.Format=DXGI_FORMAT_UNKNOWN; rd.Layout=D3D12_TEXTURE_LAYOUT_ROW_MAJOR; rd.Flags=0;
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 4;
    rd.Width=4096; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK) return 5;
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&upload)!=S_OK) return 6;
    void *v=0,*p=0,*t=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK || SLOT(upload,0x40,Map)(upload,0,0,&t)!=S_OK) return 7;
    const float tri[18]={-1,-1,0,1, .5f,.5f, 3,-1,0,1, .5f,.5f, -1,3,0,1, .5f,.5f};
    memcpy(v,tri,sizeof(tri)); memset(p,0x77,4096); memset(t,0xa5,4096);
    const unsigned char color[4]={180,90,35,128};
    for (unsigned y=0;y<8;y++) for (unsigned x=0;x<8;x++) for (unsigned c=0;c<4;c++)
        ((unsigned char *)t)[y*256+x*4+c]=color[bgra && c==0?2:bgra && c==2?0:c];
    void *heaps[3]={0}; U64 cpu[3],gpu[3];
    for (unsigned i=0;i<3;i++) {
        HeapDesc hd={{i==0?2:(i==1?0:1),1,i==0?0:1,0},{0,0}};
        if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 8;
        cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]);
    }
    D3D12_RENDER_TARGET_VIEW_DESC rtv={0}; rtv.Format=(DXGI_FORMAT)(bad_rtv?(!strcmp(argv[2],"rtv-typeless")?typeless:other):target_format); rtv.ViewDimension=D3D12_RTV_DIMENSION_TEXTURE2D;
    if (bad_rtv) say("D3D12X invalid format case armed\r\n");
    SLOT(dev,0xa0,View)(dev,target,defaults?0:&rtv,cpu[0]);
    if (bad_rtv) return 92; /* CreateRenderTargetView is void and must abort. */
    D3D12_SHADER_RESOURCE_VIEW_DESC srv={0};
    srv.Format=(DXGI_FORMAT)(bad_srv?(!strcmp(argv[2],"srv-typeless")?typeless:other):format);
    srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D; srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING; srv.Texture2D.MipLevels=1;
    if (bad_srv) say("D3D12X invalid format case armed\r\n");
    SLOT(dev,0x90,View)(dev,texture,defaults?0:&srv,cpu[1]);
    if (bad_srv) return 93; /* CreateShaderResourceView is void and must abort. */
    const unsigned linear[13]={0x15,3,3,3,0,1,8,0,0,0,0,0,0}; SLOT(dev,0xb0,Sampler)(dev,linear,cpu[2]);
    D3D12_INPUT_ELEMENT_DESC elements[2]={{"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},{"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
    union { U64 align; unsigned char b[560]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=bad_pipeline?other:target_format; *(unsigned *)(stream.b+220)=1;
    if (blend) {
        *(unsigned *)(stream.b+224)=8; *(unsigned *)(stream.b+236)=1;
        for (unsigned i=0;i<6;i++) *(unsigned *)(stream.b+244+4*i)=(unsigned[]){2,2,1,2,2,1}[i];
        stream.b[272]=15;
    }
    StreamDesc sd={blend?sizeof(stream.b):224,stream.b};
    if (bad_pipeline) say("D3D12X invalid format case armed\r\n");
    hr=SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline);
    if (bad_pipeline && FAILED(hr) && !pipeline) { say("D3D12X invalid format case rejected\r\n"); return 43; }
    if (hr!=S_OK || !pipeline) return 9;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 10;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 11;
    D3D12_TEXTURE_COPY_LOCATION us={0},ud={0}; us.pResource=upload; us.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    us.PlacedFootprint.Footprint.Format=(DXGI_FORMAT)format; us.PlacedFootprint.Footprint.Width=8; us.PlacedFootprint.Footprint.Height=8; us.PlacedFootprint.Footprint.Depth=1; us.PlacedFootprint.Footprint.RowPitch=256;
    ud.pResource=texture; ud.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&ud,0,0,0,&us,0);
    Barrier b={0,0,texture,{0xffffffffULL | (0x400ULL<<32),0x80}}; SLOT(list,0xd0,BarrierFn)(list,1,&b);
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24}; float viewport[6]={0,0,8,8,0,1}; int scissor[4]={0,0,8,8};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
    SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&cpu[0],0,0);
    if (blend) { const float clear[4]={.125f,.25f,.5f,.25f}; SLOT(list,0x1a0,Clear)(list,cpu[0],clear,0,0); }
    void *pair[2]={heaps[1],heaps[2]}; SLOT(list,0x100,SetHeaps)(list,2,pair);
    SLOT(list,0x120,SetTable)(list,1,gpu[2]); SLOT(list,0x120,SetTable)(list,2,gpu[1]); SLOT(list,0x60,Draw)(list,3,1,0,0);
    Barrier tr={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&tr);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=(DXGI_FORMAT)target_format; dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    Barrier texread={0,0,texture,{0xffffffffULL | (0x80ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&texread);
    dst.PlacedFootprint.Offset=2048; dst.PlacedFootprint.Footprint.Format=(DXGI_FORMAT)format; src.pResource=texture; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    hr=ID3D12GraphicsCommandList_Close(list);
    if (bad_pipeline) { if (FAILED(hr)) { say("D3D12X invalid format case rejected\r\n"); return 43; } return 94; }
    if (hr!=S_OK) return 12;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 13;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    const int expected[2][2][4]={{{180,90,35,128},{212,154,162,192}},{{116,26,4,128},{201,160,190,192}}};
    volatile unsigned char *pixels=p;
    for (unsigned y=0;y<8;y++) {
        for (unsigned x=0;x<8;x++) for (unsigned c=0;c<4;c++) {
            unsigned k=blend && bgra && c==0?2:blend && bgra && c==2?0:c;
            int delta=(int)pixels[y*256+x*4+c]-expected[srgb][blend][k];
            if (delta < -2 || delta > 2) return 20;
            unsigned source=bgra && c==0?2:bgra && c==2?0:c;
            if (pixels[2048+y*256+x*4+c]!=color[source]) return 21;
        }
        for (unsigned x=32;x<256;x++) if (pixels[y*256+x]!=0x77 || pixels[2048+y*256+x]!=0x77) return 22;
    }
    say("D3D12X typed format passed\r\n"); return 42;
}
