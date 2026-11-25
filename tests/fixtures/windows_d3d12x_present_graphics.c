/* TLS-bearing translated graphics and ordered PresentX exercise. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include "windows_d3d12x_ordered_shaders.h"
__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
#ifndef TARGET_FORMAT
#ifdef NATIVE_PRESENT_SRGB
#define TARGET_FORMAT DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
#else
#define TARGET_FORMAT DXGI_FORMAT_R8G8B8A8_UNORM
#endif
#endif
/* TARGET_FORMAT can also be supplied as a numeric DXGI format at build time. */
#define TARGET_BGRA (TARGET_FORMAT == DXGI_FORMAT_B8G8R8A8_UNORM || TARGET_FORMAT == DXGI_FORMAT_B8G8R8A8_UNORM_SRGB)
#define TARGET_SRGB (TARGET_FORMAT == DXGI_FORMAT_R8G8B8A8_UNORM_SRGB || TARGET_FORMAT == DXGI_FORMAT_B8G8R8A8_UNORM_SRGB)
typedef unsigned long long U64;
typedef HRESULT (__stdcall *CreateResource)(void *, const void *, unsigned, const void *, unsigned, const void *, const IID *, void **);
typedef HRESULT (__stdcall *CreateObject)(void *, const void *, const IID *, void **);
typedef HRESULT (__stdcall *Map)(void *, unsigned, const void *, void **);
typedef U64 (__stdcall *Handle)(void *);
typedef void (__stdcall *View)(void *, void *, const void *, U64);
typedef void (__stdcall *Sampler)(void *, const unsigned *, U64);
typedef void (__stdcall *Draw)(void *, unsigned, unsigned, unsigned, unsigned);
typedef void (__stdcall *Constant)(void *, unsigned, unsigned, unsigned);
typedef void (__stdcall *Table)(void *, unsigned, U64);
typedef void (__stdcall *Heaps)(void *, unsigned, void **);
typedef void (__stdcall *Topology)(void *, unsigned);
typedef void (__stdcall *Array)(void *, unsigned, const void *);
typedef void (__stdcall *Vertices)(void *, unsigned, unsigned, const void *);
typedef void (__stdcall *Targets)(void *, unsigned, const U64 *, int, const U64 *);
typedef void (__stdcall *Clear)(void *, U64, const float *, unsigned, const void *);
typedef void (__stdcall *Copy)(void *, const void *, unsigned, unsigned, unsigned, const void *, const void *);
typedef void (__stdcall *BarrierFn)(void *, unsigned, const void *);
typedef unsigned (__stdcall *Release)(void *);
typedef HRESULT (__stdcall *Present)(void *, unsigned, const void *, const void *);
typedef struct { U64 token; unsigned count; void *const *resources; } PresentPlane;
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind, flags; void *resource; U64 data[2]; } Barrier;
#ifndef NATIVE_PRESENT
static void print(const char *s, DWORD n) { DWORD written; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),s,n,&written,0); }
#define SAY(s) print(s,sizeof(s)-1)
static _Thread_local int tls_value=17;
static volatile LONG tls_attach,tls_detach,tls_bad,ready;
static HANDLE gate;
static void NTAPI tls_callback(PVOID image,DWORD reason,PVOID reserved) {
    (void)image; (void)reserved;
    if(reason==DLL_PROCESS_ATTACH || reason==DLL_THREAD_ATTACH) {
        if(tls_value!=17) InterlockedIncrement(&tls_bad);
        if(reason==DLL_THREAD_ATTACH) InterlockedIncrement(&tls_attach);
    } else if(reason==DLL_THREAD_DETACH) InterlockedIncrement(&tls_detach);
}
__attribute__((used,section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK tls_callback_slot=tls_callback;
static DWORD WINAPI worker(void *arg) {
    int value=(int)(INT_PTR)arg;
    if(tls_value!=17) return 1;
    tls_value=value; InterlockedIncrement(&ready);
    if(WaitForSingleObject(gate,5000)!=WAIT_OBJECT_0) return 2;
    return tls_value==value ? 0 : 3;
}
#else
#define SAY(s) ((void)0)
#endif
static void transition(void *list,void *resource,unsigned before,unsigned after) {
    Barrier b={0,0,resource,{0xffffffffULL|((U64)before<<32),after}};
    SLOT(list,0xd0,BarrierFn)(list,1,&b);
}
static void copy_texture(void *list,void *buffer,void *texture,unsigned width,unsigned height,int upload) {
    D3D12_TEXTURE_COPY_LOCATION buf={0},tex={0};
    buf.pResource=buffer; buf.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    buf.PlacedFootprint.Footprint.Format=upload?DXGI_FORMAT_R8G8B8A8_UNORM:TARGET_FORMAT;
    buf.PlacedFootprint.Footprint.Width=width; buf.PlacedFootprint.Footprint.Height=height;
    buf.PlacedFootprint.Footprint.Depth=1; buf.PlacedFootprint.Footprint.RowPitch=256;
    tex.pResource=texture; tex.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
    SLOT(list,0x80,Copy)(list,upload?&tex:&buf,0,0,0,upload?&buf:&tex,0);
}
#ifdef UHD_PRESENT
#ifndef UHD_WIDTH
#define UHD_WIDTH 3840
#endif
#ifndef UHD_HEIGHT
#define UHD_HEIGHT 2160
#endif
static int present_uhd(void) {
    enum { WIDTH=UHD_WIDTH, HEIGHT=UHD_HEIGHT, PITCH=WIDTH*4, SIZE=PITCH*HEIGHT };
    unsigned version=0x00020004; void *dev=0,*upload=0,*texture=0,*mapped=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,SIZE,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    CreateResource make=SLOT(dev,0xd8,CreateResource);
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&upload)!=S_OK) return 2;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=WIDTH; rd.Height=HEIGHT;
    rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN;
    if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&texture)!=S_OK) return 3;
    if (SLOT(upload,0x40,Map)(upload,0,0,&mapped)!=S_OK || !mapped) return 4;
    for (unsigned y=0;y<HEIGHT;y++) for (unsigned x=0;x<WIDTH;x++) {
        unsigned char *dst=(unsigned char *)mapped+(size_t)y*PITCH+x*4;
        unsigned bx=x/64,by=y/64;
        dst[0]=(unsigned char)(bx*17+by*13); dst[1]=(unsigned char)(bx*7+by*29);
        dst[2]=(unsigned char)(bx*31+by*3); dst[3]=(unsigned char)(64+(bx+by)%192);
    }
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 5;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,0,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 6;
    D3D12_TEXTURE_COPY_LOCATION src={0},dst={0};
    src.pResource=upload; src.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    src.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM;
    src.PlacedFootprint.Footprint.Width=WIDTH; src.PlacedFootprint.Footprint.Height=HEIGHT;
    src.PlacedFootprint.Footprint.Depth=1; src.PlacedFootprint.Footprint.RowPitch=PITCH;
    dst.pResource=texture; dst.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
    SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    transition(list,texture,0x400,0x800);
    if (ID3D12GraphicsCommandList_Close(list)!=S_OK) return 7;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 8;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list};
    ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    PresentPlane plane={0x1234,1,&texture};
    if (SLOT(queue,0xf0,Present)(queue,1,&plane,0)!=S_OK) return 9;
    SAY("Wine UHD PresentX passed\r\n");
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list);
    SLOT(allocator,0x10,Release)(allocator); SLOT(texture,0x10,Release)(texture);
    SLOT(upload,0x10,Release)(upload); SLOT(dev,0x10,Release)(dev);
    return 42;
}
#endif

#ifdef NORMAL_WINE_PRESENT
int main(void) {
#else
unsigned mainCRTStartup(void *unused) {
    (void)unused;
#endif
#ifdef UHD_PRESENT
    return present_uhd();
#else
#ifndef NATIVE_PRESENT
    HANDLE threads[2]; DWORD codes[2];
    if(tls_value!=17 || tls_bad) return 20;
    tls_value=99; gate=CreateEventW(NULL,TRUE,FALSE,NULL); if(!gate) return 21;
    threads[0]=CreateThread(NULL,0,worker,(void *)(INT_PTR)111,0,NULL);
    threads[1]=CreateThread(NULL,0,worker,(void *)(INT_PTR)222,0,NULL);
    if(!threads[0] || !threads[1]) return 22;
    ULONGLONG deadline=GetTickCount64()+5000;
    while(InterlockedCompareExchange(&ready,0,0)!=2 && GetTickCount64()<deadline) Sleep(1);
    if(ready!=2 || tls_value!=99 || !SetEvent(gate)) return 23;
    if(WaitForMultipleObjects(2,threads,TRUE,5000)!=WAIT_OBJECT_0) return 24;
    if(!GetExitCodeThread(threads[0],&codes[0]) || !GetExitCodeThread(threads[1],&codes[1])) return 25;
    CloseHandle(threads[0]); CloseHandle(threads[1]); CloseHandle(gate);
    if(codes[0] || codes[1] || tls_attach<2 || tls_detach<2 || tls_bad || tls_value!=99) return 26;
    SAY("TLS compiler state and thread callbacks passed\r\n");
#endif
    unsigned version=0x00020004; void *dev=0,*vb[2]={0},*upload[2]={0},*readback[3]={0},*target[2]={0},*tex[2]={0},*pipeline=0;
    if(D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,48,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    CreateResource make=SLOT(dev,0xd8,CreateResource);
    for(unsigned i=0;i<2;i++) if(make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb[i])!=S_OK) return 2;
    rd.Width=24576;
    for(unsigned i=0;i<2;i++) if(make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&upload[i])!=S_OK) return 3;
    for(unsigned i=0;i<3;i++) if(make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&readback[i])!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=64; rd.Height=64; rd.Format=TARGET_FORMAT; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if(make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target[0])!=S_OK) return 4;
    rd.Height=96;
    if(make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target[1])!=S_OK) return 4;
    rd.Width=2; rd.Height=1; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Flags=0;
#ifdef NATIVE_PRESENT
    for(unsigned i=0;i<2;i++) if(make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&tex[i])!=S_OK) return 5;
#else
    for(unsigned i=0;i<2;i++) if(make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&tex[i])!=S_OK) return 5;
#endif
    const float vertices[2][12]={{-1,-1,0,1,-1.0f/3,-1,0,1,-2.0f/3,1,0,1},{-1.0f/3,-1,0,1,1.0f/3,-1,0,1,0,1,0,1}};
    void *mapped[3];
    if(SLOT(readback[2],0x40,Map)(readback[2],0,0,&mapped[2])!=S_OK) return 6;
    for(unsigned i=0;i<2;i++) {
        void *v=0,*u=0;
        if(SLOT(vb[i],0x40,Map)(vb[i],0,0,&v)!=S_OK || SLOT(upload[i],0x40,Map)(upload[i],0,0,&u)!=S_OK || SLOT(readback[i],0x40,Map)(readback[i],0,0,&mapped[i])!=S_OK) return 6;
        for(unsigned j=0;j<12;j++) ((float *)v)[j]=vertices[i][j];
        for(unsigned j=0;j<24576;j++) { ((unsigned char *)u)[j]=0; ((unsigned char *)mapped[i])[j]=0x77; }
        for(unsigned j=0;j<2;j++) { ((unsigned char *)u)[j*4+i]=255; ((unsigned char *)u)[j*4+3]=255; }
#ifdef NATIVE_PRESENT
        void *sampled=0;
        if(SLOT(tex[i],0x40,Map)(tex[i],0,0,&sampled)!=S_OK) return 6;
        for(unsigned j=0;j<8;j++) ((unsigned char *)sampled)[j]=((unsigned char *)u)[j];
#endif
    }
    void *heap[5]={0}; CreateObject create=SLOT(dev,0x70,CreateObject);
    for(unsigned i=0;i<5;i++) { HeapDesc hd={{(i==0||i==4)?2:(i<3?0:1),1,(i==0||i==4)?0:1,0},{0,0}}; if(create(dev,&hd,&IID_ID3D12DescriptorHeap,&heap[i])!=S_OK) return 7; }
    U64 cpu[5],gpu[5]; for(unsigned i=0;i<5;i++) { cpu[i]=SLOT(heap[i],0x48,Handle)(heap[i]); gpu[i]=SLOT(heap[i],0x50,Handle)(heap[i]); }
    SLOT(dev,0xa0,View)(dev,target[0],0,cpu[0]); SLOT(dev,0xa0,View)(dev,target[1],0,cpu[4]);
    for(unsigned i=0;i<2;i++) SLOT(dev,0x90,View)(dev,tex[i],0,cpu[i+1]);
    const unsigned point[13]={0,3,3,3,0,1,8,0,0,0,0,0,0}; SLOT(dev,0xb0,Sampler)(dev,point,cpu[3]);
    D3D12_INPUT_ELEMENT_DESC element={"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0};
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=&element; *(unsigned *)(stream.b+168)=1;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=TARGET_FORMAT; *(unsigned *)(stream.b+220)=1;
    StreamDesc sd={sizeof(stream.b),stream.b}; if(SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline)!=S_OK) return 8;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list[2]={0}; ID3D12CommandQueue *queue=0;
    if(ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 9;
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if(ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 11;
    Present present=SLOT(queue,0xf0,Present);
    PresentPlane plane={0x1234,1,&target[0]}, invalid=plane;
    if(!FAILED(present(queue,0,&plane,0)) || !FAILED(present(queue,1,0,0)) ||
       !FAILED(present(queue,2,&plane,0)) || !FAILED(present(queue,1,&plane,&qd))) return 27;
    invalid.count=0; if(!FAILED(present(queue,1,&invalid,0))) return 28;
    invalid.count=2; if(!FAILED(present(queue,1,&invalid,0))) return 29;
    invalid.count=1; invalid.resources=0; if(!FAILED(present(queue,1,&invalid,0))) return 30;
    invalid.resources=&vb[0]; if(!FAILED(present(queue,1,&invalid,0))) return 31;
    float viewport[6]={0,0,64,64,0,1},background[4]={0.25f,0.5f,1,0}; int scissor[4]={0,0,64,64};
    for(unsigned i=0;i<2;i++) {
        if(ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list[i])!=S_OK) return 10;
        viewport[3]=(float)(i ? 96 : 64); scissor[3]=i ? 96 : 64;
#ifndef NATIVE_PRESENT
        if(i==0) for(unsigned j=0;j<2;j++) { copy_texture(list[i],upload[j],tex[j],2,1,1); transition(list[i],tex[j],0x400,0x80); }
#endif
        SLOT(list[i],0x1a0,Clear)(list[i],cpu[i?4:0],background,0,0);
#ifdef INVALID_GRAPHICS_TOPOLOGY
        SLOT(list[i],0xa0,Topology)(list[i],2);
#else
        SLOT(list[i],0xa0,Topology)(list[i],4);
#endif
        SLOT(list[i],0xa8,Array)(list[i],1,viewport); SLOT(list[i],0xb0,Array)(list[i],1,scissor);
        D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb[i],0x58,Handle)(vb[i]),48,16};
        SLOT(list[i],0x180,Vertices)(list[i],0,1,&view); SLOT(list[i],0x190,Targets)(list[i],1,&cpu[i?4:0],0,0);
        void *pair[2]={heap[i+1],heap[3]}; SLOT(list[i],0x100,Heaps)(list[i],2,pair);
        SLOT(list[i],0x120,Table)(list[i],1,gpu[3]); SLOT(list[i],0x120,Table)(list[i],2,gpu[i+1]);
        SLOT(list[i],0x130,Constant)(list[i],0,0x3ec00000,0); SLOT(list[i],0x60,Draw)(list[i],3,1,0,0);
        transition(list[i],target[i],4,0x800); copy_texture(list[i],readback[i],target[i],64,i?96:64,0);
        if(ID3D12GraphicsCommandList_Close(list[i])!=S_OK) { SAY("Ordered graphics command list rejected\r\n"); return 12; }
        if(((volatile unsigned char *)mapped[i])[2*256+4]!=0x77) return 13;
        ID3D12CommandList *lists[]={(ID3D12CommandList *)list[i]}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
        plane.resources=&target[i];
        if(present(queue,1,&plane,0)!=S_OK) return 32+i;
        ID3D12GraphicsCommandList *after=0;
        if(ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&after)!=S_OK) return 34;
        copy_texture(after,readback[2],target[i],64,i?96:64,0);
        if(ID3D12GraphicsCommandList_Close(after)!=S_OK) return 35;
        ID3D12CommandList *after_lists[]={(ID3D12CommandList *)after};
        ID3D12CommandQueue_ExecuteCommandLists(queue,1,after_lists);
        for(unsigned j=0;j<256*(i?96:64);j++) if(((volatile unsigned char *)mapped[i])[j]!=((volatile unsigned char *)mapped[2])[j]) return 36;
        SLOT(after,0x10,Release)(after);
    }
    volatile unsigned char *a=mapped[0],*b=mapped[1]; unsigned left=32*256+10*4,mid=48*256+32*4,right=48*256+56*4;
    if(a[left]!=(TARGET_BGRA?0:255) || a[left+1]!=0 || a[left+2]!=(TARGET_BGRA?255:0) || a[left+3]!=255) return 14;
    if(b[mid]!=0 || b[mid+1]!=255 || b[mid+2]!=0 || b[mid+3]!=255) return 17;
    /* RGB is encoded by an sRGB render target; alpha remains linear. */
    if(b[right]!=(TARGET_BGRA?255:(TARGET_SRGB?137:64)) ||
       b[right+1]!=(TARGET_SRGB?188:128) ||
       b[right+2]!=(TARGET_BGRA?(TARGET_SRGB?137:64):255) || b[right+3]!=0) return 18;
    for(unsigned i=0;i<2;i++) { SLOT(list[i],0x10,Release)(list[i]); SLOT(vb[i],0x10,Release)(vb[i]); SLOT(upload[i],0x10,Release)(upload[i]); SLOT(readback[i],0x10,Release)(readback[i]); SLOT(tex[i],0x10,Release)(tex[i]); SLOT(target[i],0x10,Release)(target[i]); }
    SLOT(readback[2],0x10,Release)(readback[2]);
    for(unsigned i=0;i<5;i++) SLOT(heap[i],0x10,Release)(heap[i]);
    SLOT(pipeline,0x10,Release)(pipeline); SLOT(queue,0x10,Release)(queue); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev);
    SAY("PresentX ordered graphics, source preservation and resize passed\r\n"); return 42;
#endif
}
