/* Ordinary PE guest: real named imports, compiler TLS and the AcpHal ABI. */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>
#ifdef UNSUPPORTED_IMPORT
__declspec(dllimport) void WINAPI VIctoryUnsupportedImport(void);
int main(void) { VIctoryUnsupportedImport(); return 0; }
#else
#pragma pack(push, 1)
typedef struct { uint32_t counts[6]; void *arrays[6]; uint32_t apu[6]; } Shape;
typedef struct { uint32_t kind, dropped, usec; uint8_t data[16]; } Message;
#pragma pack(pop)
typedef struct Acp Acp;
typedef struct {
    HRESULT (WINAPI *connect)(Acp *, uint32_t, uint32_t);
    HRESULT (WINAPI *disconnect)(Acp *);
    HRESULT (WINAPI *submit)(Acp *, uint32_t, uint64_t, uint32_t, const void *, uint32_t);
    _Bool (WINAPI *pop)(Acp *, Message *);
    uint32_t (WINAPI *count)(Acp *);
    void (WINAPI *release)(Acp *);
} Methods;
struct Acp { const Methods *vtable; };
__declspec(dllimport) HRESULT WINAPI AcpHalAllocateShapeContexts(Shape *);
__declspec(dllimport) HRESULT WINAPI AcpHalReleaseShapeContexts(void);
__declspec(dllimport) HRESULT WINAPI AcpHalCreate(Acp **);
__declspec(dllimport) HRESULT WINAPI ApuAlloc(void **, uint32_t *, uint32_t, uint32_t, uint32_t);
__declspec(dllimport) HRESULT WINAPI ApuCreateHeap(uint32_t, uint32_t);
__declspec(dllimport) void WINAPI ApuFree(void *);
__declspec(dllimport) _Bool WINAPI ApuIsVirtualAddressValid(const void *, uint32_t);
__declspec(dllimport) void *WINAPI ApuMapApuAddress(uint32_t);
__declspec(dllimport) uint32_t WINAPI ApuMapVirtualAddress(const void *);
#define CHECK(x) do { if (!(x)) { printf("AcpHal check failed at line %d (error %lu)\n", __LINE__, GetLastError()); return 1; } } while (0)
#define INVALID ((HRESULT)0x80070057u)
#define FAIL ((HRESULT)0x80004005u)
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
    void *p = NULL; uint32_t apu = 0;
    if (tls_value != 17) return 1;
    tls_value = value;
    if (ApuAlloc(&p, &apu, 256, 256, 0) || !p || ((uintptr_t)p & 255) || (apu & 255)) return 2;
    memset(p, value, 256);
    InterlockedIncrement(&ready);
    if (WaitForSingleObject(gate, 10000) != WAIT_OBJECT_0) return 3;
    if (ApuMapVirtualAddress((char *)p + 17) != apu + 17 || ApuMapApuAddress(apu + 17) != (char *)p + 17 || !ApuIsVirtualAddressValid(p, 256)) return 4;
    for (int i = 0; i < 256; ++i) if (((unsigned char *)p)[i] != (unsigned char)value) return 5;
    ApuFree(p);
    if (ApuMapVirtualAddress(p) || ApuMapApuAddress(apu)) return 6;
    return tls_value == value ? 0 : 7;
}
static const char *const exports[] = { "AcpHalAllocateShapeContexts", "AcpHalReleaseShapeContexts", "AcpHalCreate", "ApuAlloc", "ApuCreateHeap", "ApuFree", "ApuIsVirtualAddressValid", "ApuMapApuAddress", "ApuMapVirtualAddress" };
static int named_imports(void) {
    unsigned char *base = (unsigned char *)GetModuleHandleW(NULL);
    IMAGE_NT_HEADERS64 *nt = (IMAGE_NT_HEADERS64 *)(base + ((IMAGE_DOS_HEADER *)base)->e_lfanew);
    IMAGE_IMPORT_DESCRIPTOR *d = (IMAGE_IMPORT_DESCRIPTOR *)(base + nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].VirtualAddress);
    unsigned seen = 0;
    for (; d->Name; ++d) {
        if (_stricmp((char *)(base + d->Name), "acphal.dll")) continue;
        if (!d->OriginalFirstThunk) return 0;
        IMAGE_THUNK_DATA64 *t = (IMAGE_THUNK_DATA64 *)(base + d->OriginalFirstThunk);
        for (; t->u1.AddressOfData; ++t) {
            if (IMAGE_SNAP_BY_ORDINAL64(t->u1.Ordinal)) return 0;
            const char *name = (const char *)((IMAGE_IMPORT_BY_NAME *)(base + t->u1.AddressOfData))->Name;
            unsigned i; for (i = 0; i < 9; ++i) if (!strcmp(name, exports[i])) break;
            if (i == 9 || (seen & (1u << i))) return 0;
            seen |= 1u << i;
        }
    }
    return seen == 511;
}
static int module_path(HMODULE module) {
    WCHAR actual[32768], expected[32768];
    DWORD n = module ? GetModuleFileNameW(module, actual, 32768) : 0;
    DWORD e = GetEnvironmentVariableW(L"VICTORY_EXPECT_ACPHAL_DLL", expected, 32768);
    return n && n < 32768 && e && e < 32768 && !_wcsicmp(actual, expected);
}
static void put_bits(unsigned char *p, unsigned at, uint32_t value, unsigned width) {
    for (unsigned i = 0; i < width; ++i) {
        unsigned bit = at + i, mask = 0x80u >> (bit % 8);
        p[bit / 8] = (unsigned char)((p[bit / 8] & ~mask) | (((value >> (width - 1 - i)) & 1) ? mask : 0));
    }
}
int main(void) {
    HMODULE module = GetModuleHandleW(L"acphal.dll");
    CHECK(module_path(module) && named_imports());
    for (unsigned i = 0; i < 9; ++i) CHECK(GetProcAddress(module, exports[i]));
    CHECK(!GetProcAddress(module, "VIctoryUnsupportedImport") && !GetProcAddress(module, "ApuUnknown"));
    CHECK(ApuCreateHeap(0, 0) == INVALID);
    CHECK(ApuCreateHeap(64 << 20, 0) == 0);
    CHECK(ApuCreateHeap(64 << 20, 0) == (HRESULT)0x8aca0001u);
    void *p = NULL; uint32_t apu = 0;
    CHECK(ApuAlloc(NULL, &apu, 16, 16, 0) == INVALID);
    CHECK(ApuAlloc(&p, &apu, 0, 16, 0) == INVALID);
    CHECK(ApuAlloc(&p, &apu, 16, 6, 0) == INVALID);
    CHECK(!ApuMapApuAddress(0) && !ApuMapApuAddress(1) && !ApuMapVirtualAddress(&apu) && !ApuIsVirtualAddressValid(&apu, 0));
    CHECK(tls_value == 17 && process_attach == 1 && !bad_tls);
    tls_value = 99; gate = CreateEventW(NULL, TRUE, FALSE, NULL); CHECK(gate);
    HANDLE threads[2] = {CreateThread(NULL, 0, worker, (void *)(INT_PTR)111, 0, NULL), CreateThread(NULL, 0, worker, (void *)(INT_PTR)222, 0, NULL)};
    CHECK(threads[0] && threads[1]);
    ULONGLONG deadline = GetTickCount64() + 10000;
    while (InterlockedCompareExchange(&ready, 0, 0) != 2 && GetTickCount64() < deadline) Sleep(1);
    CHECK(ready == 2 && tls_value == 99 && SetEvent(gate));
    CHECK(WaitForMultipleObjects(2, threads, TRUE, 10000) == WAIT_OBJECT_0);
    for (int i = 0; i < 2; ++i) { DWORD code; CHECK(GetExitCodeThread(threads[i], &code) && !code); CloseHandle(threads[i]); }
    CloseHandle(gate); CHECK(attaches == 2 && detaches == 2 && !bad_tls);
    CHECK(AcpHalAllocateShapeContexts(NULL) == INVALID && AcpHalReleaseShapeContexts() == FAIL);
    Shape s = {{2, 0, 0, 2, 4, 0}, {0}, {0}};
    CHECK(sizeof(s) == 96 && AcpHalAllocateShapeContexts(&s) == 0);
    for (int i = 0; i < 6; ++i) {
        if (s.counts[i]) CHECK(s.arrays[i] && s.apu[i] && !((uintptr_t)s.arrays[i] & 255) && !(s.apu[i] & 255) && ApuMapApuAddress(s.apu[i]) == s.arrays[i] && ApuMapVirtualAddress(s.arrays[i]) == s.apu[i]);
        else CHECK(!s.arrays[i] && !s.apu[i]);
    }
    Shape second = {{0}, {0}, {0}}; CHECK(AcpHalAllocateShapeContexts(&second) == FAIL);
    Acp *acp = NULL; CHECK(AcpHalCreate(NULL) == INVALID && AcpHalCreate(&acp) == 0 && acp && acp->vtable);
    const Methods *m = acp->vtable;
    CHECK(m->connect(acp, 0, 4) == INVALID && m->disconnect(acp) == FAIL && m->connect(acp, 16, 2) == 0);
    uint32_t mask = 0x92; CHECK(m->submit(acp, 2, 0, 0, &mask, 0) == 0);
    CHECK(ApuAlloc(&p, &apu, 16, 16, 0) == 0 && p && apu); *(uint32_t *)p = 0;
    /* Unknown commands are deliberately acknowledged by the upstream shortcut. */
    CHECK(m->submit(acp, 63, 0x1122334455667788ULL, 0, NULL, apu) == 0 && *(volatile uint32_t *)p == 1);
    Message msg; CHECK(m->count(acp) == 1 && m->pop(acp, &msg) && msg.kind == 0x10 && !msg.dropped);
    uint32_t command; uint64_t id; memcpy(&command, msg.data, 4); memcpy(&id, msg.data + 4, 8);
    CHECK(command == 63 && id == 0x1122334455667788ULL && !m->pop(acp, &msg));
    uint32_t graph[2] = {0, 0x12345678}; CHECK(m->submit(acp, 1, 0, 0, graph, 0) == 0);
    CHECK(m->pop(acp, &msg) && msg.kind == 2); memcpy(&command, msg.data, 4); CHECK(command == graph[1]);
    for (uint64_t i = 1; i <= 4; ++i) CHECK(m->submit(acp, 63, i, 0, NULL, 0) == 0);
    CHECK(m->count(acp) == 2 && m->pop(acp, &msg) && m->pop(acp, &msg));
    CHECK(m->submit(acp, 63, 5, 0, NULL, 0) == 0 && m->pop(acp, &msg) && msg.dropped == 2);
    mask = 0x10; CHECK(m->submit(acp, 3, 0, 0, &mask, 0) == 0 && m->submit(acp, 63, 6, 0, NULL, 0) == 0 && !m->count(acp));
    ApuFree(p);
    /* Same synthetic silent XMA stream as the HLE decoder unit test. */
    void *input = NULL, *output = NULL; uint32_t in_apu = 0, out_apu = 0;
    CHECK(ApuAlloc(&input, &in_apu, 4096, 2048, 0) == 0 && ApuAlloc(&output, &out_apu, 1024, 256, 0) == 0);
    memset(input, 0xff, 4096); memset(output, 0x55, 1024);
    for (unsigned packet = 0; packet < 2; ++packet) {
        unsigned base = packet * 2048 * 8; put_bits(input, base, 3u << 26, 32);
        for (unsigned frame = 0; frame < 3; ++frame) {
            unsigned at = base + 32 + frame * 31;
            put_bits(input, at, 31, 15); put_bits(input, at + 15, 2, 2); put_bits(input, at + 17, 0, 14);
        }
    }
    volatile uint32_t *ctx = (volatile uint32_t *)s.arrays[4] + 16;
    ctx[0] = 2 | (1u << 20) | (4u << 22); ctx[1] = (1u << 31) | (3u << 27); ctx[2] = 32; ctx[5] = in_apu; ctx[7] = out_apu;
    uint32_t index = 1; CHECK(m->submit(acp, 5, 0, 0, &index, 0) == 0);
    deadline = GetTickCount64() + 10000;
    while ((ctx[1] & (1u << 31)) && GetTickCount64() < deadline) Sleep(1);
    CHECK(!(ctx[1] & (1u << 31)));
    for (unsigned i = 0; i < 1024; ++i) CHECK(((unsigned char *)output)[i] == 0);
    CHECK(m->submit(acp, 7, 0, 0, &index, 0) == 0);
    ApuFree(input); ApuFree(output);
    CHECK(m->disconnect(acp) == 0 && m->pop(acp, &msg) && msg.kind == 0x80 && m->disconnect(acp) == FAIL);
    m->release(acp);
    uint32_t old = s.apu[4]; CHECK(AcpHalReleaseShapeContexts() == 0 && !ApuMapApuAddress(old) && AcpHalReleaseShapeContexts() == FAIL);
    CHECK(AcpHalAllocateShapeContexts(&second) == 0 && AcpHalReleaseShapeContexts() == 0);
    puts("AcpHal named imports, TLS, heap, contexts, messages and XMA passed");
    return 0;
}
#endif
