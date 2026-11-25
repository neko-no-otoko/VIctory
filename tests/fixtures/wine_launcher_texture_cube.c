/* Real Wine PE: cube views, slice-major mip uploads and readback. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <stdio.h>
#include <string.h>
#include "shaders.h"

__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *,const unsigned *,const IID *,void **);
#define SLOT(o,n,T) ((T)((*(void ***)(o))[(n)/8]))
typedef unsigned long long U64;
typedef HRESULT (__stdcall *CreateResource)(void *,const void *,unsigned,const void *,unsigned,const void *,const IID *,void **);
typedef HRESULT (__stdcall *CreateObject)(void *,const void *,const IID *,void **);
typedef HRESULT (__stdcall *Map)(void *,unsigned,const void *,void **);
typedef void (__stdcall *Footprints)(void *,const D3D12_RESOURCE_DESC *,unsigned,unsigned,U64,D3D12_PLACED_SUBRESOURCE_FOOTPRINT *,unsigned *,U64 *,U64 *);
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
typedef unsigned (__stdcall *Release)(void *);
typedef struct { D3D12_DESCRIPTOR_HEAP_DESC desc; U64 placement[2]; } HeapDesc;
typedef struct { size_t size; const void *stream; } StreamDesc;
typedef struct { unsigned kind,flags; void *resource; U64 data[2]; } Barrier;
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),s,(DWORD)strlen(s),&n,0); }
static void color(unsigned slice,unsigned mip,unsigned x,unsigned y,unsigned char out[4]) {
    out[0]=(unsigned char)(15+slice*25+mip*7+x*3+y); out[1]=(unsigned char)(20+slice*20+mip*9+x+y*5);
    out[2]=(unsigned char)(7+slice*20+x*8+y*2); out[3]=(unsigned char)(180+slice*7+mip*4);
}

int main(int argc,char **argv) {
    const char *which=argc==2?argv[1]:"--valid";
    if (argc>2) return 90;
    int nonsquare=!strcmp(which,"--invalid-nonsquare"), faces=!strcmp(which,"--invalid-faces");
    int cube_array=!strcmp(which,"--invalid-cube-array"), bad_mip=!strcmp(which,"--invalid-mip-range");
    int bad_first_mip=!strcmp(which,"--invalid-first-mip");
    int mismatch_array=!strcmp(which,"--cube-shader-2d-view"), mismatch_2d=!strcmp(which,"--2d-shader-cube-view");
    int invalid=nonsquare||faces||cube_array||bad_mip||bad_first_mip||mismatch_array||mismatch_2d;
    if (!invalid && strcmp(which,"--valid")) return 90;
    if (invalid) say("D3D12X invalid cube case armed\r\n");
    unsigned version=0x00020004; void *dev=0,*vb=0,*out=0,*upload=0,*target=0,*texture=0,*plain=0,*pipeline[6]={0};
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,72,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    rd.Width=16384; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK || make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&upload)!=S_OK) return 3;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 4;
    rd.Width=4; rd.Height=nonsquare?2:4; rd.DepthOrArraySize=faces?5:(cube_array?12:6); rd.MipLevels=2; rd.Flags=0;
    if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&texture)!=S_OK) return 5;
    if (mismatch_array) { rd.DepthOrArraySize=1; if (make(dev,&hp,0,&rd,0x80,0,&IID_ID3D12Resource,&plain)!=S_OK) return 5; }
    void *v=0,*p=0,*t=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK || SLOT(upload,0x40,Map)(upload,0,0,&t)!=S_OK) return 6;
    const float tri[18]={-1,-1,0,1, 1.25f,1.75f, 3,-1,0,1, 1.25f,1.75f, -1,3,0,1, 1.25f,1.75f};
    memcpy(v,tri,sizeof(tri)); memset(p,0x77,16384); memset(t,0xa5,16384);
    for (unsigned s=0;s<6;s++) for (unsigned m=0;m<2;m++) {
        unsigned size=4>>m, offset=(s*2+m)*1024;
        for (unsigned y=0;y<size;y++) for (unsigned x=0;x<size;x++) { unsigned char c[4]; color(s,m,x,y,c); memcpy((char *)t+offset+y*256+x*4,c,4); }
    }
    void *heaps[3]={0}; U64 cpu[3],gpu[3];
    for (unsigned i=0;i<3;i++) {
        HeapDesc hd={{i==0?2:i==1?0:1,i==0?1:i==1?8:2,i==0?0:1,0},{0,0}};
        if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 7;
        cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]);
    }
    unsigned ts=SLOT(dev,0x78,Increment)(dev,0),ss=SLOT(dev,0x78,Increment)(dev,1); if (!ts || !ss) return 8;
    SLOT(dev,0xa0,View)(dev,target,0,cpu[0]);
    for (unsigned i=0;i<2;i++) {
        D3D12_SHADER_RESOURCE_VIEW_DESC srv={0}; srv.Format=DXGI_FORMAT_R8G8B8A8_UNORM; srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
        if (mismatch_array && i==0) { srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D; srv.Texture2D.MipLevels=1; }
        else if (cube_array && i==0) { srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURECUBEARRAY; srv.TextureCubeArray.MipLevels=1; srv.TextureCubeArray.NumCubes=2; }
        else { srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURECUBE; srv.TextureCube.MostDetailedMip=i; srv.TextureCube.MipLevels=1;
            if (i==0 && bad_first_mip) srv.TextureCube.MostDetailedMip=2;
            if (i==0 && bad_mip) { srv.TextureCube.MostDetailedMip=1; srv.TextureCube.MipLevels=2; }
        }
        SLOT(dev,0x90,View)(dev,(mismatch_array && i==0)?plain:texture,&srv,cpu[1]+i*ts);
    }
    const unsigned point[13]={0,3,3,3,0,1,8,0,0,0,0,0,0};
    SLOT(dev,0xb0,Sampler)(dev,point,cpu[2]); SLOT(dev,0xb0,Sampler)(dev,point,cpu[2]+ss);
    D3D12_INPUT_ELEMENT_DESC elements[2]={{"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},{"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}};
    union { U64 align; unsigned char b[224]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2;
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    const void *shaders[6]={ps_cube0,ps_cube1,ps_cube2,ps_cube3,ps_cube4,ps_cube5};
    const U64 shader_sizes[6]={sizeof(ps_cube0),sizeof(ps_cube1),sizeof(ps_cube2),sizeof(ps_cube3),sizeof(ps_cube4),sizeof(ps_cube5)};
    StreamDesc sd={sizeof(stream.b),stream.b};
    for (unsigned i=0;i<6;i++) { *(const void **)(stream.b+32)=mismatch_2d?ps_2d:shaders[i]; *(U64 *)(stream.b+40)=mismatch_2d?sizeof(ps_2d):shader_sizes[i]; if (SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline[i])!=S_OK) return 9; }
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK || ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline[0],&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 10;
    for (unsigned s=0;s<(faces?5:6);s++) for (unsigned m=0;m<2;m++) {
        unsigned sub=s*2+m, size=4>>m;
        D3D12_TEXTURE_COPY_LOCATION src={0},dst={0}; src.pResource=upload; src.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
        src.PlacedFootprint.Offset=sub*1024; src.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; src.PlacedFootprint.Footprint.Width=size; src.PlacedFootprint.Footprint.Height=nonsquare?(2>>m):size; src.PlacedFootprint.Footprint.Depth=1; src.PlacedFootprint.Footprint.RowPitch=256;
        dst.pResource=texture; dst.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; dst.SubresourceIndex=sub;
        SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    }
    Barrier b={0,0,texture,{0xffffffffULL | (0x400ULL<<32),0x80}}; SLOT(list,0xd0,BarrierFn)(list,1,&b);
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),72,24}; float viewport[6]={0,0,8,8,0,1};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport); SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&cpu[0],0,0);
    void *pair[2]={heaps[1],heaps[2]}; SLOT(list,0x100,SetHeaps)(list,2,pair);
    SLOT(list,0x120,SetTable)(list,1,gpu[2]); SLOT(list,0x120,SetTable)(list,3,gpu[2]+ss);
    for (unsigned i=0;i<6;i++) {
        ((void (__stdcall *)(void *,void *))(*(void ***)list)[0xc8/8])(list,pipeline[i]);
        int scissor[4]={(int)i,0,(int)i+1,8}; SLOT(list,0xb0,SetArray)(list,1,scissor);
        SLOT(list,0x120,SetTable)(list,2,gpu[1]); SLOT(list,0x120,SetTable)(list,4,gpu[1]+ts);
        SLOT(list,0x60,Draw)(list,3,1,0,0);
    }
    Barrier read={0,0,texture,{0xffffffffULL | (0x80ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&read);
    for (unsigned sub=0;sub<(faces?10:12);sub++) {
        unsigned size=4>>(sub%2); D3D12_TEXTURE_COPY_LOCATION src={0},dst={0};
        dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT; dst.PlacedFootprint.Offset=2048+sub*1024;
        dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=size; dst.PlacedFootprint.Footprint.Height=nonsquare?(2>>(sub%2)):size; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
        src.pResource=texture; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; src.SubresourceIndex=sub; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    }
    Barrier rb={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&rb);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT; dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256; src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    HRESULT hr=ID3D12GraphicsCommandList_Close(list);
    if (FAILED(hr)) { if (invalid) { say("D3D12X invalid cube case rejected\r\n"); return 43; } return 11; }
    if (((volatile unsigned char *)p)[3*256+4]!=0x77) return 12;
    SLOT(upload,0x10,Release)(upload); SLOT(vb,0x10,Release)(vb); SLOT(out,0x10,Release)(out);
    SLOT(target,0x10,Release)(target); SLOT(texture,0x10,Release)(texture); if (plain) SLOT(plain,0x10,Release)(plain); for (unsigned i=0;i<6;i++) SLOT(pipeline[i],0x10,Release)(pipeline[i]);
    for (unsigned i=0;i<3;i++) SLOT(heaps[i],0x10,Release)(heaps[i]);
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0}; if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 13;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    if (invalid) return 93;
    volatile unsigned char *pixels=p; char msg[160];
    for (unsigned i=0;i<6;i++) {
        unsigned char a[4],c[4]; color(i,0,1,3,a); color(i,1,0,1,c);
        unsigned char want[4]={a[0],a[1],c[0],c[1]};
        for (unsigned ch=0;ch<4;ch++) { unsigned got=pixels[3*256+i*4+ch]; if (got!=want[ch]) { snprintf(msg,sizeof(msg),"cube face %u channel %u: %u expected %u\r\n",i,ch,got,want[ch]); say(msg); return 20; } }
    }
    for (unsigned sub=0;sub<12;sub++) {
        unsigned size=4>>(sub%2);
        for (unsigned y=0;y<size;y++) for (unsigned x=0;x<size;x++) for (unsigned ch=0;ch<4;ch++) {
            unsigned char want[4]; color(sub/2,sub%2,x,y,want); unsigned got=pixels[2048+sub*1024+y*256+x*4+ch];
            if (got!=want[ch]) { snprintf(msg,sizeof(msg),"array subresource %u (%u,%u) channel %u: %u expected %u\r\n",sub,x,y,ch,got,want[ch]); say(msg); return 21; }
        }
        for (unsigned y=0;y<size;y++) if (pixels[2048+sub*1024+y*256+size*4]!=0x77) {
            snprintf(msg,sizeof(msg),"array subresource %u row %u padding overwritten\r\n",sub,y); say(msg); return 22;
        }
    }
    say("D3D12X texture cube passed\r\n"); return 42;
}
