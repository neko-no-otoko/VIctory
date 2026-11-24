/* Ordinary Wine PE: sample typed BC textures and round-trip their compressed mips. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <string.h>
#include <stddef.h>
#include "windows_d3d12x_uv_shaders.h"

_Static_assert(offsetof(D3D12_TEXTURE_COPY_LOCATION, SubresourceIndex)==16, "copy location ABI");
_Static_assert(offsetof(D3D12_SHADER_RESOURCE_VIEW_DESC, Texture2D)==16, "SRV union ABI");
__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef HRESULT (__stdcall *CreateResource)(void *, const void *, unsigned, const void *, unsigned, const void *, const IID *, void **);
typedef HRESULT (__stdcall *CreateObject)(void *, const void *, const IID *, void **);
typedef HRESULT (__stdcall *Map)(void *, unsigned, const void *, void **);
typedef void (__stdcall *Footprints)(void *, const D3D12_RESOURCE_DESC *, unsigned, unsigned, U64, D3D12_PLACED_SUBRESOURCE_FOOTPRINT *, unsigned *, U64 *, U64 *);
typedef U64 (__stdcall *Handle)(void *);
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
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),s,(DWORD)strlen(s),&n,0); }

enum { FIRST=1, COUNT, SUBRESOURCE, VIEW_FORMAT, TYPELESS, FORMAT, WIDTH, HEIGHT, PITCH, OFFSET, LAYOUT, MIP_COUNT };
static int rejected(void) { say("D3D12X invalid BC case rejected\r\n"); return 43; }

static int run(unsigned format, int invalid) {
    const int bc3=(format==77 || format==78), srgb=(format==72 || format==78);
    const unsigned dims[4]={8,4,2,1}, offsets[4]={512,1536,2048,2560};
    const unsigned pitches[4]={256,512,256,256}, block_bytes=bc3?16:8;
    const unsigned short endpoints[4]={0xf800,0x07e0,0x001f,0x8410};
    const unsigned char alpha[4]={255,192,64,128};
    const unsigned char expected[4][3]={{255,0,0},{0,255,0},{0,0,255},{132,130,132}};
    unsigned version=0x00020004; void *dev=0,*vb=0,*out=0,*upload=0,*target=0,*texture=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,72,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=4096; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK) return 3;
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&upload)!=S_OK) return 4;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 5;
    rd.MipLevels=4; rd.Format=(DXGI_FORMAT)format; rd.Flags=0;
    if (invalid==TYPELESS) rd.Format=(DXGI_FORMAT)70;
    if (invalid==FORMAT) rd.Format=(DXGI_FORMAT)80;
    if (invalid==WIDTH) rd.Width=7;
    if (invalid==HEIGHT) rd.Height=6;
    if (invalid==LAYOUT) rd.Layout=D3D12_TEXTURE_LAYOUT_ROW_MAJOR;
    if (invalid==MIP_COUNT) rd.MipLevels=5;
    HRESULT hr=make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&texture);
    if (invalid==TYPELESS || invalid==FORMAT || invalid==WIDTH || invalid==HEIGHT || invalid==LAYOUT || invalid==MIP_COUNT)
        return FAILED(hr) && !texture ? rejected() : 93;
    if (hr!=S_OK || !texture) return 6;
    void *v=0,*p=0,*t=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK || SLOT(upload,0x40,Map)(upload,0,0,&t)!=S_OK) return 7;
    const float tri[18]={-1,-1,0,1, -1,-1, 6,-2,0,2, -2,6, -4,12,0,4, 12,-4};
    D3D12_PLACED_SUBRESOURCE_FOOTPRINT layouts[4]; unsigned rows[4]; U64 sizes[4], total;
    SLOT(dev,0x130,Footprints)(dev,&rd,0,4,0,layouts,rows,sizes,&total);
    for (unsigned m=0;m<4;m++) {
        unsigned extent=dims[m]<4?4:dims[m], blocks=extent/4;
        if (layouts[m].Offset!=512*m || layouts[m].Footprint.Format!=(DXGI_FORMAT)format ||
            layouts[m].Footprint.Width!=extent || layouts[m].Footprint.Height!=extent ||
            layouts[m].Footprint.Depth!=1 || layouts[m].Footprint.RowPitch!=256 ||
            rows[m]!=blocks || sizes[m]!=blocks*block_bytes) return 28;
    }
    if (total!=1536+block_bytes) return 29;
    memcpy(v,tri,sizeof(tri)); memset(p,0x77,4096); memset(t,0xa5,4096);
    /* Index zero everywhere: independent RGB565 endpoints and BC3 alpha endpoints.
       Four blocks at mip 0 exercise both compressed block rows and row pitch. */
    for (unsigned m=0;m<4;m++) {
        unsigned blocks=(dims[m]+3)/4;
        for (unsigned y=0;y<blocks;y++) for (unsigned x=0;x<blocks;x++) {
            unsigned char *b=(unsigned char *)t+offsets[m]+y*pitches[m]+x*block_bytes;
            memset(b,0,block_bytes);
            if (bc3) { b[0]=alpha[m]; b[1]=0; b+=8; }
            b[0]=(unsigned char)endpoints[m]; b[1]=(unsigned char)(endpoints[m]>>8);
        }
    }
    void *heaps[6]={0}; U64 cpu[6],gpu[6];
    for (unsigned i=0;i<6;i++) {
        HeapDesc hd={{i==0?2:(i==5?1:0),1,i==0?0:1,0},{0,0}};
        if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 8;
        cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]);
    }
    SLOT(dev,0xa0,View)(dev,target,0,cpu[0]);
    for (unsigned m=0;m<4;m++) {
        D3D12_SHADER_RESOURCE_VIEW_DESC srv={0};
        srv.Format=(DXGI_FORMAT)format; srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D;
        srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
        srv.Texture2D.MostDetailedMip=m; srv.Texture2D.MipLevels=m==3 ? ~0u : 4-m;
        if (!m && invalid==FIRST) srv.Texture2D.MostDetailedMip=4;
        if (!m && invalid==COUNT) { srv.Texture2D.MostDetailedMip=3; srv.Texture2D.MipLevels=2; }
        if (!m && invalid==VIEW_FORMAT) srv.Format=(DXGI_FORMAT)(format==71?72:71);
        SLOT(dev,0x90,View)(dev,texture,&srv,cpu[m+1]);
    }
    const unsigned linear[13]={0x15,3,3,3,0,1,8,0,0,0,0,0,0}; SLOT(dev,0xb0,Sampler)(dev,linear,cpu[5]);
    D3D12_INPUT_ELEMENT_DESC elements[2]={
        {"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},
        {"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}
    };
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    StreamDesc sd={sizeof(stream.b),stream.b};
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 9;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 10;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 11;
    D3D12_TEXTURE_COPY_LOCATION us={0},ud={0};
    us.pResource=upload; us.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    us.PlacedFootprint.Footprint.Format=(DXGI_FORMAT)format; us.PlacedFootprint.Footprint.Depth=1;
    ud.pResource=texture; ud.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
    for (unsigned m=0;m<4;m++) {
        unsigned extent=dims[m]<4?4:dims[m];
        us.PlacedFootprint.Offset=offsets[m]; us.PlacedFootprint.Footprint.Width=extent;
        us.PlacedFootprint.Footprint.Height=extent; us.PlacedFootprint.Footprint.RowPitch=pitches[m]; ud.SubresourceIndex=m;
        if (m==3 && invalid==SUBRESOURCE) ud.SubresourceIndex=4;
        if (m==3 && invalid==PITCH) us.PlacedFootprint.Footprint.RowPitch=128;
        if (m==3 && invalid==OFFSET) us.PlacedFootprint.Offset++;
        SLOT(list,0x80,Copy)(list,&ud,0,0,0,&us,0);
    }
    Barrier b={0,0,texture,{0xffffffffULL | (0x400ULL<<32),0x80}}; SLOT(list,0xd0,BarrierFn)(list,1,&b);
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24}; float viewport[6]={0,0,8,8,0,1};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport);
    SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&cpu[0],0,0);
    for (unsigned m=0;m<4;m++) {
        int scissor[4]={(int)(m*2),0,(int)(m*2+2),8}; void *pair[2]={heaps[m+1],heaps[5]};
        SLOT(list,0xb0,SetArray)(list,1,scissor); SLOT(list,0x100,SetHeaps)(list,2,pair);
        SLOT(list,0x120,SetTable)(list,1,gpu[5]); SLOT(list,0x120,SetTable)(list,2,gpu[m+1]);
        SLOT(list,0x60,Draw)(list,3,1,0,0);
    }
    Barrier texture_read={0,0,texture,{0xffffffffULL | (0x80ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&texture_read);
    D3D12_TEXTURE_COPY_LOCATION mipdst={0},mipsrc={0};
    mipdst.pResource=out; mipdst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    mipdst.PlacedFootprint.Footprint.Format=(DXGI_FORMAT)format; mipdst.PlacedFootprint.Footprint.Depth=1; mipdst.PlacedFootprint.Footprint.RowPitch=256;
    mipsrc.pResource=texture; mipsrc.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
    for (unsigned m=0;m<4;m++) {
        unsigned extent=dims[m]<4?4:dims[m];
        mipdst.PlacedFootprint.Offset=2048+512*m; mipdst.PlacedFootprint.Footprint.Width=extent; mipdst.PlacedFootprint.Footprint.Height=extent;
        mipsrc.SubresourceIndex=m; SLOT(list,0x80,Copy)(list,&mipdst,0,0,0,&mipsrc,0);
    }
    Barrier rb={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&rb);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    hr=ID3D12GraphicsCommandList_Close(list);
    if (invalid) return FAILED(hr) ? rejected() : 94;
    if (hr!=S_OK) return 12;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 14;
    /* Recorded work retains resources and descriptor state after guest releases. */
    SLOT(upload,0x10,Release)(upload); SLOT(vb,0x10,Release)(vb); SLOT(out,0x10,Release)(out);
    SLOT(target,0x10,Release)(target); SLOT(texture,0x10,Release)(texture); SLOT(pipeline,0x10,Release)(pipeline);
    for (unsigned i=0;i<6;i++) SLOT(heaps[i],0x10,Release)(heaps[i]);
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    volatile unsigned char *pixels=p;
    for (unsigned m=0;m<4;m++) for (unsigned c=0;c<4;c++) {
        int want=c==3?(bc3?alpha[m]:255):(m==3 && srgb?(c==1?57:59):expected[m][c]);
        int delta=(int)pixels[3*256+(m*2+1)*4+c]-want;
        if (delta < -3 || delta > 3) return 20+m;
    }
    for (unsigned m=0;m<4;m++) {
        unsigned blocks=(dims[m]+3)/4, row_bytes=blocks*block_bytes;
        for (unsigned y=0;y<blocks;y++) {
            for (unsigned x=0;x<row_bytes;x++)
                if (pixels[2048+512*m+y*256+x]!=((unsigned char *)t)[offsets[m]+y*pitches[m]+x]) return 25;
            for (unsigned x=row_bytes;x<256;x++) if (pixels[2048+512*m+y*256+x]!=0x77) return 26;
        }
        for (unsigned y=blocks;y<2;y++) for (unsigned x=0;x<256;x++)
            if (pixels[2048+512*m+y*256+x]!=0x77) return 27;
    }
    for (unsigned y=0;y<8;y++) for (unsigned x=32;x<256;x++) if (pixels[y*256+x]!=0x77) return 24;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator);
    SLOT(dev,0x10,Release)(dev); return 0;
}

int main(int argc, char **argv) {
    static const char *names[]={"", "--invalid-srv-first", "--invalid-srv-count", "--invalid-subresource", "--invalid-view-format", "--invalid-typeless", "--invalid-format", "--invalid-width", "--invalid-height", "--invalid-pitch", "--invalid-offset", "--invalid-layout", "--invalid-mip-count"};
    int invalid=0;
    if (argc==2) { for (int i=1;i<=MIP_COUNT;i++) if (!strcmp(argv[1],names[i])) invalid=i; if (!invalid) return 90; }
    else if (argc!=1) return 90;
    if (invalid) say("D3D12X invalid BC case armed\r\n");
    const unsigned formats[]={71,72,77,78};
    for (unsigned i=0;i<4;i++) { int result=run(formats[i],invalid); if (result) return result; }
    say("D3D12X BC sampling passed\r\n"); return 42;
}
