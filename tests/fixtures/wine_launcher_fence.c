/* Ordinary Wine PE: compiler TLS, translated compute, and real fence ordering. */
#include <stdint.h>
#define mainCRTStartup unused_compute_entry
#include "windows_d3d12x_compute.c"
#undef mainCRTStartup

typedef HRESULT (WINAPI *CreateFenceFn)(void *, U64, unsigned, const IID *, void **);
typedef U64 (WINAPI *CompletedFn)(void *);
typedef HRESULT (WINAPI *EventFn)(void *, U64, HANDLE);
typedef HRESULT (WINAPI *FenceSignalFn)(void *, U64);
typedef HRESULT (WINAPI *QueueFenceFn)(void *, void *, U64);
typedef ULONG (WINAPI *ReleaseFn)(void *);
typedef struct { unsigned kind, flags, engine_or_pipe, queue; } QueueXDesc;
typedef HRESULT (WINAPI *CreateQueueXFn)(void *, const QueueXDesc *, const IID *, void **);
static const IID device11 = {0x5405c344,0xd457,0x444e,{0xb4,0xdd,0x23,0x66,0xe4,0x5a,0xee,0x39}};
static const IID invalid_iid = {0x7823c112,0xace1,0x4bcd,{1,2,3,4,5,6,7,8}};
static _Thread_local int tls_value = 17;
static volatile LONG tls_process, tls_attach, tls_detach, tls_bad;
static void NTAPI tls_callback(PVOID image, DWORD reason, PVOID reserved) {
    (void)image; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        if (tls_value != 17) InterlockedIncrement(&tls_bad);
        if (reason == DLL_PROCESS_ATTACH) InterlockedIncrement(&tls_process);
        else InterlockedIncrement(&tls_attach);
    } else if (reason == DLL_THREAD_DETACH) InterlockedIncrement(&tls_detach);
}
__attribute__((used, section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK fence_tls_callback = tls_callback;
static DWORD WINAPI tls_worker(void *unused) {
    (void)unused;
    if (tls_value != 17) return 1;
    tls_value = 23;
    return tls_value == 23 ? 0 : 2;
}
static DWORD WINAPI blocking_wait(void *fence) {
    return SLOT(fence,0x48,EventFn)(fence,15,NULL) == S_OK ? 0 : 1;
}
#define CHECK(c, msg) do { if (!(c)) { SAY(msg "\r\n"); return 1; } } while (0)
#define RELEASE(o) SLOT(o,0x10,ReleaseFn)(o)

int main(void) {
    CHECK(tls_value == 17 && tls_process == 1 && !tls_bad, "TLS process state failed");
    tls_value = 99;
    HANDLE thread = CreateThread(NULL, 0, tls_worker, NULL, 0, NULL);
    DWORD code = 1;
    CHECK(thread && WaitForSingleObject(thread, 5000) == WAIT_OBJECT_0 &&
          GetExitCodeThread(thread, &code) && !code, "TLS worker failed");
    CloseHandle(thread);
    CHECK(tls_value == 99 && tls_attach >= 1 && tls_detach >= 1 && !tls_bad, "TLS thread state failed");
    SAY("Fence fixture TLS passed\r\n");

    unsigned version = 0x00020004;
    ID3D12Device *dev = NULL;
    CHECK(D3D12XboxCreateDevice(NULL, &version, &IID_ID3D12Device, (void **)&dev) == S_OK && dev, "device failed");
    CreateFenceFn create_fence = SLOT(dev,0x120,CreateFenceFn);
    void *bad = NULL, *gate = NULL, *done = NULL;
    CHECK(FAILED(create_fence(dev, 0, 1, &IID_ID3D12Fence, &bad)) && !bad, "shared fence flag accepted");
    CHECK(FAILED(create_fence(dev, 0, 2, &IID_ID3D12Fence, &bad)) && !bad, "cross-adapter fence flag accepted");
    CHECK(FAILED(create_fence(dev, 0, 0x80000000u, &IID_ID3D12Fence, &bad)) && !bad, "unknown fence flag accepted");
    CHECK(FAILED(create_fence(dev, 0, 0, &invalid_iid, &bad)) && !bad, "invalid fence IID accepted");
    CHECK(FAILED(create_fence(dev, 0, 0, &IID_ID3D12Fence, NULL)), "null fence output accepted");
    CHECK(FAILED(create_fence(dev, UINT64_MAX, 0, &IID_ID3D12Fence, &bad)) && !bad, "reserved fence value accepted");
    CHECK(create_fence(dev, 5, 0, &IID_ID3D12Fence, &gate) == S_OK && gate, "gate fence failed");
    CHECK(create_fence(dev, 11, 0, &IID_ID3D12Fence, &done) == S_OK && done, "completion fence failed");
    CompletedFn completed = SLOT(done,0x40,CompletedFn);
    EventFn event_on = SLOT(done,0x48,EventFn);
    CHECK(SLOT(gate,0x40,CompletedFn)(gate) == 5 && completed(done) == 11, "initial fence values wrong");
    HANDLE event = CreateEventW(NULL, FALSE, FALSE, NULL);
    CHECK(event && event_on(done, 12, event) == S_OK, "event registration failed");
    CHECK(WaitForSingleObject(event, 50) == WAIT_TIMEOUT && completed(done) == 11, "unsignaled event completed");

    HeapDesc hd = {{D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,1,D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,0},{0,0}};
    void *heap = NULL, *resource = NULL, *memory = NULL, *pipeline = NULL;
    CHECK(SLOT(dev,0x70,CreateHeap)(dev,&hd,&IID_ID3D12DescriptorHeap,&heap) == S_OK && heap, "heap failed");
    U64 handle = SLOT(heap,0x48,Handle)(heap);
    D3D12_HEAP_PROPERTIES hp = {D3D12_HEAP_TYPE_DEFAULT,0,0,0,0};
    D3D12_RESOURCE_DESC rd = {D3D12_RESOURCE_DIMENSION_BUFFER,0,64,1,1,1,DXGI_FORMAT_UNKNOWN,{1,0},D3D12_TEXTURE_LAYOUT_ROW_MAJOR,D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS};
    CHECK(SLOT(dev,0xd8,CreateResource)(dev,&hp,0,&rd,0,0,&IID_ID3D12Resource,&resource) == S_OK && resource, "resource failed");
    CHECK(SLOT(resource,0x40,Map)(resource,0,0,&memory) == S_OK && memory, "map failed");
    ((volatile unsigned *)memory)[0] = 7; ((volatile unsigned *)memory)[1] = 1;
    D3D12_UNORDERED_ACCESS_VIEW_DESC uav = {0};
    uav.Format = DXGI_FORMAT_R32_TYPELESS; uav.ViewDimension = D3D12_UAV_DIMENSION_BUFFER;
    uav.Buffer.NumElements = 16; uav.Buffer.Flags = D3D12_BUFFER_UAV_FLAG_RAW;
    SLOT(dev,0x98,CreateUav)(dev,resource,NULL,&uav,handle);
    ComputeDesc pd = {NULL,{shader,sizeof(shader)}};
    CHECK(SLOT(dev,0x58,CreatePipeline)(dev,&pd,&IID_ID3D12PipelineState,&pipeline) == S_OK && pipeline, "pipeline failed");
    ID3D12CommandAllocator *allocator = NULL;
    ID3D12GraphicsCommandList *list = NULL;
    CHECK(ID3D12Device_CreateCommandAllocator(dev,D3D12_COMMAND_LIST_TYPE_COMPUTE,&IID_ID3D12CommandAllocator,(void **)&allocator) == S_OK && allocator, "allocator failed");
    CHECK(ID3D12Device_CreateCommandList(dev,0,D3D12_COMMAND_LIST_TYPE_COMPUTE,allocator,(ID3D12PipelineState *)pipeline,&IID_ID3D12GraphicsCommandList,(void **)&list) == S_OK && list, "list failed");
    ID3D12GraphicsCommandList_SetPipelineState(list,(ID3D12PipelineState *)pipeline);
    SLOT(list,0x100,SetHeaps)(list,1,&heap);
    SLOT(list,0x118,SetTable)(list,1,handle);
    unsigned constants[4] = {41,0,0,0};
    SLOT(list,0x138,SetConstants)(list,0,4,constants,0);
    SLOT(list,0x70,Dispatch)(list,1,1,1);
    CHECK(ID3D12GraphicsCommandList_Close(list) == S_OK, "list close failed");
    D3D12_COMMAND_QUEUE_DESC qd = {D3D12_COMMAND_LIST_TYPE_COMPUTE,0,0,0};
    ID3D12CommandQueue *queue = NULL, *second = NULL;
    CHECK(ID3D12Device_CreateCommandQueue(dev,&qd,&IID_ID3D12CommandQueue,(void **)&queue) == S_OK && queue, "queue failed");
    CHECK(ID3D12Device_CreateCommandQueue(dev,&qd,&IID_ID3D12CommandQueue,(void **)&second) == S_OK && second, "second queue failed");
    ID3D12CommandList *lists[] = {(ID3D12CommandList *)list};
    ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    CHECK(SLOT(queue,0x70,QueueFenceFn)(queue,done,12) == S_OK, "first queue signal failed");
    CHECK(WaitForSingleObject(event, 10000) == WAIT_OBJECT_0 && completed(done) == 12 && ((volatile unsigned *)memory)[0] == 42, "GPU before signal failed");
    SAY("Fence signal after GPU readback passed\r\n");

    ((volatile unsigned *)memory)[0] = 7;
    CHECK(event_on(done, 13, event) == S_OK, "second event registration failed");
    CHECK(SLOT(queue,0x78,QueueFenceFn)(queue,gate,6) == S_OK, "queue wait failed");
    ID3D12CommandQueue_ExecuteCommandLists(queue,1,lists);
    CHECK(SLOT(queue,0x70,QueueFenceFn)(queue,done,13) == S_OK, "queued signal failed");
    CHECK(WaitForSingleObject(event, 50) == WAIT_TIMEOUT && completed(done) == 12 && ((volatile unsigned *)memory)[0] == 7, "wait failed to order GPU work");
    RELEASE(queue); queue = NULL;
    CHECK(SLOT(second,0x70,QueueFenceFn)(second,gate,6) == S_OK, "second queue signal failed");
    CHECK(WaitForSingleObject(event, 10000) == WAIT_OBJECT_0 && completed(done) == 13 && ((volatile unsigned *)memory)[0] == 42, "pending queue lifetime or ordering failed");
    CHECK(FAILED(event_on(done, 14, (HANDLE)(uintptr_t)0x12345)), "invalid event handle accepted");
    HANDLE original = CreateEventW(NULL, FALSE, FALSE, NULL), duplicate = NULL;
    CHECK(original && DuplicateHandle(GetCurrentProcess(), original, GetCurrentProcess(), &duplicate, 0, FALSE,
          DUPLICATE_SAME_ACCESS), "event duplication failed");
    CHECK(event_on(done, 14, original) == S_OK && CloseHandle(original), "event registration lifetime failed");
    CHECK(SLOT(second,0x70,QueueFenceFn)(second,done,14) == S_OK &&
          WaitForSingleObject(duplicate, 1000) == WAIT_OBJECT_0, "event lifetime or signal failed");
    CloseHandle(duplicate);
    thread = CreateThread(NULL, 0, blocking_wait, done, 0, NULL);
    CHECK(thread && WaitForSingleObject(thread, 50) == WAIT_TIMEOUT, "null-event wait failed to block");
    CHECK(SLOT(done,0x50,FenceSignalFn)(done,15) == S_OK &&
          WaitForSingleObject(thread, 10000) == WAIT_OBJECT_0 &&
          GetExitCodeThread(thread, &code) && !code, "null-event wait did not wake");
    CloseHandle(thread);
    CHECK(SLOT(gate,0x50,FenceSignalFn)(gate,9) == S_OK && SLOT(gate,0x40,CompletedFn)(gate) == 9, "CPU signal failed");
    CHECK(event_on(gate, 8, event) == S_OK && WaitForSingleObject(event, 1000) == WAIT_OBJECT_0, "already completed event failed");
    CHECK(FAILED(SLOT(second,0x70,QueueFenceFn)(second,gate,UINT64_MAX)) &&
          FAILED(SLOT(second,0x78,QueueFenceFn)(second,NULL,1)) &&
          FAILED(event_on(gate, UINT64_MAX, event)), "invalid queue or fence arguments accepted");
    ID3D12Device *other = NULL;
    void *foreign_fence = NULL;
    CHECK(D3D12XboxCreateDevice(NULL, &version, &IID_ID3D12Device, (void **)&other) == S_OK && other &&
          SLOT(other,0x120,CreateFenceFn)(other,0,0,&IID_ID3D12Fence,&foreign_fence) == S_OK && foreign_fence,
          "second device fence failed");
    CHECK(FAILED(SLOT(second,0x70,QueueFenceFn)(second,foreign_fence,1)) &&
          FAILED(SLOT(second,0x78,QueueFenceFn)(second,foreign_fence,1)), "cross-device fence accepted");
    RELEASE(foreign_fence); RELEASE(other);
    RELEASE(second); RELEASE(done); RELEASE(gate); CloseHandle(event);
    RELEASE(list); RELEASE(allocator); RELEASE(pipeline); RELEASE(resource); RELEASE(heap); RELEASE(dev);

    void *newdev = NULL, *newqueue = NULL, *newfence = NULL;
    CHECK(D3D12XboxCreateDevice(NULL,&version,&device11,&newdev) == S_OK && newdev, "new ABI device failed");
    QueueXDesc xqd = {0,0,0,0};
    CHECK(SLOT(newdev,0x180,CreateQueueXFn)(newdev,&xqd,&IID_ID3D12CommandQueue,&newqueue) == S_OK && newqueue, "new ABI queue failed");
    CHECK(SLOT(newdev,0x120,CreateFenceFn)(newdev,2,0,&IID_ID3D12Fence,&newfence) == S_OK && newfence, "new ABI fence failed");
    CHECK(SLOT(newqueue,0x88,QueueFenceFn)(newqueue,newfence,2) == S_OK && SLOT(newqueue,0x80,QueueFenceFn)(newqueue,newfence,3) == S_OK && SLOT(newfence,0x40,CompletedFn)(newfence) == 3, "new ABI queue fence slots failed");
    RELEASE(newqueue); RELEASE(newfence); RELEASE(newdev);
    SAY("D3D12X Wine Metal fence ordering passed\r\n");
    return 42;
}
