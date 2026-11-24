/* Self-authored D3D12.X device and empty-list integration fixture. */
#define COBJMACROS
#define INITGUID
#include <windows.h>
#include <initguid.h>
#include <d3d12.h>
__declspec(dllimport) HRESULT __stdcall D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
static void print(const char *s, DWORD n) { DWORD written; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), s, n, &written, 0); }
#define SAY(s) print(s, sizeof(s)-1)
unsigned mainCRTStartup(void *unused) {
    (void)unused;
    unsigned params = 0x00020004;
    ID3D12Device *dev = 0;
    HRESULT hr = D3D12XboxCreateDevice(0, &params, &IID_ID3D12Device, (void **)&dev);
    if (hr != S_OK || !dev) { SAY("device creation failed\r\n"); return 1; }
    if (ID3D12Device_GetNodeCount(dev) != 1) { SAY("node count failed\r\n"); return 2; }
    D3D12_COMMAND_QUEUE_DESC desc = { D3D12_COMMAND_LIST_TYPE_DIRECT, 0, 0, 0 };
    ID3D12CommandQueue *queue = 0;
    hr = ID3D12Device_CreateCommandQueue(dev, &desc, &IID_ID3D12CommandQueue, (void **)&queue);
    if (hr != S_OK || !queue) { SAY("queue creation failed\r\n"); return 3; }
    ID3D12CommandAllocator *allocator = 0;
    hr = ID3D12Device_CreateCommandAllocator(dev, D3D12_COMMAND_LIST_TYPE_DIRECT, &IID_ID3D12CommandAllocator, (void **)&allocator);
    if (hr != S_OK || !allocator) { SAY("allocator creation failed\r\n"); return 4; }
    ID3D12GraphicsCommandList *list = 0;
    hr = ID3D12Device_CreateCommandList(dev, 0, D3D12_COMMAND_LIST_TYPE_DIRECT, allocator, 0, &IID_ID3D12GraphicsCommandList, (void **)&list);
    if (hr != S_OK || !list) { SAY("list creation failed\r\n"); return 5; }
    if (ID3D12GraphicsCommandList_Close(list) != S_OK) { SAY("list close failed\r\n"); return 6; }
    ID3D12CommandList *lists[] = { (ID3D12CommandList *)list };
    ID3D12CommandQueue_ExecuteCommandLists(queue, 1, lists);
    ID3D12Fence *fence = 0;
    hr = ID3D12Device_CreateFence(dev, 0, D3D12_FENCE_FLAG_NONE, &IID_ID3D12Fence, (void **)&fence);
    if (hr != S_OK || !fence) { SAY("fence creation failed\r\n"); return 7; }
    if (ID3D12CommandQueue_Signal(queue, fence, 42) != S_OK || ID3D12Fence_GetCompletedValue(fence) != 42) { SAY("fence signal failed\r\n"); return 8; }
    ID3D12Fence_Release(fence);
    ID3D12GraphicsCommandList_Release(list);
    ID3D12CommandAllocator_Release(allocator);
    ID3D12CommandQueue_Release(queue);
    ID3D12Device_Release(dev);
    SAY("D3D12X device list and fence passed\r\n");
    return 42;
}
