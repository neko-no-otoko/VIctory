/* Win64 PE: native BC1/BC3 GPU sampling of linear and non-XOR tiled mips. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include "shaders.h"
#include "bc_tiles.h"
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
void *memset(void *dst, int value, size_t len) { volatile unsigned char *p=dst; for (size_t i=0;i<len;i++) p[i]=(unsigned char)value; return dst; }
void *memcpy(void *dst, const void *src, size_t len) { volatile unsigned char *d=dst; const volatile unsigned char *s=src; for (size_t i=0;i<len;i++) d[i]=s[i]; return dst; }
static int marker=7; static int *volatile relocated=&marker;
static void draw(ID3D12GraphicsCommandList *list, const D3D12_VERTEX_BUFFER_VIEW *vb, U64 rtv, void **heaps, U64 srv, U64 sampler) {
    float viewport[6]={0,0,1,1,0,1}; int scissor[4]={0,0,1,1};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
    SLOT(list,0x180,Vertices)(list,0,1,vb); SLOT(list,0x190,Targets)(list,1,&rtv,0,0);
    SLOT(list,0x100,SetHeaps)(list,2,heaps); SLOT(list,0x120,SetTable)(list,1,sampler); SLOT(list,0x120,SetTable)(list,2,srv);
    SLOT(list,0x60,Draw)(list,3,1,0,0);
}
unsigned mainCRTStartup(void) {
    if (*relocated!=7) return 99;
    unsigned version=0x00020004; void *dev=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    ID3D12CommandAllocator *allocator=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 2;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 3;
    for (unsigned fmtcase=0;fmtcase<8;fmtcase++) {
        unsigned format=(fmtcase%2)?77:71; unsigned mode=fmtcase/2;
        D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
        D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,72,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
        void *vb=0,*out=0,*target=0,*texture=0,*pipeline=0,*heaps[3]={0};
        if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 4;
        rd.Width=256; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&out)!=S_OK) return 5;
        rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=1; rd.Height=1; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Layout=D3D12_TEXTURE_LAYOUT_ROW_MAJOR; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
        if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 6;
        rd.Width=32; rd.Height=32; rd.MipLevels=6; rd.Format=(DXGI_FORMAT)format; rd.Flags=0;
        { void *bad=0;
          rd.Layout=(D3D12_TEXTURE_LAYOUT)0x106;
          if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&bad)==S_OK) return 36;
          rd.Layout=(D3D12_TEXTURE_LAYOUT)0x11b;
          if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&bad)==S_OK) return 37;
          rd.Layout=D3D12_TEXTURE_LAYOUT_ROW_MAJOR;
          rd.Format=(DXGI_FORMAT)(format-1); /* typeless */
          if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&bad)==S_OK) return 38;
          rd.Format=(DXGI_FORMAT)(format==71?74:80); /* unsupported BC2/BC4 */
          if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&bad)==S_OK) return 39;
          rd.Format=(DXGI_FORMAT)format;
          rd.MipLevels=7;
          if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&bad)==S_OK) return 40;
          rd.MipLevels=6; }
        rd.Layout=(D3D12_TEXTURE_LAYOUT)(mode==0?1:mode==1?0:mode==2?0x105:0x10a);
        if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&texture)!=S_OK) return 7;
        void *v=0,*p=0;
        if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK) return 8;
        const float tri[18]={-1,-1,0,1, .5f,.5f, 3,-1,0,1, .5f,.5f, -1,3,0,1, .5f,.5f};
        for (unsigned i=0;i<18;i++) ((float *)v)[i]=tri[i];
        /* Encoded endpoints are exact RGB565 red, green, blue, white, red, green. */
        const unsigned char colors[6][4]={{255,0,0,255},{0,255,0,255},{0,0,255,255},{255,255,255,255},{255,0,0,255},{0,255,0,255}};
        if (mode==0) {
            const unsigned short endpoints[6]={0xf800,0x07e0,0x001f,0xffff,0xf800,0x07e0};
            for (unsigned mip=0;mip<6;mip++) {
                void *t=0; unsigned side=32>>mip, blocks=(side+3)/4;
                if (SLOT(texture,0x40,Map)(texture,mip,0,&t)!=S_OK || !t) return 9;
                for (unsigned y=0;y<blocks;y++) for (unsigned x=0;x<blocks;x++) {
                    unsigned char *b=(unsigned char *)t+y*256+x*(format==71?8:16);
                    if (format==77) { b[0]=255; b[1]=0; b[2]=8; for (unsigned j=3;j<8;j++) b[j]=0; b+=8; }
                    unsigned short endpoint=mip==0 && x==4 && y==3 ? 0x001f : endpoints[mip];
                    b[0]=(unsigned char)endpoint; b[1]=(unsigned char)(endpoint>>8);
                    b[2]=b[0]; b[3]=b[1]; for (unsigned j=4;j<8;j++) b[j]=0;
                }
            }
        } else {
            void *t=0;
            if (SLOT(texture,0x40,Map)(texture,0,0,&t)!=S_OK || !t) return 9;
            if (mode<=2 && format==71) memcpy(t,bc_4k_1,sizeof(bc_4k_1));
            if (mode<=2 && format==77) memcpy(t,bc_4k_3,sizeof(bc_4k_3));
            if (mode==3 && format==71) memcpy(t,bc_64k_1,sizeof(bc_64k_1));
            if (mode==3 && format==77) memcpy(t,bc_64k_3,sizeof(bc_64k_3));
            void *badmap=0; if (SLOT(texture,0x40,Map)(texture,1,0,&badmap)==S_OK) return 10;
        }
        { void *badmap=0; if (SLOT(texture,0x40,Map)(texture,6,0,&badmap)==S_OK) return 10; }
        for (unsigned i=0;i<3;i++) { HeapDesc hd={{i==0?2:(i==1?0:1),1,i==0?0:1,0},{0,0}}; if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 11; }
        U64 cpu[3],gpu[3]; for (unsigned i=0;i<3;i++) { cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]); }
        SLOT(dev,0xa0,View)(dev,target,0,cpu[0]);
        D3D12_SHADER_RESOURCE_VIEW_DESC srv={0}; srv.Format=(DXGI_FORMAT)format; srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D; srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING; srv.Texture2D.MipLevels=6;
        D3D12_SAMPLER_DESC sampler={0}; sampler.Filter=D3D12_FILTER_MIN_MAG_MIP_POINT; sampler.AddressU=sampler.AddressV=sampler.AddressW=D3D12_TEXTURE_ADDRESS_MODE_CLAMP; sampler.MaxAnisotropy=1; sampler.ComparisonFunc=D3D12_COMPARISON_FUNC_ALWAYS;
        D3D12_INPUT_ELEMENT_DESC elements[2]={{"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},{"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
        union { U64 align; unsigned char b[224]; } stream={0};
        *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
        *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
        *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
        *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
        *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
        StreamDesc sd={sizeof(stream.b),stream.b};
        if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 12;
        D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24}; void *pair[2]={heaps[1],heaps[2]};
        /* Descriptor errors are recorded on the drawing list and reported by Close. */
        for (unsigned bad=0;bad<10;bad++) {
            srv.Texture2D.MostDetailedMip=0; srv.Texture2D.MipLevels=6;
            sampler.MinLOD=0; sampler.MaxLOD=0;
            if (bad==0) srv.Texture2D.MostDetailedMip=6;
            if (bad==1) { srv.Texture2D.MostDetailedMip=5; srv.Texture2D.MipLevels=2; }
            if (bad==2) srv.Texture2D.MipLevels=0;
            if (bad==3) sampler.MinLOD=-1;
            if (bad==4) { sampler.MinLOD=2; sampler.MaxLOD=1; }
            if (bad==5) { union { unsigned u; float f; } x={0x7f800000}; sampler.MaxLOD=x.f; }
            if (bad==6) { union { unsigned u; float f; } x={0x7fc00000}; sampler.MinLOD=x.f; }
            if (bad==7) sampler.MipLODBias=1;
            if (bad==8) srv.Format=(DXGI_FORMAT)(format-1);
            if (bad==9) srv.Format=(DXGI_FORMAT)(format+1);
            SLOT(dev,0x90,View)(dev,texture,&srv,cpu[1]); SLOT(dev,0xb0,Sampler)(dev,&sampler,cpu[2]);
            ID3D12GraphicsCommandList *list=0;
            if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 13;
            draw(list,&view,cpu[0],pair,gpu[1],gpu[2]);
            if (ID3D12GraphicsCommandList_Close(list)!=E_NOTIMPL) return 14+bad;
            SLOT(list,0x10,Release)(list); sampler.MipLODBias=0; srv.Format=(DXGI_FORMAT)format;
        }
        /* Each mip is selected through a view-relative LOD, including 2x2 and 1x1. */
        const unsigned bases[6]={0,1,1,2,1,0};
        const unsigned lods[6] ={0,0,1,1,3,5};
        const unsigned xs[6]   ={17,1,4,2,1,0};
        const unsigned ys[6]   ={12,1,4,2,0,0};
        for (unsigned test=0;test<6;test++) {
            unsigned base=bases[test], lod=lods[test], mip=base+lod;
            unsigned side=32>>mip;
            for (unsigned vertex=0;vertex<3;vertex++) {
                ((float *)v)[vertex*6+4]=(xs[test]+.5f)/side;
                ((float *)v)[vertex*6+5]=(ys[test]+.5f)/side;
            }
            srv.Texture2D.MostDetailedMip=base; srv.Texture2D.MipLevels=6-base;
            sampler.Filter=D3D12_FILTER_MIN_MAG_MIP_POINT;
            sampler.MinLOD=(float)lod; sampler.MaxLOD=sampler.MinLOD;
            SLOT(dev,0x90,View)(dev,texture,(test==0 || test==5)?0:&srv,cpu[1]); SLOT(dev,0xb0,Sampler)(dev,&sampler,cpu[2]);
            for (unsigned i=0;i<256;i++) ((unsigned char *)p)[i]=0x77;
            ID3D12GraphicsCommandList *list=0;
            if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 23;
            if (test) { Barrier back={0,0,target,{0xffffffffULL | (0x800ULL<<32),4}}; SLOT(list,0xd0,BarrierFn)(list,1,&back); }
            draw(list,&view,cpu[0],pair,gpu[1],gpu[2]);
            Barrier barrier={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&barrier);
            D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT; dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=1; dst.PlacedFootprint.Footprint.Height=1; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256; src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
            SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
            if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 24;
            /* The final list owns every resource after the caller releases its references. */
            if (test==5) { SLOT(vb,0x10,Release)(vb); SLOT(out,0x10,Release)(out); SLOT(target,0x10,Release)(target); SLOT(texture,0x10,Release)(texture); SLOT(pipeline,0x10,Release)(pipeline); for (unsigned i=0;i<3;i++) SLOT(heaps[i],0x10,Release)(heaps[i]); }
            ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
            for (unsigned c=0;c<4;c++) {
                int expected=c==3 && format==77 && (xs[test]%4)==1 && (ys[test]%4)==0 ? 0 : colors[test==0?2:mip][c];
                int delta=(int)((volatile unsigned char *)p)[c]-expected;
                if (delta != 0) return 25+test;
            }
            for (unsigned i=4;i<256;i++) if (((volatile unsigned char *)p)[i]!=0x77) return 30+test;
            SLOT(list,0x10,Release)(list);
        }
    }
    SLOT(queue,0x10,Release)(queue); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev);
    return 42;
}
