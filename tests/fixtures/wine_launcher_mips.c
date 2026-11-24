/* Ordinary Wine PE: upload and sample independently selected RGBA8 mips. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
#include <string.h>
#include <stddef.h>
_Static_assert(offsetof(D3D12_TEXTURE_COPY_LOCATION, SubresourceIndex)==16, "copy location ABI");
_Static_assert(offsetof(D3D12_SHADER_RESOURCE_VIEW_DESC, Texture2D)==16, "SRV union ABI");
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
typedef void (__stdcall *DrawIndexed)(void *, unsigned, unsigned, unsigned, int, unsigned);
typedef void (__stdcall *IndexBuffer)(void *, const void *);
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
static _Thread_local int tls_value = 17;
static void say(const char *s) { DWORD n; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),s,(DWORD)strlen(s),&n,0); }
static DWORD WINAPI tls_thread(void *arg) {
    (void)arg;
    if (tls_value != 17) return 1;
    tls_value=55;
    return tls_value != 55;
}

int main(int argc, char **argv) {
    int invalid=0, additive=0, blend_case=0, vertex_case=0;
    if (argc==2) {
        if (!strcmp(argv[1],"--invalid-srv-first")) invalid=1;
        else if (!strcmp(argv[1],"--invalid-srv-count")) invalid=2;
        else if (!strcmp(argv[1],"--invalid-subresource")) invalid=3;
        else if (!strcmp(argv[1],"--invalid-format")) invalid=4;
        else if (!strcmp(argv[1],"--invalid-pitch")) invalid=5;
        else if (!strcmp(argv[1],"--invalid-offset")) invalid=6;
        else if (!strcmp(argv[1],"--invalid-layout")) invalid=7;
        else if (!strcmp(argv[1],"--invalid-mip-count")) invalid=8;
        else if (!strcmp(argv[1],"--additive")) additive=1;
        else if (!strcmp(argv[1],"--invalid-blend")) { additive=1; invalid=9; }
        else if (!strcmp(argv[1],"--blend-factors")) blend_case=1;
        else if (!strcmp(argv[1],"--blend-subtract")) blend_case=2;
        else if (!strcmp(argv[1],"--blend-minmax")) blend_case=3;
        else if (!strcmp(argv[1],"--blend-disabled-mask")) blend_case=4;
        else if (!strcmp(argv[1],"--blend-maxmin")) blend_case=5;
        else if (!strcmp(argv[1],"--blend-reverse")) blend_case=6;
        else if (!strcmp(argv[1],"--blend-enabled-mask")) blend_case=7;
        else if (!strcmp(argv[1],"--blend-saturate")) blend_case=8;
        else if (!strcmp(argv[1],"--invalid-blend-alpha")) { additive=1; invalid=10; }
        else if (!strcmp(argv[1],"--invalid-blend-constant")) { additive=1; invalid=11; }
        else if (!strcmp(argv[1],"--invalid-blend-logic")) { additive=1; invalid=12; }
        else if (!strcmp(argv[1],"--invalid-blend-operation")) { additive=1; invalid=13; }
        else if (!strcmp(argv[1],"--vertex-interleaved")) vertex_case=1;
        else if (!strcmp(argv[1],"--vertex-instance")) vertex_case=2;
        else if (!strcmp(argv[1],"--invalid-vertex-range")) vertex_case=3;
        else if (!strcmp(argv[1],"--invalid-vertex-format")) vertex_case=4;
        else if (!strcmp(argv[1],"--invalid-vertex-step")) vertex_case=5;
        else if (!strcmp(argv[1],"--vertex-semantic")) vertex_case=6;
        else if (!strcmp(argv[1],"--vertex-unorm")) vertex_case=7;
        else if (!strcmp(argv[1],"--vertex-indexed")) vertex_case=8;
        else return 90;
    } else if (argc!=1) return 90;
    if (tls_value!=17) return 91;
    tls_value=23;
    HANDLE thread=CreateThread(0,0,tls_thread,0,0,0); DWORD code=1;
    if (!thread || WaitForSingleObject(thread,5000)!=WAIT_OBJECT_0 ||
        !GetExitCodeThread(thread,&code) || code || tls_value!=23) return 92;
    CloseHandle(thread);
    unsigned version=0x00020004; void *dev=0,*vb=0,*uvb=0,*out=0,*upload=0,*target=0,*texture=0,*pipeline=0;
    if (D3D12XboxCreateDevice(0,&version,&IID_ID3D12Device,&dev)!=S_OK || !dev) return 1;
    CreateResource make=SLOT(dev,0xd8,CreateResource); CreateObject create=SLOT(dev,0x70,CreateObject);
    D3D12_HEAP_PROPERTIES hp={D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd={D3D12_RESOURCE_DIMENSION_BUFFER,0,vertex_case?112:72,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,0};
    if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&vb)!=S_OK) return 2;
    if (vertex_case==2 || vertex_case==7) {rd.Width=16;if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&uvb)!=S_OK) return 28;}
    if (vertex_case==8) {rd.Width=8;if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&uvb)!=S_OK) return 28;}
    rd.Width=4096; if (make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&out)!=S_OK) return 3;
    rd.Width=4096; if (make(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&upload)!=S_OK) return 4;
    rd.Dimension=D3D12_RESOURCE_DIMENSION_TEXTURE2D; rd.Width=8; rd.Height=8; rd.Format=DXGI_FORMAT_R8G8B8A8_UNORM; rd.Layout=D3D12_TEXTURE_LAYOUT_UNKNOWN; rd.Flags=D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
    if (make(dev,&hp,0,&rd,4,0,&IID_ID3D12Resource,&target)!=S_OK) return 5;
    rd.Width=7; rd.Height=5; rd.MipLevels=3; rd.Flags=0;
    if (invalid==4 || invalid==7 || invalid==8) {
        say("D3D12X invalid mip case armed\r\n");
        if (invalid==4) rd.Format=DXGI_FORMAT_R8G8B8A8_TYPELESS;
        if (invalid==7) rd.Layout=D3D12_TEXTURE_LAYOUT_ROW_MAJOR;
        if (invalid==8) rd.MipLevels=4;
    }
    HRESULT hr=make(dev,&hp,0,&rd,0x400,0,&IID_ID3D12Resource,&texture);
    if (invalid==4 || invalid==7 || invalid==8) {
        if (FAILED(hr) && !texture) { say("D3D12X invalid mip case rejected\r\n"); return 43; }
        return 93;
    }
    if (hr!=S_OK || !texture) return 6;
    void *v=0,*p=0,*t=0;
    if (SLOT(vb,0x40,Map)(vb,0,0,&v)!=S_OK || SLOT(out,0x40,Map)(out,0,0,&p)!=S_OK || SLOT(upload,0x40,Map)(upload,0,0,&t)!=S_OK) return 7;
    const float tri[18]={-1,-1,0,1, -1,-1, 6,-2,0,2, -2,6, -4,12,0,4, 12,-4};
    if (vertex_case) {
        memset(v,0xcc,112);
        for (unsigned i=0;i<3;i++) memcpy((unsigned char *)v+i*32+4,tri+i*6,16);
        if (vertex_case!=2 && vertex_case!=7) for (unsigned i=0;i<3;i++) memcpy((unsigned char *)v+i*32+20,tri+i*6+4,8);
        if (vertex_case==2 || vertex_case==7) {
            void *u=0;if (SLOT(uvb,0x40,Map)(uvb,0,0,&u)!=S_OK) return 29;
            memset(u,0,16); ((unsigned short *)u)[6]=vertex_case==7?0xffff:0x3c00; /* instance record 3: UV=(1,0) */
        }
        if (vertex_case==8) {
            void *u=0;if (SLOT(uvb,0x40,Map)(uvb,0,0,&u)!=S_OK) return 29;
            const unsigned short indices[4]={99,1,2,3};memcpy(u,indices,sizeof(indices));
        }
    } else memcpy(v,tri,sizeof(tri));
    memset(p,0x77,4096); memset(t,0xa5,4096);
    const unsigned widths[3]={7,3,1}, heights[3]={5,2,1};
    const unsigned offsets[3]={512,2048,3072}, pitches[3]={256,512,256};
    const unsigned char colors[3][4]={{231,37,81,100},{19,197,63,150},{47,89,219,200}};
    for (unsigned m=0;m<3;m++) for (unsigned y=0;y<heights[m];y++) for (unsigned x=0;x<widths[m];x++)
        memcpy((unsigned char *)t+offsets[m]+y*pitches[m]+4*x,colors[m],4);
    if (vertex_case==2 || vertex_case==7) for (unsigned y=0;y<heights[0];y++)
        memset((unsigned char *)t+offsets[0]+y*pitches[0],0,4); /* UV=(0,0) differs */
    void *heaps[5]={0}; U64 cpu[5],gpu[5];
    for (unsigned i=0;i<5;i++) {
        HeapDesc hd={{i==0?2:(i==4?1:0),1,i==0?0:1,0},{0,0}};
        if (create(dev,&hd,&IID_ID3D12DescriptorHeap,&heaps[i])!=S_OK) return 8;
        cpu[i]=SLOT(heaps[i],0x48,Handle)(heaps[i]); gpu[i]=SLOT(heaps[i],0x50,Handle)(heaps[i]);
    }
    SLOT(dev,0xa0,View)(dev,target,0,cpu[0]);
    for (unsigned m=0;m<3;m++) {
        D3D12_SHADER_RESOURCE_VIEW_DESC srv={0};
        srv.Format=DXGI_FORMAT_R8G8B8A8_UNORM; srv.ViewDimension=D3D12_SRV_DIMENSION_TEXTURE2D;
        srv.Shader4ComponentMapping=D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING;
        srv.Texture2D.MostDetailedMip=m; srv.Texture2D.MipLevels=m==2?~0u:3-m;
        if (m==0 && (invalid==1 || invalid==2)) {
            say("D3D12X invalid mip case armed\r\n");
            if (invalid==1) srv.Texture2D.MostDetailedMip=3;
            else { srv.Texture2D.MostDetailedMip=2; srv.Texture2D.MipLevels=2; }
        }
        SLOT(dev,0x90,View)(dev,texture,&srv,cpu[m+1]);
    }
    const unsigned linear[13]={0x15,3,3,3,0,1,8,0,0,0,0,0,0}; SLOT(dev,0xb0,Sampler)(dev,linear,cpu[4]);
    D3D12_INPUT_ELEMENT_DESC elements[2]={
        {"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,0,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0},
        {"TEXCOORD",0,DXGI_FORMAT_R32G32_FLOAT,0,16,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0}
    };
    if (vertex_case==2 || vertex_case==7) {
        elements[0]=(D3D12_INPUT_ELEMENT_DESC){"TEXCOORD",0,vertex_case==7?DXGI_FORMAT_R16G16_UNORM:DXGI_FORMAT_R16G16_FLOAT,1,0,D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA,2};
        elements[1]=(D3D12_INPUT_ELEMENT_DESC){"POSITION",0,DXGI_FORMAT_R32G32B32A32_FLOAT,0,4,D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,0};
    } else if (vertex_case) {
        elements[0].AlignedByteOffset=4; elements[1].AlignedByteOffset=D3D12_APPEND_ALIGNED_ELEMENT;
        if (vertex_case==4) elements[1].Format=DXGI_FORMAT_R32_UINT;
        if (vertex_case==5) {elements[1].InputSlotClass=D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA;elements[1].InstanceDataStepRate=0;}
    }
    unsigned char custom_vs[sizeof(vs)];
    if (vertex_case==6) {
        memcpy(custom_vs,vs,sizeof(vs));
        int found=0;
        for (size_t i=0;i+8<sizeof(vs);i++) if (!memcmp(custom_vs+i,"ISG1",4)) {
            unsigned size;memcpy(&size,custom_vs+i+4,4);
            if (size>sizeof(vs)-i-8) return 98;
            for (size_t j=i+8;j+9<=i+8+size;j++) if (!memcmp(custom_vs+j,"TEXCOORD",9)) {
                memcpy(custom_vs+j,"COLOR___",8);found=1;break;
            }
            break;
        }
        if (!found) return 99;
        elements[1].SemanticName="COLOR___";
    }
    union { U64 align; unsigned char b[560]; } stream={0};
    *(unsigned *)(stream.b+0)=1; *(const void **)(stream.b+8)=vertex_case==6?custom_vs:vs; *(U64 *)(stream.b+16)=sizeof(vs);
    *(unsigned *)(stream.b+24)=2; *(const void **)(stream.b+32)=ps; *(U64 *)(stream.b+40)=sizeof(ps);
    *(unsigned *)(stream.b+48)=10; *(unsigned *)(stream.b+52)=3; *(unsigned *)(stream.b+56)=1; *(unsigned *)(stream.b+76)=1;
    *(unsigned *)(stream.b+96)=11; *(unsigned *)(stream.b+152)=12; *(void **)(stream.b+160)=elements; *(unsigned *)(stream.b+168)=2;
    *(unsigned *)(stream.b+176)=14; *(unsigned *)(stream.b+180)=3; *(unsigned *)(stream.b+184)=15; *(unsigned *)(stream.b+188)=28; *(unsigned *)(stream.b+220)=1;
    if (additive || blend_case) {
        static const unsigned states[8][6]={
            {3,10,1,5,8,1}, {2,2,2,2,2,3}, {1,1,4,1,1,5},
            {0,0,0,0,0,0}, {1,1,5,1,1,4}, {2,2,3,2,2,2},
            {2,1,1,2,1,1}, {11,1,1,2,1,1}
        };
        const unsigned *state=blend_case ? states[blend_case-1] : (const unsigned[]){2,2,1,2,2,1};
        *(unsigned *)(stream.b+224)=8;
        *(unsigned *)(stream.b+236)=blend_case!=4;
        for (unsigned i=0;i<6;i++) *(unsigned *)(stream.b+244+4*i)=state[i];
        if (invalid==9) *(unsigned *)(stream.b+248)=16; /* dual source */
        if (invalid==10) *(unsigned *)(stream.b+256)=3; /* color factor for alpha */
        if (invalid==11) *(unsigned *)(stream.b+244)=14; /* dynamic blend constant */
        if (invalid==12) *(unsigned *)(stream.b+240)=1; /* logic operation */
        if (invalid==13) *(unsigned *)(stream.b+252)=6;
        stream.b[272]=blend_case==4?5:blend_case==7?10:15;
    }
    StreamDesc sd={(additive||blend_case)?sizeof(stream.b):224,stream.b};
    hr=SLOT(dev,0x2b0,CreateObject)(dev,&sd,&IID_ID3D12PipelineState,&pipeline);
    if (vertex_case==4 || vertex_case==5) {
        if (FAILED(hr) && !pipeline) {say("invalid vertex layout rejected\r\n");return 43;} return 96;
    }
    if (invalid>=9) {
        say("D3D12X invalid blend case armed\r\n");
        if (FAILED(hr) && !pipeline) { say("D3D12X invalid blend case rejected\r\n"); return 43; }
        return 95;
    }
    if (hr!=S_OK) return 9;
    ID3D12CommandAllocator *allocator=0; ID3D12GraphicsCommandList *list=0; ID3D12CommandQueue *queue=0;
    if (ID3D12Device_CreateCommandAllocator((ID3D12Device *)dev,0,&IID_ID3D12CommandAllocator,(void **)&allocator)!=S_OK) return 10;
    if (ID3D12Device_CreateCommandList((ID3D12Device *)dev,0,0,allocator,pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list)!=S_OK) return 11;
    D3D12_TEXTURE_COPY_LOCATION us={0},ud={0};
    us.pResource=upload; us.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    us.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; us.PlacedFootprint.Footprint.Depth=1;
    ud.pResource=texture; ud.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX;
    for (unsigned m=0;m<3;m++) {
        us.PlacedFootprint.Offset=offsets[m]; us.PlacedFootprint.Footprint.Width=widths[m];
        us.PlacedFootprint.Footprint.Height=heights[m]; us.PlacedFootprint.Footprint.RowPitch=pitches[m];
        ud.SubresourceIndex=m;
        if (invalid==3 && m==2) { say("D3D12X invalid mip case armed\r\n"); ud.SubresourceIndex=3; }
        if (invalid==5 && m==2) { say("D3D12X invalid mip case armed\r\n"); us.PlacedFootprint.Footprint.RowPitch=128; }
        if (invalid==6 && m==2) { say("D3D12X invalid mip case armed\r\n"); us.PlacedFootprint.Offset=3073; }
        SLOT(list,0x80,Copy)(list,&ud,0,0,0,&us,0);
    }
    /* This bridge supports whole-resource barriers, not per-mip barriers. */
    Barrier b={0,0,texture,{0xffffffffULL | (0x400ULL<<32),0x80}}; SLOT(list,0xd0,BarrierFn)(list,1,&b);
    D3D12_VERTEX_BUFFER_VIEW view={SLOT(vb,0x58,Handle)(vb),vertex_case?(vertex_case==3?20:112):72,vertex_case?32:24};
    float viewport[6]={0,0,8,8,0,1};
    SLOT(list,0xa0,Topology)(list,4); SLOT(list,0xa8,SetArray)(list,1,viewport);
    SLOT(list,0x180,Vertices)(list,0,1,&view); SLOT(list,0x190,Targets)(list,1,&cpu[0],0,0);
    if (vertex_case==2 || vertex_case==7) {D3D12_VERTEX_BUFFER_VIEW uv={SLOT(uvb,0x58,Handle)(uvb),16,4};SLOT(list,0x180,Vertices)(list,1,1,&uv);}
    if (vertex_case==8) {D3D12_INDEX_BUFFER_VIEW ib={SLOT(uvb,0x58,Handle)(uvb),8,DXGI_FORMAT_R16_UINT};SLOT(list,0x178,IndexBuffer)(list,&ib);}
    if (additive || blend_case) {
        const float clear[4]={0.125f,blend_case?0.25f:0.125f,blend_case?0.5f:0.125f,blend_case?0.75f:0.125f};
        SLOT(list,0x1a0,Clear)(list,cpu[0],clear,0,0);
    }
    for (unsigned m=0;m<3;m++) {
        int scissor[4]={(int)(m*2),0,(int)(m*2+2),8}; void *pair[2]={heaps[m+1],heaps[4]};
        SLOT(list,0xb0,SetArray)(list,1,scissor); SLOT(list,0x100,SetHeaps)(list,2,pair);
        SLOT(list,0x120,SetTable)(list,1,gpu[4]); SLOT(list,0x120,SetTable)(list,2,gpu[m+1]);
        if (vertex_case==8) SLOT(list,0x68,DrawIndexed)(list,3,1,1,-1,0);
        else SLOT(list,0x60,Draw)(list,3,(vertex_case==2 || vertex_case==7)?3:1,0,(vertex_case==2 || vertex_case==7)?2:0);
        if (additive) SLOT(list,0x60,Draw)(list,3,1,0,0);
    }
    Barrier texture_read={0,0,texture,{0xffffffffULL | (0x80ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&texture_read);
    D3D12_TEXTURE_COPY_LOCATION mipdst={0},mipsrc={0};
    mipdst.pResource=out; mipdst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    mipdst.PlacedFootprint.Offset=2048; mipdst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM;
    mipdst.PlacedFootprint.Footprint.Width=3; mipdst.PlacedFootprint.Footprint.Height=2;
    mipdst.PlacedFootprint.Footprint.Depth=1; mipdst.PlacedFootprint.Footprint.RowPitch=256;
    mipsrc.pResource=texture; mipsrc.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; mipsrc.SubresourceIndex=1;
    SLOT(list,0x80,Copy)(list,&mipdst,0,0,0,&mipsrc,0);
    Barrier rb={0,0,target,{0xffffffffULL | (4ULL<<32),0x800}}; SLOT(list,0xd0,BarrierFn)(list,1,&rb);
    D3D12_TEXTURE_COPY_LOCATION dst={0},src={0}; dst.pResource=out; dst.Type=D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT;
    dst.PlacedFootprint.Footprint.Format=DXGI_FORMAT_R8G8B8A8_UNORM; dst.PlacedFootprint.Footprint.Width=8; dst.PlacedFootprint.Footprint.Height=8; dst.PlacedFootprint.Footprint.Depth=1; dst.PlacedFootprint.Footprint.RowPitch=256;
    src.pResource=target; src.Type=D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX; SLOT(list,0x80,Copy)(list,&dst,0,0,0,&src,0);
    hr=ID3D12GraphicsCommandList_Close(list);
    if (vertex_case==3) {
        if (FAILED(hr)) {say("invalid vertex range rejected\r\n");return 43;} return 97;
    }
    if (invalid) {
        if (FAILED(hr)) { say("D3D12X invalid mip case rejected\r\n"); return 43; }
        return 94;
    }
    if (hr!=S_OK) return 12;
    if (((volatile unsigned char *)p)[3*256+4]!=0x77) return 13;
    SLOT(upload,0x10,Release)(upload); SLOT(vb,0x10,Release)(vb); SLOT(out,0x10,Release)(out);
    if (uvb) SLOT(uvb,0x10,Release)(uvb);
    SLOT(target,0x10,Release)(target); SLOT(texture,0x10,Release)(texture); SLOT(pipeline,0x10,Release)(pipeline);
    for (unsigned i=0;i<5;i++) SLOT(heaps[i],0x10,Release)(heaps[i]);
    D3D12_COMMAND_QUEUE_DESC qd={0,0,0,0};
    if (ID3D12Device_CreateCommandQueue((ID3D12Device *)dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue)!=S_OK) return 14;
    ID3D12CommandList *lists[]={(ID3D12CommandList *)list}; ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    volatile unsigned char *pixels=p;
    for (unsigned m=0;m<3;m++) for (unsigned c=0;c<4;c++) {
        int s=colors[m][c], d=(int[]){32,64,128,191}[c];
        int want=s;
        if (additive) want=32+2*s;
        else if (blend_case==1) want=(s*s+d*(255-d)+127)/255;
        else if (blend_case==2) want=c==3?d-s:s-d;
        else if (blend_case==3) want=c==3?(s>d?s:d):(s<d?s:d);
        else if (blend_case==4) want=(c==0 || c==2)?s:d;
        else if (blend_case==5) want=c==3?(s<d?s:d):(s>d?s:d);
        else if (blend_case==6) want=c==3?s-d:d-s;
        else if (blend_case==7) want=(c==1 || c==3)?s:d;
        else if (blend_case==8) {
            int sat=colors[m][3] < 255-191 ? colors[m][3] : 255-191;
            want=c==3?s:(s*sat+127)/255;
        }
        if (want>255) want=255;
        if (want<0) want=0;
        int delta=(int)pixels[3*256+(m*2+1)*4+c]-want;
        if (delta < -2 || delta > 2) return 20+m;
    }
    if (additive || blend_case) for (unsigned c=0;c<4;c++) {
        int delta=(int)pixels[3*256+7*4+c]-(blend_case?(int[]){32,64,128,191}[c]:32);
        if (delta < -1 || delta > 1) return 27;
    }
    for (unsigned y=0;y<2;y++) {
        for (unsigned x=0;x<3;x++) for (unsigned c=0;c<4;c++)
            if (pixels[2048+y*256+x*4+c]!=colors[1][c]) return 25;
        for (unsigned x=12;x<256;x++) if (pixels[2048+y*256+x]!=0x77) return 26;
    }
    for (unsigned y=0;y<8;y++) for (unsigned x=32;x<256;x++) if (pixels[y*256+x]!=0x77) return 24;
    SLOT(queue,0x10,Release)(queue); SLOT(list,0x10,Release)(list); SLOT(allocator,0x10,Release)(allocator); SLOT(dev,0x10,Release)(dev);
    say(vertex_case ? "D3D12X flexible vertex inputs passed\r\n" : additive ? "D3D12X additive blending passed\r\n" : blend_case ? "D3D12X blend mapping passed\r\n" : "D3D12X mip sampling passed\r\n"); return 42;
}
