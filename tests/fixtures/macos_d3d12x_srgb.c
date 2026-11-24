/* Win64 PE: native Metal sRGB sampling, clears, rendering and blending. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include "shaders.h"
#include "reference.h"
__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef HRESULT (__stdcall *CreateResource)(void *,const void *,unsigned,const void *,unsigned,const void *,const IID *,void **);
typedef HRESULT (__stdcall *CreateObject)(void *,const void *,const IID *,void **);
typedef HRESULT (__stdcall *Map)(void *,unsigned,const void *,void **);
typedef U64 (__stdcall *Handle)(void *);
typedef void (__stdcall *View)(void *,void *,const void *,U64);
typedef void (__stdcall *Sampler)(void *,const void *,U64);
typedef void (__stdcall *Draw)(void *,unsigned,unsigned,unsigned,unsigned);
typedef void (__stdcall *Constant)(void *,unsigned,unsigned,unsigned);
typedef void (__stdcall *SetPipeline)(void *,void *);
typedef void (__stdcall *SetTable)(void *,unsigned,U64);
typedef void (__stdcall *SetHeaps)(void *,unsigned,void **);
typedef void (__stdcall *Topology)(void *,unsigned);
typedef void (__stdcall *SetArray)(void *,unsigned,const void *);
typedef void (__stdcall *Vertices)(void *,unsigned,unsigned,const void *);
typedef void (__stdcall *Targets)(void *,unsigned,const U64 *,int,const U64 *);
typedef void (__stdcall *Clear)(void *,U64,const float *,unsigned,const void *);
typedef void (__stdcall *Copy)(void *,const void *,unsigned,unsigned,unsigned,const void *,const void *);
typedef unsigned (__stdcall *BarrierFn)(void *,unsigned,const void *);
typedef unsigned (__stdcall *Release)(void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind,flags; void *resource; U64 data[2]; } Barrier;
void *memset(void *dst,int value,size_t len) { volatile unsigned char *p=dst; for(size_t i=0;i<len;i++) p[i]=(unsigned char)value; return dst; }
void *memcpy(void *dst,const void *src,size_t len) { volatile unsigned char *d=dst; const volatile unsigned char *s=src; for(size_t i=0;i<len;i++) d[i]=s[i]; return dst; }
static int marker=7; static int *volatile relocated=&marker;
static void setup(ID3D12GraphicsCommandList *l,const D3D12_VERTEX_BUFFER_VIEW *vb,U64 rtv,unsigned width,unsigned height) {
    float vp[6]={0,0,(float)width,(float)height,0,1}; int sc[4]={0,0,(int)width,(int)height};
    SLOT(l,0xa0,Topology)(l,4); SLOT(l,0xa8,SetArray)(l,1,vp); SLOT(l,0xb0,SetArray)(l,1,sc);
    SLOT(l,0x180,Vertices)(l,0,1,vb); SLOT(l,0x190,Targets)(l,1,&rtv,0,0);
}
static void copy_target(ID3D12GraphicsCommandList *l,void *target,void *out,unsigned format,unsigned width,unsigned height) {
    Barrier b={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(l,0xd0,BarrierFn)(l,1,&b);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=(DXGI_FORMAT)format; dst.PlacedFootprint.Footprint.Width=width; dst.PlacedFootprint.Footprint.Height=height; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(l,0x80,Copy)(l,&dst,0,0,0,&src,0);
}
static int near_byte(int a,int b) { int d=a-b; return d>=-2 && d<=2; }
static void set_color(ID3D12GraphicsCommandList *l,const float *color) {
    for(unsigned c=0;c<4;c++) { union { float f; unsigned u; } v; v.f=color[c]; SLOT(l,0x130,Constant)(l,3,v.u,c); }
}
static void draw_column(ID3D12GraphicsCommandList *l,int x,const float *color) {
    int sc[4]={x,0,x+1,8}; SLOT(l,0xb0,SetArray)(l,1,sc); set_color(l,color); SLOT(l,0x60,Draw)(l,3,1,0,0);
}
unsigned mainCRTStartup(void) {
    if (*relocated!=7) return 99;
    unsigned version=0x00020004; void *dev=0;
    if(D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    ID3D12CommandAllocator *alloc=0; ID3D12CommandQueue *queue=0;
    if(ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&alloc)!=S_OK) return 2;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if(ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 3;
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_TEXTURE2D,0,4,4,1,3,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    const unsigned formats[8]={28,29,87,91,71,72,77,78};
    const unsigned typeless[8]={27,27,90,90,70,70,76,76};
    /* All typed UNORM and sRGB resources must reject typeless resources and views. */
    for(unsigned test=0;test<8;test++) {
        unsigned format=formats[test], is_srgb=test&1, bgra=(test==2 || test==3), bc=test>=4, bc3=test>=6;
        void *vb=0,*out=0,*target=0,*texture=0,*pipeline=0,*heaps[3]={0},*badres=0;
        rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=4; rd.Height=4; rd.MipLevels=3; rd.Format=(DXGI_FORMAT)typeless[test]; rd.Flags=0;
        if(make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&badres)==S_OK) return 4;
        rd.Format=(DXGI_FORMAT)format; if(make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&texture)!=S_OK) return 5;
        rd.Dimension=D3D12_RESOURCE_DIMENSION_BUFFER; rd.Width=72; rd.Height=1; rd.MipLevels=1; rd.Format=DXGI_FORMAT_UNKNOWN;
        if(make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 6;
        rd.Width=256; if(make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&out)!=S_OK) return 7;
        rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=1; rd.Height=1; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
        if(make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 8;
        void *v=0,*p=0; if(SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK) return 9;
        const float tri[18]={-1,-1,0,1,.5f,.5f, 3,-1,0,1,.5f,.5f, -1,3,0,1,.5f,.5f};
        for(unsigned i=0;i<18;i++) ((float *)v)[i]=tri[i];
        for(unsigned mip=0;mip<3;mip++) {
            void *t=0; if(SLOT(texture,0x40,Map)(texture,mip,0,&t)!=S_OK || !t) return 10;
            unsigned e=endpoints[mip], r=(e>>11)&31, g=(e>>5)&63, b=e&31; r=(r<<3)|(r>>2); g=(g<<2)|(g>>4); b=(b<<3)|(b>>2);
            if(bc) {
                unsigned char *q=t; if(bc3) { q[0]=123; q[1]=123; for(unsigned j=2;j<8;j++) q[j]=0; q+=8; }
                q[0]=(unsigned char)e; q[1]=(unsigned char)(e>>8); q[2]=q[0]; q[3]=q[1]; for(unsigned j=4;j<8;j++) q[j]=0;
            } else {
                unsigned side=4>>mip; for(unsigned y=0;y<side;y++) for(unsigned x=0;x<side;x++) {
                    unsigned char *q=(unsigned char *)t+y*256+x*4; q[0]=(unsigned char)(bgra?b:r); q[1]=(unsigned char)g; q[2]=(unsigned char)(bgra?r:b); q[3]=123;
                }
            }
        }
        for(unsigned i=0;i<3;i++) { HeapDesc hd={{i==0?2:(i==1?0:1),1,i==0?0:1,0},{0,0}}; if(create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 11; }
        U64 cpu[3],gpu[3]; for(unsigned i=0;i<3;i++) { cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]); }
        SLOT(dev,0xa0,View)(dev,target,0,cpu[0]);
        D3D12_SHADER_RESOURCE_VIEW_DESC srv={0}; srv.Format=(DXGI_FORMAT)format; srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D; srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING; srv.Texture2D.MipLevels=3;
        D3D12_SAMPLER_DESC sampler={0}; sampler.Filter=D3D12_FILTER_MIN_MAG_MIP_POINT; sampler.AddressU=sampler.AddressV=sampler.AddressW=D3D12_TEXTURE_ADDRESS_MODE_CLAMP; sampler.MaxAnisotropy=1; sampler.ComparisonFunc=D3D12_COMPARISON_FUNC_ALWAYS;
        D3D12_INPUT_ELEMENT_DESC elements[2]={{"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},{"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
        union { U64 align; unsigned char b[224]; } stream={0};
        *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs); *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
        *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1; *(unsigned *)(stream.b+96)=11;
        *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2; *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
        StreamDesc sd={sizeof(stream.b),stream.b}; if(SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 12;
        D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24}; void *pair[2]={heaps[1],heaps[2]};
        for(unsigned bad=0;bad<2;bad++) {
            srv.Format=(DXGI_FORMAT)(bad?typeless[test]:formats[test^1]); SLOT(dev,0x90,View)(dev,texture,&srv,cpu[1]); SLOT(dev,0xb0,Sampler)(dev,&sampler,cpu[2]);
            ID3D12GraphicsCommandList *l=0; if(ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,alloc,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&l)!=S_OK) return 13;
            setup(l,&view,cpu[0],1,1); SLOT(l,0x100,SetHeaps)(l,2,pair); SLOT(l,0x120,SetTable)(l,1,gpu[2]); SLOT(l,0x120,SetTable)(l,2,gpu[1]); SLOT(l,0x60,Draw)(l,3,1,0,0);
            if(ID3D12GraphicsCommandList_Close(l)!=E_NOTIMPL) return 14; SLOT(l,0x10,Release)(l);
        }
        srv.Format=(DXGI_FORMAT)format;
        for(unsigned iteration=0;iteration<6;iteration++) {
            unsigned mip=iteration/2; int explicit_view=iteration&1;
            srv.Texture2D.MostDetailedMip=explicit_view?mip:0; srv.Texture2D.MipLevels=explicit_view?1:3;
            sampler.MinLOD=sampler.MaxLOD=(float)(explicit_view?0:mip);
            SLOT(dev,0x90,View)(dev,texture,explicit_view?&srv:0,cpu[1]); SLOT(dev,0xb0,Sampler)(dev,&sampler,cpu[2]);
            for(unsigned i=0;i<256;i++) ((unsigned char *)p)[i]=0x77;
            ID3D12GraphicsCommandList *l=0; if(ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,alloc,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&l)!=S_OK) return 15;
            if(iteration) { Barrier back={0,0,target,{0xffffffffULL | (0x800ULL<<32),4}}; SLOT(l,0xd0,BarrierFn)(l,1,&back); }
            setup(l,&view,cpu[0],1,1); SLOT(l,0x100,SetHeaps)(l,2,pair); SLOT(l,0x120,SetTable)(l,1,gpu[2]); SLOT(l,0x120,SetTable)(l,2,gpu[1]); SLOT(l,0x60,Draw)(l,3,1,0,0);
            copy_target(l,target,out,28,1,1); if(ID3D12GraphicsCommandList_Close(l)!=S_OK) return 16;
            if(iteration==5) { SLOT(vb,0x10,Release)(vb); SLOT(out,0x10,Release)(out); SLOT(target,0x10,Release)(target); SLOT(texture,0x10,Release)(texture); SLOT(pipeline,0x10,Release)(pipeline); for(unsigned i=0;i<3;i++) SLOT(heaps[i],0x10,Release)(heaps[i]); }
            ID3D12CommandList *lists[]={(ID3D12CommandList *)l}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
            for(unsigned c=0;c<3;c++) if(!near_byte(((volatile unsigned char *)p)[c],sample_expected[is_srgb][mip][c])) return 17+test;
            if(!near_byte(((volatile unsigned char *)p)[3],bc && !bc3?255:123)) return 25;
            for(unsigned i=4;i<256;i++) if(((volatile unsigned char *)p)[i]!=0x77) return 26;
            SLOT(l,0x10,Release)(l);
        }
    }
    /* Both native sRGB target orders: full and rect clear, direct shader output,
       and two ordered source-alpha blends. Reference bytes use IEC 61966-2-1. */
    for(unsigned test=0;test<2;test++) {
        unsigned format=test?91:29; void *vb=0,*out=0,*target=0,*heap=0,*pipelines[2]={0};
        rd.Dimension=D3D12_RESOURCE_DIMENSION_BUFFER; rd.Width=48; rd.Height=1; rd.MipLevels=1; rd.Format=DXGI_FORMAT_UNKNOWN; rd.Flags=0;
        if(make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 27;
        rd.Width=2048; if(make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&out)!=S_OK) return 28;
        rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=(DXGI_FORMAT)format; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
        if(make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 29;
        void *v=0,*p=0; if(SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK) return 30;
        const float tri[12]={-1,-1,0,1,3,-1,0,1,-1,3,0,1}; for(unsigned i=0;i<12;i++) ((float *)v)[i]=tri[i]; for(unsigned i=0;i<2048;i++) ((unsigned char *)p)[i]=0x77;
        HeapDesc hd={{2,1,0,0},{0,0}}; if(create(dev,&hd,&IID_ID3D12DescriptorHeap,&heap)!=S_OK) return 31;
        U64 rtv=SLOT(heap,0x48,Handle)(heap); D3D12_RENDER_TARGET_VIEW_DESC desc={0}; desc.Format=(DXGI_FORMAT)format; desc.ViewDimension=D3D12_RTV_DIMENSION_TEXTURE2D;
        D3D12_INPUT_ELEMENT_DESC element={"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0};
        union { U64 align; unsigned char b[560]; } stream={0}; unsigned char *cs=stream.b;
        *(unsigned *)(cs+0)=1; *(const void **)(cs+8)=color_vs; *(U64 *)(cs+16)=sizeof(color_vs); *(unsigned *)(cs+24)=2; *(const void **)(cs+32)=color_ps; *(U64 *)(cs+40)=sizeof(color_ps);
        *(unsigned *)(cs+48)=10; *(unsigned *)(cs+52)=3; *(unsigned *)(cs+56)=1; *(unsigned *)(cs+76)=1; *(unsigned *)(cs+96)=12; *(void **)(cs+104)=&element; *(unsigned *)(cs+112)=1;
        *(unsigned *)(cs+120)=14; *(unsigned *)(cs+124)=3; *(unsigned *)(cs+128)=15; *(unsigned *)(cs+132)=format; *(unsigned *)(cs+164)=1;
        *(unsigned *)(cs+168)=8; *(unsigned *)(cs+180)=1; *(unsigned *)(cs+188)=5; *(unsigned *)(cs+192)=6; *(unsigned *)(cs+196)=1; *(unsigned *)(cs+200)=2; *(unsigned *)(cs+204)=6; *(unsigned *)(cs+208)=1; cs[216]=15; *(unsigned *)(cs+504)=11;
        StreamDesc sd={sizeof(stream.b),stream.b}; if(SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipelines[0])!=S_OK) return 32;
        *(unsigned *)(cs+180)=0; if(SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipelines[1])!=S_OK) return 33;
        D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),48,16};
        for(unsigned bad=0;bad<3;bad++) {
            void *wrong=0,*use=pipelines[1]; desc.Format=(DXGI_FORMAT)(bad==0?(test?87:28):bad==1?(test?90:27):format); SLOT(dev,0xa0,View)(dev,target,&desc,rtv);
            if(bad==2) { *(unsigned *)(cs+132)=test?87:28; if(SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&wrong)!=S_OK) return 34; *(unsigned *)(cs+132)=format; use=wrong; }
            ID3D12GraphicsCommandList *l=0; if(ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,alloc,use,&IID_ID3D12GraphicsCommandList,(void **)&l)!=S_OK) return 35;
            setup(l,&view,rtv,8,8); const float c[4]={.25f,.5f,.75f,.375f}; draw_column(l,0,c);
            if(ID3D12GraphicsCommandList_Close(l)!=E_NOTIMPL) return 36; SLOT(l,0x10,Release)(l); if(wrong) SLOT(wrong,0x10,Release)(wrong);
        }
        desc.Format=(DXGI_FORMAT)format; SLOT(dev,0xa0,View)(dev,target,test?&desc:0,rtv);
        ID3D12GraphicsCommandList *l=0; if(ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,alloc,pipelines[1],&IID_ID3D12GraphicsCommandList,(void **)&l)!=S_OK) return 37;
        const float full[4]={.0023f,.3487f,.7023f,.4137f}, rect_color[4]={.0013f,.6147f,.0893f,.6237f}; int rect[4]={1,0,2,8};
        SLOT(l,0x1a0,Clear)(l,rtv,full,0,0); SLOT(l,0x1a0,Clear)(l,rtv,rect_color,1,rect); setup(l,&view,rtv,8,8);
        const float direct[4]={.25f,.5f,.75f,.375f}, red[4]={.75f,.125f,.25f,.5f}, green[4]={.125f,.75f,.375f,.5f};
        draw_column(l,2,direct); SLOT(l,0xc8,SetPipeline)(l,pipelines[0]); draw_column(l,3,red); draw_column(l,3,green); draw_column(l,4,green); draw_column(l,4,red);
        copy_target(l,target,out,format,8,8); if(ID3D12GraphicsCommandList_Close(l)!=S_OK) return 38;
        SLOT(vb,0x10,Release)(vb); SLOT(out,0x10,Release)(out); SLOT(target,0x10,Release)(target); SLOT(heap,0x10,Release)(heap); for(unsigned i=0;i<2;i++) SLOT(pipelines[i],0x10,Release)(pipelines[i]);
        ID3D12CommandList *lists[]={(ID3D12CommandList *)l}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
        for(unsigned y=0;y<8;y++) { for(unsigned x=0;x<8;x++) for(unsigned c=0;c<4;c++) { unsigned k=test && c==0?2:test && c==2?0:c; if(!near_byte(((volatile unsigned char *)p)[y*256+x*4+c],target_expected[x<5?x:0][k])) return 39+test; } for(unsigned x=32;x<256;x++) if(((volatile unsigned char *)p)[y*256+x]!=0x77) return 41; }
        SLOT(l,0x10,Release)(l);
    }
    SLOT(queue,0x10,Release)(queue); SLOT(alloc,0x10,Release)(alloc); SLOT(dev,0x10,Release)(dev); return 42;
}
