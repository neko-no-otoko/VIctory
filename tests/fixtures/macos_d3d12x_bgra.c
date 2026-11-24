/* Win64 PE: BGRA/RGBA texture sampling, rendering and pitched byte readback. */
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
typedef void (__stdcall *Constant)(void *, unsigned, unsigned, unsigned);
typedef void (__stdcall *SetPipeline)(void *, void *);
typedef void (__stdcall *SetTable)(void *, unsigned, U64);
typedef void (__stdcall *SetHeaps)(void *, unsigned, void **);
typedef void (__stdcall *Topology)(void *, unsigned);
typedef void (__stdcall *SetArray)(void *, unsigned, const void *);
typedef void (__stdcall *Vertices)(void *, unsigned, unsigned, const void *);
typedef void (__stdcall *Targets)(void *, unsigned, const U64 *, int, const U64 *);
typedef void (__stdcall *Clear)(void *, U64, const float *, unsigned, const void *);
typedef void (__stdcall *Copy)(void *, const void *, unsigned, unsigned, unsigned, const void *, const void *);
typedef unsigned (__stdcall *BarrierFn)(void *, unsigned, const void *);
typedef unsigned (__stdcall *Release)(void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind, flags; void *resource; U64 data[2]; } Barrier;
void *memset(void *dst, int value, size_t len) { volatile unsigned char *p=dst; for (size_t i=0;i<len;i++) p[i]=(unsigned char)value; return dst; }
void *memcpy(void *dst, const void *src, size_t len) { volatile unsigned char *d=dst; const volatile unsigned char *s=src; for (size_t i=0;i<len;i++) d[i]=s[i]; return dst; }
static int marker=7; static int *volatile relocated=&marker;

static void setup(ID3D12GraphicsCommandList *l, const D3D12_VERTEX_BUFFER_VIEW *v, U64 rtv, void **pair, U64 srv, U64 sampler) {
    float viewport[6]={0,0,8,8,0,1}; int scissor[4]={1,0,7,8};
    SLOT(l,0xa0,Topology)(l,4); SLOT(l,0xa8,SetArray)(l,1,viewport); SLOT(l,0xb0,SetArray)(l,1,scissor);
    SLOT(l,0x180,Vertices)(l,0,1,v); SLOT(l,0x190,Targets)(l,1,&rtv,0,0);
    SLOT(l,0x100,SetHeaps)(l,2,pair); SLOT(l,0x120,SetTable)(l,1,sampler); SLOT(l,0x120,SetTable)(l,2,srv);
}
static void footprint(D3D12_TEXTURE_COPY_LOCATION *dst, D3D12_TEXTURE_COPY_LOCATION *src, void *out, void *target, unsigned format) {
    memset(dst,0,sizeof(*dst)); memset(src,0,sizeof(*src));
    dst->pResource=out; dst->Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst->PlacedFootprint.Footprint.Format=(DXGI_FORMAT)format; dst->PlacedFootprint.Footprint.Width=7; dst->PlacedFootprint.Footprint.Height=8; dst->PlacedFootprint.Footprint.Depth=1; dst->PlacedFootprint.Footprint.RowPitch=256;
    src->pResource=target; src->Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
}
static int pixel(volatile unsigned char *p, unsigned offset, const unsigned char *rgba, int bgra) {
    for (unsigned c=0;c<4;c++) if (p[offset+c]!=rgba[bgra && c==0?2:bgra && c==2?0:c]) return 0;
    return 1;
}
unsigned mainCRTStartup(void) {
    if (*relocated!=7) return 99;
    unsigned version=0x00020004; void *dev=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_TEXTURE2D,0,2,2,1,1,DXGI_FORMAT_B8G8R8A8_TYPELESS,{1,0},D3D12_TEXTURE_LAYOUT_UNKNOWN,0};
    for (unsigned i=0;i<2;i++) { void *bad=0; rd.Format=(DXGI_FORMAT)(i?91:90); if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&bad)==S_OK) return 2+i; }
    ID3D12CommandAllocator *allocator=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 4;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 5;
    for (unsigned test=0;test<3;test++) {
        unsigned sf=test==1?28:87, tf=test==0?28:87;
        void *vb=0,*color_vb=0,*out=0,*target=0,*texture=0,*pipeline=0,*heaps[3]={0};
        rd.Dimension=D3D12_RESOURCE_DIMENSION_BUFFER; rd.Width=72; rd.Height=1; rd.Format=DXGI_FORMAT_UNKNOWN; rd.Layout=D3D12_TEXTURE_LAYOUT_ROW_MAJOR; rd.Flags=0;
        if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 6;
        rd.Width=48; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&color_vb)!=S_OK) return 28;
        rd.Width=2048; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&out)!=S_OK) return 7;
        rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=7; rd.Height=8; rd.Format=(DXGI_FORMAT)tf; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
        if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 8;
        rd.Width=2; rd.Height=2; rd.Format=(DXGI_FORMAT)sf; rd.Flags=0;
        if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&texture)!=S_OK) return 9;
        void *v=0,*cv=0,*p=0,*t=0;
        if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(color_vb,0x40,Map)(color_vb,0,0,&cv)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK || SLOT(texture,0x40,Map)(texture,0,0,&t)!=S_OK) return 10;
        const float tri[18]={-1,-1,0,1, -1,-1, 6,-2,0,2, -2,6, -4,12,0,4, 12,-4};
        for (unsigned i=0;i<18;i++) ((float *)v)[i]=tri[i];
        const float color_tri[12]={-1,-1,0,1, 3,-1,0,1, -1,3,0,1};
        for (unsigned i=0;i<12;i++) ((float *)cv)[i]=color_tri[i];
        for (unsigned i=0;i<2048;i++) ((unsigned char *)p)[i]=0x77;
        const unsigned char texels[16]={200,30,70,255, 12,80,190,255, 31,210,90,255, 17,120,240,255};
        for (unsigned y=0;y<2;y++) for (unsigned x=0;x<2;x++) for (unsigned c=0;c<4;c++) ((unsigned char *)t)[y*256+x*4+c]=texels[(y*2+x)*4+(sf==87 && c==0?2:sf==87 && c==2?0:c)];
        for (unsigned i=0;i<3;i++) { HeapDesc hd={{i==0?2:(i==1?0:1),1,i==0?0:1,0},{0,0}}; if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 11; }
        U64 cpu[3],gpu[3]; for (unsigned i=0;i<3;i++) { cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]); }
        D3D12_RENDER_TARGET_VIEW_DESC rtv={0}; rtv.Format=(DXGI_FORMAT)tf; rtv.ViewDimension=D3D12_RTV_DIMENSION_TEXTURE2D;
        D3D12_SHADER_RESOURCE_VIEW_DESC srv={0}; srv.Format=(DXGI_FORMAT)sf; srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D; srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING; srv.Texture2D.MipLevels=1;
        SLOT(dev,0xa0,View)(dev,target,test?&rtv:0,cpu[0]); SLOT(dev,0x90,View)(dev,texture,test?&srv:0,cpu[1]);
        const unsigned point[13]={0,3,3,3,0,1,8,0,0,0,0,0,0}; SLOT(dev,0xb0,Sampler)(dev,point,cpu[2]);
        D3D12_INPUT_ELEMENT_DESC elements[2]={{"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},{"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
        union { U64 align; unsigned char b[224]; } stream={0};
        *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
        *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
        *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
        *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
        *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=tf; *(unsigned *)(stream.b+220)=1;
        StreamDesc sd={sizeof(stream.b),stream.b};
        if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 12;
        union { U64 align; unsigned char b[560]; } color_stream={0};
        unsigned char *cs=color_stream.b; void *color_pipelines[2]={0};
        *(unsigned *)(cs+0)=1; *(const void **)(cs+8)=color_vs; *(U64 *)(cs+16)=sizeof(color_vs);
        *(unsigned *)(cs+24)=2; *(const void **)(cs+32)=color_ps; *(U64 *)(cs+40)=sizeof(color_ps);
        *(unsigned *)(cs+48)=10; *(unsigned *)(cs+52)=3; *(unsigned *)(cs+56)=1; *(unsigned *)(cs+76)=1;
        *(unsigned *)(cs+96)=12; *(void **)(cs+104)=elements; *(unsigned *)(cs+112)=1;
        *(unsigned *)(cs+120)=14; *(unsigned *)(cs+124)=3; *(unsigned *)(cs+128)=15; *(unsigned *)(cs+132)=tf; *(unsigned *)(cs+164)=1;
        *(unsigned *)(cs+168)=8; *(unsigned *)(cs+180)=1;
        *(unsigned *)(cs+188)=5; *(unsigned *)(cs+192)=6; *(unsigned *)(cs+196)=1;
        *(unsigned *)(cs+200)=2; *(unsigned *)(cs+204)=6; *(unsigned *)(cs+208)=1; cs[216]=15;
        *(unsigned *)(cs+504)=11;
        StreamDesc color_sd={sizeof(color_stream.b),color_stream.b};
        if (SLOT(dev,0x2b0,CreateObject)(dev,&color_sd,&IID_ID3D12PipelineState,&color_pipelines[0])!=S_OK) return 23;
        *(unsigned *)(cs+180)=0; cs[216]=1;
        if (SLOT(dev,0x2b0,CreateObject)(dev,&color_sd,&IID_ID3D12PipelineState,&color_pipelines[1])!=S_OK) return 24;
        D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24}; void *pair[2]={heaps[1],heaps[2]};
        /* Each invalid descriptor or incompatible draw is captured by Close. */
        for (unsigned badcase=0;badcase<7;badcase++) {
            ID3D12GraphicsCommandList *bad=0; void *badpipeline=0; void *use=pipeline;
            if (badcase==0) { *(unsigned *)(stream.b+188)=tf==87?28:87; if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&badpipeline)!=S_OK) return 13; use=badpipeline; *(unsigned *)(stream.b+188)=tf; }
            if (badcase==1) { rtv.Format=(DXGI_FORMAT)(tf==87?28:87); SLOT(dev,0xa0,View)(dev,target,&rtv,cpu[0]); }
            if (badcase==2) { srv.Format=(DXGI_FORMAT)(sf==87?28:87); SLOT(dev,0x90,View)(dev,texture,&srv,cpu[1]); }
            if (badcase==3) { srv.Shader4ComponentMapping=0; SLOT(dev,0x90,View)(dev,texture,&srv,cpu[1]); }
            if (badcase==4) { srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2DARRAY; SLOT(dev,0x90,View)(dev,texture,&srv,cpu[1]); }
            if (badcase==5) { rtv.ViewDimension=D3D12_RTV_DIMENSION_TEXTURE2DARRAY; SLOT(dev,0xa0,View)(dev,target,&rtv,cpu[0]); }
            if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,use,&IID_ID3D12GraphicsCommandList,(void **)&bad)!=S_OK) return 14;
            if (badcase==6) { D3D12_TEXTURE_COPY_LOCATION dst,src; footprint(&dst,&src,out,target,tf==87?28:87); SLOT(bad,0x80,Copy)(bad,&dst,0,0,0,&src,0); }
            else { setup(bad,&view,cpu[0],pair,gpu[1],gpu[2]); SLOT(bad,0x60,Draw)(bad,3,1,0,0); }
            if (ID3D12GraphicsCommandList_Close(bad)!=E_NOTIMPL) return 60+badcase;
            SLOT(bad,0x10,Release)(bad); if (badpipeline) SLOT(badpipeline,0x10,Release)(badpipeline);
            rtv.Format=(DXGI_FORMAT)tf; rtv.ViewDimension=D3D12_RTV_DIMENSION_TEXTURE2D;
            srv.Format=(DXGI_FORMAT)sf; srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D; srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING; srv.Texture2D.MostDetailedMip=0; srv.Texture2D.MipLevels=1; srv.Texture2D.PlaneSlice=0; srv.Texture2D.ResourceMinLODClamp=0;
            SLOT(dev,0xa0,View)(dev,target,test?&rtv:0,cpu[0]); SLOT(dev,0x90,View)(dev,texture,test?&srv:0,cpu[1]);
        }
        ID3D12GraphicsCommandList *sample_list=0,*list=0;
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&sample_list)!=S_OK) return 15;
        const float clear[4]={1.0f,0.0f,0.25f,1.0f}; SLOT(sample_list,0x1a0,Clear)(sample_list,cpu[0],clear,0,0);
        setup(sample_list,&view,cpu[0],pair,gpu[1],gpu[2]); SLOT(sample_list,0x60,Draw)(sample_list,3,1,0,0);
        if (ID3D12GraphicsCommandList_Close(sample_list)!=S_OK) return 29;
        /* A separate list avoids carrying texture root tables into the color pipeline. */
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,color_pipelines[0],&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 30;
        float viewport[6]={0,0,8,8,0,1};
        SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0x190,Targets)(list,1,&cpu[0],0,0);
        /* Rectangle clear, then alpha blending and a logical red-only write mask. */
        int rect[4]={1,0,3,8}; SLOT(list,0x1a0,Clear)(list,cpu[0],clear,1,rect);
        const unsigned green[4]={0,0x3f800000,0,0x3f000000};
        for (unsigned i=0;i<4;i++) SLOT(list,0x130,Constant)(list,3,green[i],i);
        D3D12_VERTEX_BUFFER_VIEW color_view={SLOT(color_vb,0x58,Handle)(color_vb),48,16};
        SLOT(list,0x180,Vertices)(list,0,1,&color_view);
        int blend_rect[4]={1,0,2,8}; SLOT(list,0xb0,SetArray)(list,1,blend_rect);
        SLOT(list,0xc8,SetPipeline)(list,color_pipelines[0]); SLOT(list,0x60,Draw)(list,3,1,0,0);
        int mask_rect[4]={2,0,3,8}; SLOT(list,0xb0,SetArray)(list,1,mask_rect);
        SLOT(list,0xc8,SetPipeline)(list,color_pipelines[1]); SLOT(list,0x60,Draw)(list,3,1,0,0);
        Barrier barrier={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&barrier);
        D3D12_TEXTURE_COPY_LOCATION dst,src; footprint(&dst,&src,out,target,tf); SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
        if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 16;
        SLOT(vb,0x10,Release)(vb); SLOT(color_vb,0x10,Release)(color_vb); SLOT(out,0x10,Release)(out); SLOT(target,0x10,Release)(target); SLOT(texture,0x10,Release)(texture); SLOT(pipeline,0x10,Release)(pipeline);
        for (unsigned i=0;i<2;i++) SLOT(color_pipelines[i],0x10,Release)(color_pipelines[i]);
        for (unsigned i=0;i<3;i++) SLOT(heaps[i],0x10,Release)(heaps[i]);
        ID3D12CommandList *sample_lists[]={(ID3D12CommandList *)sample_list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,sample_lists);
        ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
        const unsigned char cleared[4]={255,0,64,255};
        if (!pixel(p,0,cleared,tf==87) || !pixel(p,3*256+4*4,texels,tf==87) || !pixel(p,1*256+5*4,texels+12,tf==87)) return 17+test;
        const unsigned char blended[4]={128,128,32,255}, masked[4]={0,0,64,255};
        for (unsigned c=0;c<4;c++) {
            unsigned k=tf==87 && c==0?2:tf==87 && c==2?0:c;
            int delta=(int)((volatile unsigned char *)p)[4+c]-blended[k];
            if (delta < -2 || delta > 2 || ((volatile unsigned char *)p)[8+c]!=masked[k]) return 25+test;
        }
        for (unsigned y=0;y<8;y++) for (unsigned x=28;x<256;x++) if (((volatile unsigned char *)p)[y*256+x]!=0x77) return 20+test;
        SLOT(sample_list,0x10,Release)(sample_list); SLOT(list,0x10,Release)(list);
    }
    SLOT(queue,0x10,Release)(queue); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev);
    return 42;
}
