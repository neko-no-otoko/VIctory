/* No static AcpHal import: release the only explicit loader reference. */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <wchar.h>
typedef struct Acp Acp;
typedef struct {
    HRESULT (WINAPI *connect)(Acp *, uint32_t, uint32_t);
    HRESULT (WINAPI *disconnect)(Acp *);
    HRESULT (WINAPI *submit)(Acp *, uint32_t, uint64_t, uint32_t, const void *, uint32_t);
    _Bool (WINAPI *pop)(Acp *, void *);
    uint32_t (WINAPI *count)(Acp *);
    void (WINAPI *release)(Acp *);
} Methods;
struct Acp { const Methods *vtable; };
typedef HRESULT (WINAPI *CreateHeap)(uint32_t, uint32_t);
typedef HRESULT (WINAPI *Alloc)(void **, uint32_t *, uint32_t, uint32_t, uint32_t);
typedef void (WINAPI *Free)(void *);
typedef void *(WINAPI *Map)(uint32_t);
typedef HRESULT (WINAPI *Create)(Acp **);
int main(void) {
    HMODULE module = LoadLibraryW(L"acphal.dll");
    WCHAR actual[32768], expected[32768];
    DWORD n = module ? GetModuleFileNameW(module, actual, 32768) : 0;
    DWORD e = GetEnvironmentVariableW(L"VICTORY_EXPECT_ACPHAL_DLL", expected, 32768);
    if (!n || n >= 32768 || !e || e >= 32768 || _wcsicmp(actual, expected)) return 1;
    CreateHeap heap = (CreateHeap)GetProcAddress(module, "ApuCreateHeap");
    Alloc alloc = (Alloc)GetProcAddress(module, "ApuAlloc");
    Free free_block = (Free)GetProcAddress(module, "ApuFree");
    Map map = (Map)GetProcAddress(module, "ApuMapApuAddress");
    Create create = (Create)GetProcAddress(module, "AcpHalCreate");
    void *block = NULL; uint32_t apu = 0; Acp *acp = NULL;
    if (!heap || !alloc || !free_block || !map || !create || heap(4096, 0) ||
        alloc(&block, &apu, 256, 256, 0) || !block || !apu || create(&acp) || !acp) return 2;
    const Methods *m = acp->vtable;
    if (!FreeLibrary(module) || GetModuleHandleW(L"acphal.dll") != module) return 3;
    *(uint32_t *)block = 0x12345678;
    if (map(apu) != block || *(uint32_t *)map(apu) != 0x12345678 ||
        m->connect(acp, 16, 2) || m->submit(acp, 63, 0, 0, NULL, apu) || *(uint32_t *)block != 1 ||
        m->disconnect(acp)) return 4;
    m->release(acp); free_block(block);
    if (map(apu)) return 5;
    puts("AcpHal dynamic provider remains pinned");
    return 0;
}
