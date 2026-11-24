/* Win64 PE: perspective-correct TEXCOORD0 interpolation and texture sampling. */
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
typedef void (__stdcall *DrawIndexed)(void *, unsigned, unsigned, unsigned, int, unsigned);
typedef void (__stdcall *IndexBuffer)(void *, const D3D12_INDEX_BUFFER_VIEW *);
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
/* Clang can lower structure initialization to memset even without a CRT. */
void *memset(void *dst, int value, size_t len) {
    volatile unsigned char *p=dst;
    for (size_t i=0;i<len;i++) p[i]=(unsigned char)value;
    return dst;
}
void *memcpy(void *dst, const void *src, size_t len) {
    volatile unsigned char *d=dst;
    const volatile unsigned char *s=src;
    for (size_t i=0;i<len;i++) d[i]=s[i];
    return dst;
}
static void setup(ID3D12GraphicsCommandList *list, const D3D12_VERTEX_BUFFER_VIEW *view, U64 rtv, void **pair, U64 srv, U64 sampler) {
    float viewport[6]={0,0,8,8,0,1}; int scissor[4]={0,0,8,8};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0xb0,SetArray)(list,1,scissor);
    SLOT(list,0x180,Vertices)(list,0,1,view); SLOT(list,0x190,Targets)(list,1,&rtv,0,0);
    SLOT(list,0x100,SetHeaps)(list,2,pair); SLOT(list,0x120,SetTable)(list,1,sampler); SLOT(list,0x120,SetTable)(list,2,srv);
}
unsigned mainCRTStartup(void) {
    if (*relocated!=7) return 99;
    unsigned version=0x00020004; void *dev=0,*vb=0,*ib=0,*out=0,*target=0,*texture=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,88,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=8; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&ib)!=S_OK) return 35;
    rd.Width=2048; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&out)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 4;
    rd.Width=2; rd.Height=2; rd.Flags=0;
    if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&texture)!=S_OK) return 5;
    void *v=0,*p=0,*t=0,*indices=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(ib,0x40,Map)(ib,0,0,&indices)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK || SLOT(texture,0x40,Map)(texture,0,0,&t)!=S_OK) return 6;
    /* Independent UV swaps clip x/y; unequal w requires perspective interpolation. */
    const float tri[18]={-1,-1,0,1, -1,-1, 6,-2,0,2, -2,6, -4,12,0,4, 12,-4};
    for (unsigned i=0;i<18;i++) ((float *)((unsigned char *)v+16))[i]=tri[i];
    ((unsigned short *)indices)[0]=2; ((unsigned short *)indices)[1]=0; ((unsigned short *)indices)[2]=1;
    for (unsigned i=0;i<2048;i++) ((unsigned char *)p)[i]=0x77;
    const unsigned char texels[16]={255,0,0,255, 0,255,0,255, 0,0,255,255, 255,255,255,255};
    for (unsigned i=0;i<8;i++) { ((unsigned char *)t)[i]=texels[i]; ((unsigned char *)t)[256+i]=texels[8+i]; }
    void *heaps[3]={0};
    for (unsigned i=0;i<3;i++) { HeapDesc hd={{i==0?2:(i==1?0:1),1,i==0?0:1,0},{0,0}}; if(create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 7; }
    U64 cpu[3],gpu[3]; for(unsigned i=0;i<3;i++) { cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]); }
    SLOT(dev,0xa0,View)(dev,target,0,cpu[0]); SLOT(dev,0x90,View)(dev,texture,0,cpu[1]);
    const unsigned point[13]={0,3,3,3,0,1,8,0,0,0,0,0,0}; SLOT(dev,0xb0,Sampler)(dev,point,cpu[2]);
    D3D12_INPUT_ELEMENT_DESC elements[3]={
        {"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},
        {"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},
        {"COLOR",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}
    };
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    StreamDesc sd={sizeof(stream.b),stream.b};
    if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 8;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 9;
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb)+16,72,24}; void *pair[2]={heaps[1],heaps[2]};
    /* Invalid layout and shader input metadata must poison Close. */
    for (unsigned i=0;i<12;i++) {
        void *bad_pipeline=0; ID3D12GraphicsCommandList *bad=0;
        D3D12_INPUT_ELEMENT_DESC bad_elements[3]={elements[0],elements[1],elements[2]};
        D3D12_VERTEX_BUFFER_VIEW bad_view=view;
        if (i==0) bad_view.StrideInBytes=16;
        if (i==1) bad_elements[1].SemanticName="COLOR";
        if (i==2) bad_elements[1].Format=DXGI_FORMAT_R32G32B32A32_FLOAT;
        if (i==3) bad_elements[1].AlignedByteOffset=12;
        if (i==4) bad_elements[1].InputSlot=1;
        if (i==5) bad_elements[1].InputSlotClass=D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA;
        if (i==10) bad_elements[1].SemanticIndex=1;
        if (i==11) bad_elements[1].InstanceDataStepRate=1;
        *(void **)(stream.b+160)=bad_elements;
        *(unsigned *)(stream.b+168)=i==6?1:(i==7?3:2);
        *(const void **)(stream.b+8)=i==8?vs_missing:(i==9?vs_no_signature_uv:vs);
        *(U64 *)(stream.b+16)=i==8?sizeof(vs_missing):(i==9?sizeof(vs_no_signature_uv):sizeof(vs));
        if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&bad_pipeline)!=S_OK) return 10;
        if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,bad_pipeline,&IID_ID3D12GraphicsCommandList,(void **)&bad)!=S_OK) return 11;
        setup(bad,&bad_view,cpu[0],pair,gpu[1],gpu[2]);
        SLOT(bad,0x60,Draw)(bad,3,1,0,0);
        if (ID3D12GraphicsCommandList_Close(bad)!=E_NOTIMPL) return 50+i;
        SLOT(bad,0x10,Release)(bad); SLOT(bad_pipeline,0x10,Release)(bad_pipeline);
    }
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 12;
    setup(list,&view,cpu[0],pair,gpu[1],gpu[2]);
    int top[4]={0,0,8,2}, bottom[4]={0,2,8,8};
    D3D12_INDEX_BUFFER_VIEW index_view={SLOT(ib,0x58,Handle)(ib),6,DXGI_FORMAT_R16_UINT};
    SLOT(list,0xb0,SetArray)(list,1,top);
    SLOT(list,0x178,IndexBuffer)(list,&index_view);
    SLOT(list,0x68,DrawIndexed)(list,3,1,0,0,0);
    SLOT(list,0xb0,SetArray)(list,1,bottom);
    SLOT(list,0x60,Draw)(list,3,1,0,0);
    Barrier barrier={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&barrier);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 13;
    if (((volatile unsigned char *)p)[3*256+16]!=0x77) return 14;
    SLOT(vb,0x10,Release)(vb); SLOT(ib,0x10,Release)(ib); SLOT(out,0x10,Release)(out); SLOT(target,0x10,Release)(target); SLOT(texture,0x10,Release)(texture); SLOT(pipeline,0x10,Release)(pipeline);
    for (unsigned i=0;i<3;i++) SLOT(heaps[i],0x10,Release)(heaps[i]);
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 15;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    volatile unsigned char *pixels=p;
    /* Swapped UV samples red, blue, green, white; the last sample
       would be green under affine interpolation. */
    const unsigned offsets[4]={3*256+4*4,3*256+7*4,1*256+4*4,1*256+5*4};
    const unsigned char expected[16]={255,0,0,255, 0,0,255,255, 0,255,0,255, 255,255,255,255};
    for (unsigned i=0;i<4;i++) for(unsigned c=0;c<4;c++) if(pixels[offsets[i]+c]!=expected[4*i+c]) return 30+i;
    if (pixels[32]!=0x77) return 34;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev);
    return 42;
}
