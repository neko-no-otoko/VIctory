/* Real Wine/Metal streaming: 32 MiB texture, 64 MiB upload and 32 MiB readback. */
/* Each batch samples, replaces, and samples the same texture again. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <string.h>
#include "windows_d3d12x_uv_shaders.h"

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
typedef void (__stdcall *SetTable)(void *, unsigned, U64);
typedef void (__stdcall *SetHeaps)(void *, unsigned, void **);
typedef void (__stdcall *Topology)(void *, unsigned);
typedef void (__stdcall *SetArray)(void *, unsigned, const void *);
typedef void (__stdcall *Vertices)(void *, unsigned, unsigned, const void *);
typedef void (__stdcall *Targets)(void *, unsigned, const U64 *, int, const U64 *);
typedef void (__stdcall *Copy)(void *, const void *, unsigned, unsigned, unsigned, const void *, const void *);
typedef void (__stdcall *BarrierFn)(void *, unsigned, const void *);
typedef unsigned (__stdcall *Release)(void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind, flags; void *resource; U64 data[2]; } Barrier;
#define WIDTH 4096u
#define HEIGHT 2048u
#define PITCH (WIDTH*4u)
#define IMAGE_SIZE (PITCH*HEIGHT)
#define OUTPUT_SIZE (IMAGE_SIZE+2048u)
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),s,(DWORD)strlen(s),&n,0); }
static void transition(void *list, void *resource, unsigned before, unsigned after) {
    Barrier b={0,0,resource,{0xffffffffULL|((U64)before<<32),after}};
    SLOT(list,0xd0,BarrierFn)(list,1,&b);
}
static void copy(void *list, void *buffer, void *texture, unsigned width, unsigned height, U64 offset, int upload, int invalid) {
    D3D12_TEXTURE_COPY_LOCATION buf={0},tex={0};
    buf.pResource=buffer; buf.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    buf.PlacedFootprint.Offset=offset;
    buf.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM;
    buf.PlacedFootprint.Footprint.Width=width; buf.PlacedFootprint.Footprint.Height=height;
    buf.PlacedFootprint.Footprint.Depth=1;
    buf.PlacedFootprint.Footprint.RowPitch=width==8?256:invalid?PITCH-256:PITCH;
    tex.pResource=texture; tex.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
    SLOT(list,0x80,Copy)(list,upload?&tex:&buf,0,0,0,upload?&buf:&tex,0);
}
int main(int argc, char **argv) {
    int invalid=0;
    if(argc==2 && !strcmp(argv[1],"--invalid-pitch")) invalid=1;
    else if(argc==2 && !strcmp(argv[1],"--invalid-range")) invalid=2;
    else if(argc==2 && !strcmp(argv[1],"--resource-limit")) invalid=3;
    else if(argc==2 && !strcmp(argv[1],"--batch-limit")) invalid=4;
    else if(argc!=1) return 90;
    unsigned version=0x00020004; void *dev=0,*vb=0,*upload=0,*out=0,*texture=0,*target=0,*pipeline=0;
    if(D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,72,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if(invalid==4) {
        /* Three individually legal 96 MiB buffers exceed the 256 MiB batch cap. */
        void *buffers[3]={0};
        rd.Width=96ULL*1024*1024;
        for(unsigned i=0;i<3;i++)
            if(make(dev,&hp,0,&rd,i==2?0x400:0x800,0,&IID_ID3D12Resource,&buffers[i])!=S_OK) return 94;
        ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
        if(ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 95;
        if(ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,0,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 96;
        ID3D12GraphicsCommandList_CopyBufferRegion(list,buffers[2],0,buffers[0],0,1);
        ID3D12GraphicsCommandList_CopyBufferRegion(list,buffers[2],1,buffers[1],0,1);
        say("large invalid case armed\r\n");
        if(FAILED(ID3D12GraphicsCommandList_Close(list))) { say("large invalid case rejected\r\n"); return 43; }
        D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};
        if(ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 97;
        ID3D12CommandList *submit[]={(ID3D12CommandList *)list};
        ID3D12CommandQueue_ExecuteCommandLists(queue,1,submit);
        return 92;
    }
    if(invalid==3) {
        say("large invalid case armed\r\n"); rd.Width=128ULL*1024*1024+1;
        HRESULT hr=make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&upload);
        if(FAILED(hr) && !upload) { say("large invalid case rejected\r\n"); return 43; }
        if(hr!=S_OK || !upload) return 91;
        /* Force the oversized resource into a submission if creation is lazy. */
    }
    rd.Width=72; if(make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=2ULL*IMAGE_SIZE; if(!upload && make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&upload)!=S_OK) return 3;
    rd.Width=OUTPUT_SIZE; if(make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK) return 4;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if(make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 5;
    rd.Width=WIDTH; rd.Height=HEIGHT; rd.Flags=0;
    if(make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&texture)!=S_OK) return 6;
    void *v=0,*u=0,*p=0;
    if(SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(upload,0x40,Map)(upload,0,0,&u)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK) return 7;
    const float tri[18]={-1,-1,0,1,-1,-1, 6,-2,0,2,-2,6, -4,12,0,4,12,-4};
    memcpy(v,tri,sizeof(tri)); memset(p,0x77,OUTPUT_SIZE);
    void *heap[3]={0}; U64 cpu[3],gpu[3];
    for(unsigned i=0;i<3;i++) {
        HeapDesc hd={{i==0?2:(i==2?1:0),1,i==0?0:1,0},{0,0}};
        if(create(dev,&hd,&IID_ID3D12DescriptorHeap,&heap[i])!=S_OK) return 8;
        cpu[i]=SLOT(heap[i],0x48,Handle)(heap[i]); gpu[i]=SLOT(heap[i],0x50,Handle)(heap[i]);
    }
    SLOT(dev,0xa0,View)(dev,target,0,cpu[0]); SLOT(dev,0x90,View)(dev,texture,0,cpu[1]);
    const unsigned linear[13]={0x15,3,3,3,0,1,8,0,0,0,0,0,0}; SLOT(dev,0xb0,Sampler)(dev,linear,cpu[2]);
    D3D12_INPUT_ELEMENT_DESC elements[2]={
        {"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},
        {"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    StreamDesc sd={sizeof(stream.b),stream.b}; if(SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 9;
    ID3D12CommandAllocator *allocator=0; ID3D12CommandQueue *queue=0; ID3D12GraphicsCommandList *lists[2]={0};
    if(ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 10;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if(ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 11;
    const unsigned char colors[2][2][4]={{{231,37,81,255},{47,89,219,255}},{{19,197,63,255},{211,173,23,255}}};
    for(unsigned pass=0;pass<2;pass++) {
        for(unsigned i=0;i<IMAGE_SIZE;i+=4) {
            memcpy((unsigned char *)u+i,colors[pass][0],4);
            memcpy((unsigned char *)u+IMAGE_SIZE+i,colors[pass][1],4);
        }
        if(ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&lists[pass])!=S_OK) return 12;
        void *list=lists[pass];
        if(pass) { transition(list,texture,0x800,0x400); transition(list,target,0x800,4); }
        if(invalid && pass==0 && invalid!=3) say("large invalid case armed\r\n");
        copy(list,upload,texture,WIDTH,HEIGHT,invalid==2?IMAGE_SIZE+512:0,1,invalid==1);
        transition(list,texture,0x400,0x80);
        D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24};
        float viewport[6]={0,0,8,8,0,1}; int scissor[4]={0,0,4,8};
        SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
        SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&cpu[0],0,0);
        void *pair[2]={heap[1],heap[2]}; SLOT(list,0x100,SetHeaps)(list,2,pair);
        SLOT(list,0x120,SetTable)(list,1,gpu[2]); SLOT(list,0x120,SetTable)(list,2,gpu[1]); SLOT(list,0x60,Draw)(list,3,1,0,0);
        /* The second draw reuses the unchanged large sampled view. */
        SLOT(list,0x60,Draw)(list,3,1,0,0);
        transition(list,texture,0x80,0x400);
        copy(list,upload,texture,WIDTH,HEIGHT,IMAGE_SIZE,1,0);
        transition(list,texture,0x400,0x80);
        scissor[0]=4; scissor[2]=8; SLOT(list,0xb0,SetArray)(list,1,scissor);
        SLOT(list,0x60,Draw)(list,3,1,0,0);
        transition(list,texture,0x80,0x800); copy(list,out,texture,WIDTH,HEIGHT,0,0,0);
        transition(list,target,4,0x800); copy(list,out,target,8,8,IMAGE_SIZE,0,0);
        HRESULT hr=ID3D12GraphicsCommandList_Close(lists[pass]);
        if(invalid && FAILED(hr)) { say("large invalid case rejected\r\n"); return 43; }
        if(hr!=S_OK) return 13;
        if(!pass && ((volatile unsigned char *)p)[IMAGE_SIZE+3*256+4]!=0x77) return 14;
        /* The second list retains all of its dependencies through submission. */
        if(pass) { SLOT(upload,0x10,Release)(upload); SLOT(vb,0x10,Release)(vb); SLOT(texture,0x10,Release)(texture); SLOT(target,0x10,Release)(target); for(unsigned i=0;i<3;i++) SLOT(heap[i],0x10,Release)(heap[i]); SLOT(pipeline,0x10,Release)(pipeline); }
        ID3D12CommandList *submit[]={(ID3D12CommandList *)lists[pass]}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,submit);
        if(invalid) return 92;
        volatile unsigned char *pixels=p;
        for(unsigned c=0;c<4;c++) {
            int left=(int)pixels[IMAGE_SIZE+3*256+4*2+c]-colors[pass][0][c];
            int right=(int)pixels[IMAGE_SIZE+3*256+4*6+c]-colors[pass][1][c];
            if(left < -2 || left > 2 || right < -2 || right > 2) return 20+pass;
            if(pixels[c]!=colors[pass][1][c] || pixels[IMAGE_SIZE-4+c]!=colors[pass][1][c]) return 22+pass;
        }
        for(unsigned y=0;y<8;y++) for(unsigned x=32;x<256;x++) if(pixels[IMAGE_SIZE+y*256+x]!=0x77) return 24;
    }
    for(unsigned i=0;i<2;i++) SLOT(lists[i],0x10,Release)(lists[i]);
    SLOT(out,0x10,Release)(out); SLOT(queue,0x10,Release)(queue); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev);
    say("D3D12X large streamed sampling passed\r\n"); return 42;
}
