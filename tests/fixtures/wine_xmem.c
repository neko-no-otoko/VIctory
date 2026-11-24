/* A normal PE title with static TLS and named XMem imports. */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>

#ifdef UNSUPPORTED_IMPORT
__declspec(dllimport) void WINAPI VIctoryUnsupportedImport(void);
int main(void) { VIctoryUnsupportedImport(); return 0; }
#else
__declspec(dllimport) void *WINAPI XMemAlloc(size_t, uint64_t);
__declspec(dllimport) void WINAPI XMemFree(void *, uint64_t);
__declspec(dllimport) void *WINAPI XMemAllocDefault(size_t, uint64_t);
__declspec(dllimport) void WINAPI XMemFreeDefault(void *, uint64_t);
__declspec(dllimport) BOOL WINAPI XMemIsToolsMemAvailable(void);
__declspec(dllimport) BOOL WINAPI XMemAllocatePhysicalPages(uint64_t, size_t *, size_t *);
__declspec(dllimport) BOOL WINAPI XMemFreePhysicalPages(size_t, const size_t *);
__declspec(dllimport) void *WINAPI XMemMapPhysicalPages(void *, size_t, const size_t *);
__declspec(dllimport) BOOL WINAPI XMemMapPhysicalPagesScatter(void *const *, size_t, const size_t *);
__declspec(dllimport) void *WINAPI XMemVirtualAlloc(void *, size_t, DWORD, uint64_t, DWORD);
typedef struct { MEMORY_BASIC_INFORMATION host; uint64_t flags; uint32_t tag_code; unsigned char tag[4]; } BasicInformation;
__declspec(dllimport) BOOL WINAPI XMemVirtualQuery(HANDLE, const void *, BasicInformation *);
typedef struct { size_t title_limit, title_used, tools_limit, tools_used; uint32_t stack_used, physical_pages; size_t gpu_limit, gpu_used, tools_gpu_limit, tools_gpu_used; uint32_t kernel_used, kernel_limit; } WorkingSet;
__declspec(dllimport) BOOL WINAPI XMemGetWorkingSetStatistics(unsigned, WorkingSet *);
__declspec(dllimport) BOOL WINAPI XMemTransferMemory(unsigned, size_t);
typedef struct { unsigned short major, minor, build, revision; } Version;
typedef struct { Version os, hosting_os; char family[64], form[64]; } AnalyticsInfo;
static int check_analytics(const char *expected) {
    const GUID id = {0xb884675d, 0xb738, 0x4a9c, {0x81,0x5d,0x9a,0x9a,0x1e,0x0c,0x6c,0x9b}};
    HMODULE runtime = LoadLibraryW(L"xgameruntime.dll");
    if (!runtime) return 0;
    typedef HRESULT (WINAPI *Query)(const GUID *, const GUID *, void **);
    Query query = (Query)GetProcAddress(runtime, "QueryApiImpl");
    void *object = NULL;
    if (!query || query(&id, &id, &object) || !object) return 0;
    typedef AnalyticsInfo *(WINAPI *GetInfo)(void *, AnalyticsInfo *);
    AnalyticsInfo info;
    GetInfo get_info = (GetInfo)(*(void ***)object)[3];
    return get_info(object, &info) == &info && !strcmp(info.family, "Xbox") &&
        !strcmp(info.form, expected);
}
#define CHECK(x) do { if (!(x)) { printf("XMem check failed at line %d (error %lu)\n", __LINE__, GetLastError()); return 1; } } while (0)

static volatile LONG hook_allocs, hook_frees;
static void *WINAPI title_alloc(size_t size, uint64_t attributes) {
    InterlockedIncrement(&hook_allocs);
    return XMemAllocDefault(size, attributes);
}
static void WINAPI title_free(void *address, uint64_t attributes) {
    InterlockedIncrement(&hook_frees);
    XMemFreeDefault(address, attributes);
}
__declspec(dllexport) void * const __XMemAllocationHooks__[2] = { (void *)title_alloc, (void *)title_free };

static _Thread_local int tls_value = 17;
static volatile LONG attaches, detaches, bad_tls, process_attach, ready;
static HANDLE gate;
static void NTAPI tls_callback(PVOID image, DWORD reason, PVOID reserved) {
    (void)image; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        if (tls_value != 17) InterlockedIncrement(&bad_tls);
        if (reason == DLL_PROCESS_ATTACH) InterlockedIncrement(&process_attach);
        else InterlockedIncrement(&attaches);
    } else if (reason == DLL_THREAD_DETACH) InterlockedIncrement(&detaches);
}
__attribute__((used, section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK wine_tls_callback = tls_callback;
static DWORD WINAPI worker(void *arg) {
    int value = (int)(INT_PTR)arg;
    if (tls_value != 17) return 1;
    tls_value = value;
    unsigned char *p = XMemAlloc(128, (uint64_t)8 << 24);
    if (!p || ((uintptr_t)p & 255)) return 2;
    memset(p, value, 128);
    InterlockedIncrement(&ready);
    if (WaitForSingleObject(gate, 10000) != WAIT_OBJECT_0) return 3;
    for (int i = 0; i < 128; ++i) if (p[i] != (unsigned char)value) return 4;
    XMemFree(p, (uint64_t)8 << 24);
    return tls_value == value ? 0 : 5;
}
static int module_path(HMODULE module, const WCHAR *variable) {
    WCHAR actual[32768], expected[32768];
    DWORD n = module ? GetModuleFileNameW(module, actual, 32768) : 0;
    DWORD e = GetEnvironmentVariableW(variable, expected, 32768);
    return n && n < 32768 && e && e < 32768 && !_wcsicmp(actual, expected);
}
static const char *const exports[] = {
    "XMemAlloc", "XMemFree", "XMemAllocDefault", "XMemFreeDefault", "XMemIsToolsMemAvailable",
    "XMemAllocatePhysicalPages", "XMemFreePhysicalPages", "XMemMapPhysicalPages", "XMemMapPhysicalPagesScatter",
    "XMemVirtualAlloc", "XMemVirtualQuery", "XMemGetWorkingSetStatistics", "XMemTransferMemory"
};
static int named_imports(void) {
    unsigned char *base = (unsigned char *)GetModuleHandleW(NULL);
    IMAGE_DOS_HEADER *dos = (IMAGE_DOS_HEADER *)base;
    IMAGE_NT_HEADERS64 *nt = (IMAGE_NT_HEADERS64 *)(base + dos->e_lfanew);
    IMAGE_IMPORT_DESCRIPTOR *d = (IMAGE_IMPORT_DESCRIPTOR *)(base + nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].VirtualAddress);
    unsigned seen = 0;
    for (; d->Name; ++d) {
        if (_stricmp((char *)(base + d->Name), "xmem.dll")) continue;
        if (!d->OriginalFirstThunk) return 0;
        IMAGE_THUNK_DATA64 *t = (IMAGE_THUNK_DATA64 *)(base + d->OriginalFirstThunk);
        for (; t->u1.AddressOfData; ++t) {
            if (IMAGE_SNAP_BY_ORDINAL64(t->u1.Ordinal)) return 0;
            const char *name = (const char *)((IMAGE_IMPORT_BY_NAME *)(base + t->u1.AddressOfData))->Name;
            unsigned i;
            for (i = 0; i < sizeof(exports)/sizeof(exports[0]); ++i) if (!strcmp(name, exports[i])) break;
            if (i == sizeof(exports)/sizeof(exports[0]) || (seen & (1u << i))) return 0;
            seen |= 1u << i;
        }
    }
    return seen == (1u << (sizeof(exports)/sizeof(exports[0]))) - 1;
}
int main(void) {
    HMODULE xmem = GetModuleHandleW(L"xmem.dll");
    CHECK(!XMemIsToolsMemAvailable());
    HMODULE backend = GetModuleHandleW(L"victory_xmem_backend.dll");
    CHECK(module_path(xmem, L"VICTORY_EXPECT_XMEM_DLL"));
    CHECK(module_path(backend, L"VICTORY_EXPECT_XMEM_BACKEND_DLL"));
    CHECK(named_imports());
    for (unsigned i = 0; i < sizeof(exports)/sizeof(exports[0]); ++i) CHECK(GetProcAddress(xmem, exports[i]));
    CHECK(!GetProcAddress(xmem, "VIctoryUnsupportedImport"));
    CHECK(!GetProcAddress(xmem, "VIctoryXMemMapAlias"));
    typedef uint32_t (WINAPI *MapAlias)(uint64_t, uint64_t, uint64_t);
    MapAlias alias = (MapAlias)GetProcAddress(backend, "VIctoryXMemMapAlias");
    CHECK(alias);
    CHECK(GetProcAddress(GetModuleHandleW(NULL), "__XMemAllocationHooks__"));
    CHECK(tls_value == 17 && process_attach == 1 && !bad_tls);
    tls_value = 99;
    gate = CreateEventW(NULL, TRUE, FALSE, NULL); CHECK(gate);
    HANDLE threads[2] = {CreateThread(NULL, 0, worker, (void *)(INT_PTR)111, 0, NULL), CreateThread(NULL, 0, worker, (void *)(INT_PTR)222, 0, NULL)};
    CHECK(threads[0] && threads[1]);
    ULONGLONG deadline = GetTickCount64() + 10000;
    while (InterlockedCompareExchange(&ready, 0, 0) != 2 && GetTickCount64() < deadline) Sleep(1);
    CHECK(ready == 2 && tls_value == 99 && SetEvent(gate));
    CHECK(WaitForMultipleObjects(2, threads, TRUE, 10000) == WAIT_OBJECT_0);
    for (int i = 0; i < 2; ++i) { DWORD code; CHECK(GetExitCodeThread(threads[i], &code) && !code); CloseHandle(threads[i]); }
    CloseHandle(gate);
    CHECK(attaches == 2 && detaches == 2 && !bad_tls && hook_allocs == 2 && hook_frees == 2);
    unsigned char *p = XMemAllocDefault(4096, (uint64_t)12 << 24);
    CHECK(p && !((uintptr_t)p & 4095)); memset(p, 0xa5, 4096); CHECK(p[0] == 0xa5 && p[4095] == 0xa5);
    XMemFreeDefault(p, (uint64_t)12 << 24); CHECK(hook_allocs == 2 && hook_frees == 2);
    CHECK(!XMemIsToolsMemAvailable());
    char model[32];
    DWORD model_len = GetEnvironmentVariableA("VICTORY_TEST_EXPECT_CONSOLE", model, sizeof(model));
    CHECK(model_len && model_len < sizeof(model));
    int series_s = !strcmp(model, "series-s");
    CHECK(series_s || !strcmp(model, "series-x"));
    CHECK(check_analytics(series_s ? "Xbox Series S" : "Xbox Series X"));
    size_t standard = series_s ? ((size_t)7776 << 20) : ((size_t)7 << 29);
    size_t gpu = series_s ? 0 : ((size_t)10 << 30);
    WorkingSet stats; CHECK(XMemGetWorkingSetStatistics(0, &stats));
    CHECK(stats.title_limit == standard + gpu && stats.gpu_limit == gpu);
    CHECK(!XMemGetWorkingSetStatistics(0, NULL));
    CHECK(!XMemTransferMemory(1, 1));
    CHECK(!XMemTransferMemory(0, SIZE_MAX));
    CHECK(XMemTransferMemory(0, 1));
    CHECK(XMemGetWorkingSetStatistics(0, &stats) && stats.title_limit == standard + gpu - (2u << 20));
    CHECK(!XMemTransferMemory(0, standard));
    CHECK(XMemTransferMemory(0, standard - (2u << 20)));
    CHECK(!XMemTransferMemory(0, 1));
    CHECK(XMemGetWorkingSetStatistics(0, &stats) && stats.title_limit == gpu && stats.gpu_limit == gpu);
    p = XMemVirtualAlloc(NULL, 65536, MEM_RESERVE | MEM_COMMIT, 0, PAGE_READWRITE);
    CHECK(p); p[0] = 42; p[65535] = 81;
    BasicInformation info; memset(&info, 0xcc, sizeof(info));
    CHECK(XMemVirtualQuery(GetCurrentProcess(), p, &info));
    CHECK(info.host.State == MEM_COMMIT && info.host.AllocationBase == p && info.flags == 3 && !info.tag_code);
    CHECK(p[0] == 42 && p[65535] == 81 && VirtualFree(p, 0, MEM_RELEASE));
    size_t count = 2, pages[2];
    CHECK(XMemAllocatePhysicalPages(0, &count, pages) && count == 2 && pages[1] == pages[0] + 16);
    const DWORD physical = MEM_RESERVE | 0x20000000u | 0x400000u;
    unsigned char *source = XMemVirtualAlloc(NULL, 131072, physical, 0, PAGE_NOACCESS);
    unsigned char *dest = XMemVirtualAlloc(NULL, 131072, physical, 0, PAGE_NOACCESS);
    CHECK(source && dest);
    CHECK(XMemMapPhysicalPages(source, 2, pages) == source);
    source[0] = 0x12; source[65536] = 0x34;
    CHECK(alias((uint64_t)(uintptr_t)dest, (uint64_t)(uintptr_t)source, 131072));
    CHECK(dest[0] == 0x12 && dest[65536] == 0x34);
    dest[65536] = 0x56; CHECK(source[65536] == 0x56);
    CHECK(alias((uint64_t)(uintptr_t)dest, 0, 131072));
    /* Exercise the local D3D12.X queue callback against this same XMem
       backend. The separate launcher regression exercises the Metal proxy;
       this deliberately selects the local HLE path for the queue ABI. */
    CHECK(SetEnvironmentVariableA("VICTORY_WINE_METAL_ADDR", NULL));
    HMODULE d3d = LoadLibraryW(L"d3d12_xs.dll");
    CHECK(module_path(d3d, L"VICTORY_EXPECT_D3D12X_DLL"));
    typedef HRESULT (WINAPI *CreateDevice)(const void *, const unsigned *, const GUID *, void **);
    CreateDevice create_device = (CreateDevice)GetProcAddress(d3d, "D3D12XboxCreateDevice");
    const GUID device_iid = {0x189819f1, 0x1db6, 0x4b57, {0xbe,0x54,0x18,0x21,0x33,0x9b,0x85,0xf7}};
    const GUID queue_iid = {0x0ec870a6, 0x5d7e, 0x4c22, {0x8c,0xfc,0x5b,0xaa,0xe0,0x76,0x16,0xed}};
    unsigned version = 0x00020004;
    void *device = NULL, *queue = NULL;
    CHECK(create_device && create_device(NULL, &version, &device_iid, &device) == 0 && device);
    const GUID dxgi_iid = {0x54ec77fa, 0x1377, 0x44e6, {0x8c,0x32,0x88,0xfd,0x5f,0x44,0xc8,0x4c}};
    typedef HRESULT (WINAPI *QueryInterface)(void *, const GUID *, void **);
    typedef HRESULT (WINAPI *GetAdapter)(void *, void **);
    struct AdapterDescription {
        WCHAR name[128];
        unsigned vendor, device, subsystem, revision;
        size_t dedicated_video, dedicated_system, shared_system;
        unsigned luid[2];
    } description;
    typedef HRESULT (WINAPI *GetDescription)(void *, struct AdapterDescription *);
    void *dxgi = NULL, *adapter = NULL;
    CHECK(((QueryInterface)(*(void ***)device)[0])(device, &dxgi_iid, &dxgi) == 0 && dxgi);
    CHECK(((GetAdapter)(*(void ***)dxgi)[7])(dxgi, &adapter) == 0 && adapter);
    CHECK(((GetDescription)(*(void ***)adapter)[8])(adapter, &description) == 0);
    CHECK(description.dedicated_video == gpu && description.shared_system == standard);
    typedef ULONG (WINAPI *Release)(void *);
    ((Release)(*(void ***)adapter)[2])(adapter);
    ((Release)(*(void ***)dxgi)[2])(dxgi);
    struct QueueDesc { unsigned kind; int priority; unsigned flags, nodes; } queue_desc = {0,0,0,1};
    typedef HRESULT (WINAPI *CreateQueue)(void *, const struct QueueDesc *, const GUID *, void **);
    CreateQueue create_queue = (CreateQueue)(*(void ***)device)[8];
    CHECK(create_queue(device, &queue_desc, &queue_iid, &queue) == 0 && queue);
    struct PageRange { unsigned kind, start, count; } range = {0, 0, 2};
    struct PageBatch { uint64_t destination; unsigned count; const struct PageRange *ranges; } batch = {(uint64_t)(uintptr_t)dest, 1, &range};
    typedef HRESULT (WINAPI *CopyPages)(void *, unsigned, const struct PageBatch *, uint64_t, unsigned, unsigned);
    CopyPages copy_pages = (CopyPages)(*(void ***)queue)[29];
    CHECK(copy_pages(queue, 1, &batch, (uint64_t)(uintptr_t)source, 2, 0) == 0);
    CHECK(dest[0] == 0x12 && dest[65536] == 0x56);
    range.kind = 2;
    CHECK(copy_pages(queue, 1, &batch, 0, 0, 0) == 0);
    CHECK(XMemVirtualQuery(GetCurrentProcess(), dest, &info) && info.host.State == MEM_RESERVE);
    ((Release)(*(void ***)queue)[2])(queue);
    ((Release)(*(void ***)device)[2])(device);
    CHECK(XMemMapPhysicalPages(source, 2, NULL) == source);
    void *addresses[2] = {source, source + 65536};
    CHECK(XMemMapPhysicalPagesScatter(addresses, 2, pages));
    CHECK(source[0] == 0x12 && source[65536] == 0x56);
    CHECK(XMemMapPhysicalPagesScatter(addresses, 2, NULL));
    CHECK(XMemFreePhysicalPages(2, pages));
    CHECK(VirtualFree(source, 0, MEM_RELEASE) && VirtualFree(source + 65536, 0, MEM_RELEASE));
    CHECK(VirtualFree(dest, 0, MEM_RELEASE) && VirtualFree(dest + 65536, 0, MEM_RELEASE));
    puts("XMem named imports, TLS, hooks, virtual memory and shared physical aliases passed");
    return 0;
}
#endif
