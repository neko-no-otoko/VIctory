/* Native PE imports, compiler TLS and the XApu command ABI. */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>
#ifdef UNSUPPORTED_IMPORT
__declspec(dllimport) void WINAPI VIctoryUnsupportedImport(void);
int main(void) { VIctoryUnsupportedImport(); return 0; }
#else
#define INVALID ((HRESULT)0x80070057u)
#define NOTIMPL ((HRESULT)0x80004001u)
#define FULL ((HRESULT)0x8ac81000u)
#define PENDING ((HRESULT)0x8ac81001u)
#define BAD_COMMAND ((HRESULT)0x8ac81002u)
#define BAD_STREAM ((HRESULT)0x8ac81003u)
#define ACTIVE ((HRESULT)0x8ac81005u)
typedef struct { uint32_t process, streams, commands, memory, frames, options; } Input;
typedef struct { void *memory; HANDLE event; } Output;
#pragma pack(push, 1)
typedef struct { uint32_t kind, stream, sequence, options; uint64_t context; } Id;
typedef struct { Id id; uint32_t channels, init_length; const void *init; float pitch; uint32_t processing_length; void *processing; } Activate;
typedef struct { Id id; float ramp, pitch; uint32_t first, frames, input_length; const uint8_t *input; uint32_t output_length; uint8_t *output; } Process;
typedef struct { Id id; uint32_t state; uint64_t state_data, usec; HRESULT error; uint32_t frames, output_length; uint8_t *output; float pitch; uint32_t input_length; const uint8_t *input; } Result;
#pragma pack(pop)
__declspec(dllimport) HRESULT WINAPI XApuConnect(const Input *, Output *, void **);
__declspec(dllimport) HRESULT WINAPI XApuDisconnect(void *);
__declspec(dllimport) HRESULT WINAPI XApuEnqueueCommand(void *, const Id *, uint32_t *);
__declspec(dllimport) HRESULT WINAPI XApuDequeueResult(void *, Result *);
__declspec(dllimport) uint32_t WINAPI XApuGetResultCount(void *, uint32_t *);
#define CHECK(x) do { if (!(x)) { printf("XApu check failed line %d (error %lu)\n", __LINE__, GetLastError()); return 1; } } while (0)
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
    Input in = {1, 1, 2, 128, 0, 0}; Output out; void *c = NULL; Result r;
    Id id = {4, 0, (uint32_t)value, 0, (uint64_t)value};
    if (tls_value != 17) return 1;
    tls_value = value;
    if (XApuConnect(&in, &out, &c) || !c || !out.memory || out.event) return 2;
    memset(out.memory, value, 128);
    if (XApuEnqueueCommand(c, &id, NULL)) return 3;
    InterlockedIncrement(&ready);
    if (WaitForSingleObject(gate, 10000) != WAIT_OBJECT_0) return 4;
    if (XApuGetResultCount(c, NULL) != 1 || XApuDequeueResult(c, &r) || r.id.sequence != (uint32_t)value || r.id.context != (uint64_t)value || r.error) return 5;
    for (int i = 0; i < 128; ++i) if (((unsigned char *)out.memory)[i] != (unsigned char)value) return 6;
    if (XApuDisconnect(c) || tls_value != value) return 7;
    return 0;
}
static const char *const exports[] = {"XApuConnect", "XApuDisconnect", "XApuEnqueueCommand", "XApuDequeueResult", "XApuGetResultCount"};
static int named_imports(void) {
    unsigned char *base = (unsigned char *)GetModuleHandleW(NULL);
    IMAGE_NT_HEADERS64 *nt = (IMAGE_NT_HEADERS64 *)(base + ((IMAGE_DOS_HEADER *)base)->e_lfanew);
    IMAGE_IMPORT_DESCRIPTOR *d = (IMAGE_IMPORT_DESCRIPTOR *)(base + nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].VirtualAddress);
    unsigned seen = 0;
    for (; d->Name; ++d) {
        if (_stricmp((char *)(base + d->Name), "xapu.dll")) continue;
        if (!d->OriginalFirstThunk) return 0;
        IMAGE_THUNK_DATA64 *t = (IMAGE_THUNK_DATA64 *)(base + d->OriginalFirstThunk);
        for (; t->u1.AddressOfData; ++t) {
            if (IMAGE_SNAP_BY_ORDINAL64(t->u1.Ordinal)) return 0;
            const char *name = (const char *)((IMAGE_IMPORT_BY_NAME *)(base + t->u1.AddressOfData))->Name;
            unsigned i; for (i = 0; i < 5; ++i) if (!strcmp(name, exports[i])) break;
            if (i == 5 || (seen & (1u << i))) return 0;
            seen |= 1u << i;
        }
    }
    return seen == 31;
}
static int module_path(HMODULE module) {
    WCHAR actual[32768], expected[32768];
    DWORD n = module ? GetModuleFileNameW(module, actual, 32768) : 0;
    DWORD e = GetEnvironmentVariableW(L"VICTORY_EXPECT_XAPU_DLL", expected, 32768);
    return n && n < 32768 && e && e < 32768 && !_wcsicmp(actual, expected);
}
int main(void) {
    HMODULE module = GetModuleHandleW(L"xapu.dll");
    CHECK(module_path(module) && named_imports());
    for (unsigned i = 0; i < 5; ++i) CHECK(GetProcAddress(module, exports[i]));
    CHECK(!GetProcAddress(module, "VIctoryUnsupportedImport") && !GetProcAddress(module, "XApuUnknown"));
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

    Input in = {1, 2, 2, 16384, 0, 8}; Output out, other; void *c = NULL, *c2 = NULL; Result r; uint32_t total;
    CHECK(XApuConnect(NULL, &out, &c) == INVALID && XApuConnect(&in, NULL, &c) == INVALID && XApuConnect(&in, &out, NULL) == INVALID);
    Input bad = in; bad.streams = 0; CHECK(XApuConnect(&bad, &out, &c) == INVALID);
    bad = in; bad.streams = 501; CHECK(XApuConnect(&bad, &out, &c) == INVALID);
    bad = in; bad.commands = 0; CHECK(XApuConnect(&bad, &out, &c) == INVALID);
    bad = in; bad.commands = 17; CHECK(XApuConnect(&bad, &out, &c) == INVALID);
    bad = in; bad.memory = 0; CHECK(XApuConnect(&bad, &out, &c) == INVALID);
    bad = in; bad.frames = 1; CHECK(XApuConnect(&bad, &out, &c) == INVALID);
    bad = in; bad.process = 2; CHECK(XApuConnect(&bad, &out, &c) == NOTIMPL);
    CHECK(XApuDisconnect(NULL) == INVALID && XApuDequeueResult(NULL, &r) == INVALID && XApuGetResultCount(NULL, NULL) == 0);
    CHECK(XApuConnect(&in, &out, &c) == 0 && c && out.memory && !((uintptr_t)out.memory & 15) && out.event);
    CHECK(XApuConnect(&in, &other, &c2) == 0 && c2 && c2 != c && other.memory && other.memory != out.memory && other.event != out.event);
    for (unsigned i = 0; i < in.memory; ++i) CHECK(!((uint8_t *)out.memory)[i]);
    CHECK(WaitForSingleObject(out.event, 0) == WAIT_TIMEOUT && XApuDequeueResult(c, &r) == PENDING);
    CHECK(XApuDequeueResult(c, NULL) == INVALID && XApuEnqueueCommand(c, NULL, NULL) == INVALID);
    Id id = {99, 0, 1, 0, 0};
    CHECK(XApuEnqueueCommand(NULL, &id, NULL) == INVALID && XApuEnqueueCommand(c, &id, NULL) == BAD_COMMAND);
    id.stream = 2; CHECK(XApuEnqueueCommand(c, &id, NULL) == BAD_STREAM);
    Activate a = {{1, 0, 10, 1, 0x123456789abcdef0ULL}, 1, 0, NULL, 1.0f, 0, NULL};
    CHECK(XApuEnqueueCommand(c, &a.id, &total) == 0 && total == 1);
    CHECK(WaitForSingleObject(out.event, 0) == WAIT_OBJECT_0 && WaitForSingleObject(out.event, 0) == WAIT_TIMEOUT);
    a.id.sequence = 11; a.id.options = 0;
    CHECK(XApuEnqueueCommand(c, &a.id, &total) == 0 && total == 2);
    CHECK(XApuEnqueueCommand(c, &a.id, NULL) == FULL);
    a.id.stream = 1; a.id.sequence = 12; a.channels = 2;
    CHECK(XApuEnqueueCommand(c, &a.id, &total) == 0 && total == 3);
    CHECK(XApuGetResultCount(c, &total) == 3 && total == 0 && XApuGetResultCount(c2, NULL) == 0);
    CHECK(XApuDequeueResult(c, &r) == 0 && r.id.sequence == 10 && r.id.context == 0x123456789abcdef0ULL && r.error == 0);
    CHECK(XApuDequeueResult(c, &r) == 0 && r.id.sequence == 11 && r.error == ACTIVE);
    CHECK(XApuDequeueResult(c, &r) == 0 && r.id.sequence == 12 && r.id.stream == 1 && !r.error);
    CHECK(XApuGetResultCount(c, &total) == 0 && total == 3 && XApuDequeueResult(c, &r) == PENDING);
    /* RFC 6716 20 ms Opus silence packet, decoded to mono float PCM. */
    uint8_t *packet = (uint8_t *)out.memory; float *pcm = (float *)(packet + 16);
    packet[0] = 0xf8; packet[1] = 0xff; packet[2] = 0xfe;
    for (unsigned i = 0; i < 960; ++i) pcm[i] = 42.0f;
    Process p = {{3, 0, 13, 0, 0x55}, 0, 1, 0, 960, 3, packet, 960 * 4, (uint8_t *)pcm};
    CHECK(XApuEnqueueCommand(c, &p.id, NULL) == 0 && XApuDequeueResult(c, &r) == 0);
    CHECK(r.id.sequence == 13 && r.id.context == 0x55 && r.error == 0 && r.frames == 960 && r.output_length == 3840 && r.output == (uint8_t *)pcm && r.input == packet && r.input_length == 3);
    for (unsigned i = 0; i < 960; ++i) CHECK(pcm[i] > -0.00001f && pcm[i] < 0.00001f);
    p.id.sequence = 14; p.input = NULL; CHECK(XApuEnqueueCommand(c, &p.id, NULL) == 0 && XApuDequeueResult(c, &r) == 0 && r.error == INVALID);
    id = (Id){2, 0, 15, 0, 0}; CHECK(XApuEnqueueCommand(c, &id, NULL) == 0);
    p.id.sequence = 16; p.input = packet; CHECK(XApuEnqueueCommand(c, &p.id, NULL) == 0);
    CHECK(XApuDequeueResult(c, &r) == 0 && r.id.sequence == 15 && !r.error);
    CHECK(XApuDequeueResult(c, &r) == 0 && r.id.sequence == 16 && r.error == BAD_STREAM);
    /* The other connection's stream zero is still inactive. */
    CHECK(XApuEnqueueCommand(c2, &p.id, NULL) == 0 && XApuDequeueResult(c2, &r) == 0 && r.error == BAD_STREAM);
    id = (Id){5, 1, 17, 0, 0}; CHECK(XApuEnqueueCommand(c, &id, NULL) == 0 && XApuDequeueResult(c, &r) == 0 && !r.error);
    id = (Id){2, 1, 18, 0, 0}; CHECK(XApuEnqueueCommand(c, &id, NULL) == 0 && XApuDequeueResult(c, &r) == 0 && !r.error);
    HANDLE event = out.event, event2 = other.event;
    CHECK(XApuDisconnect(c) == 0 && XApuDisconnect(c2) == 0);
    CHECK(WaitForSingleObject(event, 0) == WAIT_FAILED && WaitForSingleObject(event2, 0) == WAIT_FAILED);
    puts("XApu named imports, TLS, Opus and queues passed");
    return 0;
}
#endif
