/* A normal Wine executable importing the independently built provider DLLs. */
#include <windows.h>
#include <stdint.h>
#include <string.h>
#include <wchar.h>

struct Desc { unsigned dim; uint64_t align, width; unsigned height; unsigned short depth, mips; unsigned format, samples, quality, layout, flags; };
__declspec(dllimport) HRESULT WINAPI XGCreateTextureComputer(const struct Desc *, void **);
__declspec(dllimport) HRESULT WINAPI XGSuggestSwizzleMode(const struct Desc *, unsigned, unsigned *);
__declspec(dllimport) HRESULT WINAPI QueryApiImpl(const GUID *, const GUID *, void **);

static const GUID system_provider = {0xe349bd1a,0xfc20,0x4e40,{0xb9,0x9c,0x41,0x78,0xcc,0x6b,0x40,0x9f}};
static const GUID system_iid = {0x6fd71f09,0x7513,0x49f0,{0x89,0xbc,0xbf,0xaf,0x5d,0xf6,0xf8,0x52}};
static const GUID user_provider = {0x01acd177,0x91f9,0x4763,{0xa3,0x8e,0xcc,0xbb,0x55,0xce,0x32,0xe0}};
static const GUID user_iid = {0xeb9bf948,0x18dc,0x4d82,{0xbb,0xcc,0x40,0xe0,0xa8,0x09,0xc4,0xc0}};
static const GUID tag_iid = {0xcef4fac0,0x7676,0x4a94,{0xa1,0x19,0x4c,0x43,0xf9,0xeb,0x5b,0x74}};
static const GUID unknown = {0};
typedef ULONG (WINAPI *Ref)(void *);
typedef HRESULT (WINAPI *GetString)(void *, size_t, char *, size_t *);
typedef HRESULT (WINAPI *Find)(void *, uint64_t, void **);
typedef HRESULT (WINAPI *GetId)(void *, void *, uint64_t *);
typedef HRESULT (WINAPI *Duplicate)(void *, void *, void **);
typedef void (WINAPI *Close)(void *, void *);
typedef int (WINAPI *Compare)(void *, void *, void *);
typedef HRESULT (WINAPI *GetTag)(void *, void *, unsigned, size_t, char *, size_t *);
#define SLOT(obj, type, n) ((type)(*(void ***)(obj))[n])

static _Thread_local int tls_value = 17;
static volatile LONG attaches, detaches, bad_tls, ready, process_attach;
static HANDLE gate;
static void NTAPI tls_callback(PVOID image, DWORD reason, PVOID reserved) {
    (void)image; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        if (tls_value != 17) InterlockedIncrement(&bad_tls);
        if (reason == DLL_PROCESS_ATTACH) InterlockedIncrement(&process_attach);
        else InterlockedIncrement(&attaches);
    } else if (reason == DLL_THREAD_DETACH) InterlockedIncrement(&detaches);
}
__attribute__((used, section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK provider_tls_callback = tls_callback;

static void say(const char *s) {
    DWORD written;
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), s, (DWORD)strlen(s), &written, NULL);
}
static int module_ok(const WCHAR *name, const WCHAR *env, const char *const *exports, size_t count) {
    WCHAR actual[32768], expected[32768];
    HMODULE module = GetModuleHandleW(name);
    DWORD n = module ? GetModuleFileNameW(module, actual, 32768) : 0;
    DWORD e = GetEnvironmentVariableW(env, expected, 32768);
    if (!n || n >= 32768 || !e || e >= 32768 || _wcsicmp(actual, expected)) return 0;
    for (size_t i = 0; i < count; ++i) if (!GetProcAddress(module, exports[i])) return 0;
    return !GetProcAddress(module, "VIctoryUnsupportedImport") &&
        !GetProcAddress(module, !wcscmp(name, L"xg_xs.dll") ? "XGUnknown" : "XUserGetGamertag");
}
static int check_modules(void) {
    static const char *const xg[] = {"XGCreateTextureComputer", "XGSuggestSwizzleMode"};
    static const char *const runtime[] = {"XGameRuntimeInitialize", "XGameRuntimeUninitialize", "InitializeApiImpl", "InitializeApiImplEx", "InitializeApiImplEx2", "UninitializeApiImpl", "QueryApiImpl", "XErrorReport"};
    if (!module_ok(L"xg_xs.dll", L"VICTORY_EXPECT_XG_DLL", xg, sizeof(xg)/sizeof(*xg)) ||
        !module_ok(L"xgameruntime.dll", L"VICTORY_EXPECT_RUNTIME_DLL", runtime, sizeof(runtime)/sizeof(*runtime))) {
        say("Provider DLL path or exports mismatch\r\n"); return 1;
    }
    HMODULE module = GetModuleHandleW(L"xgameruntime.dll");
    typedef HRESULT (WINAPI *Init)(void);
    typedef HRESULT (WINAPI *Init2)(uint64_t, uint64_t);
    typedef HRESULT (WINAPI *Init3)(uint64_t, uint64_t, uint64_t);
    typedef HRESULT (WINAPI *Init4)(uint64_t, uint64_t, uint64_t, const void *);
    if (((Init)GetProcAddress(module, runtime[0]))() ||
        ((Init2)GetProcAddress(module, runtime[2]))(1, 2) ||
        ((Init3)GetProcAddress(module, runtime[3]))(1, 2, 3) ||
        ((Init4)GetProcAddress(module, runtime[4]))(1, 2, 3, NULL)) return 2;
    say("Provider DLL identities and exports passed\r\n"); return 0;
}
static int check_xg(void) {
    struct Desc d = {0}; void *computer = NULL; unsigned mode = 0;
    static unsigned char layout[0x1b40] __attribute__((aligned(16)));
    typedef HRESULT (WINAPI *Layout)(void *, void *);
    if ((uint32_t)XGCreateTextureComputer(NULL, &computer) != 0x80070057u) return 1;
    d.dim = 3; d.width = 256; d.height = 256; d.depth = 1; d.format = 98; d.samples = 1; d.layout = 0x10a;
    if (XGSuggestSwizzleMode(&d, 0, &mode) || mode != 0x105) return 2;
    if (XGCreateTextureComputer(&d, &computer) || !computer) return 3;
    if (SLOT(computer, Ref, 0)(computer) != 2) return 4;
    if (SLOT(computer, Layout, 2)(computer, layout) || *(uint64_t *)layout != 131072 ||
        *(uint64_t *)(layout+8) != 65536 || *(unsigned *)(layout+16) != 9 || *(unsigned *)(layout+20) != 1) return 5;
    if (SLOT(computer, Ref, 1)(computer) != 1 || SLOT(computer, Ref, 1)(computer) != 0) return 6;
    say("XG texture computer lifetime passed\r\n"); return 0;
}
static int check_system(void) {
    void *obj = NULL; char buffer[64]; size_t used = 99;
    if (QueryApiImpl(&system_provider, &system_iid, &obj) || !obj) return 1;
    GetString get = SLOT(obj, GetString, 4);
    if ((uint32_t)get(obj, 6, buffer, &used) != 0x8007007au || used != 99) return 2;
    if (get(obj, sizeof(buffer), buffer, &used) || used != 7 || strcmp(buffer, "RETAIL")) return 3;
    if (SLOT(obj, Ref, 2)(obj) != 1) return 4;
    return 0;
}
static int check_user(void) {
    void *obj = NULL, *tag = NULL, *handle = NULL, *copy = NULL, *missing = NULL;
    uint64_t local = 0, id = 0; char buffer[64]; size_t used = 0;
    if (QueryApiImpl(&user_provider, &user_iid, &obj) || !obj ||
        QueryApiImpl(&user_provider, &tag_iid, &tag) || !tag) return 1;
    if (SLOT(obj, Find, 10)(obj, 1, &handle) || !handle ||
        SLOT(obj, GetId, 9)(obj, handle, &local) || local != 1 ||
        SLOT(obj, GetId, 11)(obj, handle, &id) || id != UINT64_C(0x0009000000000001)) return 2;
    if (SLOT(obj, Duplicate, 3)(obj, handle, &copy) || !copy || SLOT(obj, Compare, 5)(obj, handle, copy)) return 3;
    if (SLOT(tag, GetTag, 3)(tag, copy, 0, sizeof(buffer), buffer, &used) || used != 7 || strcmp(buffer, "Player")) return 4;
    if ((uint32_t)SLOT(tag, GetTag, 3)(tag, copy, 0, 2, buffer, &used) != 0x8007007au) return 5;
    if ((uint32_t)SLOT(obj, Find, 12)(obj, 0, &missing) != 0x89245104u) return 6;
    SLOT(obj, Close, 4)(obj, copy); SLOT(obj, Close, 4)(obj, handle);
    if (SLOT(tag, Ref, 2)(tag) != 1 || SLOT(obj, Ref, 2)(obj) != 1) return 7;
    if ((uint32_t)QueryApiImpl(&user_provider, &unknown, &missing) != 0x80070032u ||
        (uint32_t)QueryApiImpl(NULL, &user_iid, &missing) != 0x80070032u ||
        (uint32_t)QueryApiImpl(&user_provider, &user_iid, NULL) != 0x80070032u) return 8;
    say("XSystem and XUser providers passed\r\n"); return 0;
}
static DWORD WINAPI worker(void *arg) {
    int value = (int)(INT_PTR)arg;
    if (tls_value != 17 || check_system()) return 1;
    tls_value = value; InterlockedIncrement(&ready);
    if (WaitForSingleObject(gate, 10000) != WAIT_OBJECT_0) return 2;
    return tls_value == value ? 0 : 3;
}
static int check_tls(void) {
    HANDLE threads[2]; DWORD codes[2];
    if (tls_value != 17 || bad_tls || process_attach != 1) return 1;
    tls_value = 99; gate = CreateEventW(NULL, TRUE, FALSE, NULL);
    if (!gate) return 2;
    threads[0] = CreateThread(NULL, 0, worker, (void *)(INT_PTR)111, 0, NULL);
    threads[1] = CreateThread(NULL, 0, worker, (void *)(INT_PTR)222, 0, NULL);
    if (!threads[0] || !threads[1]) return 3;
    ULONGLONG deadline = GetTickCount64() + 10000;
    while (InterlockedCompareExchange(&ready, 0, 0) != 2 && GetTickCount64() < deadline) Sleep(1);
    if (ready != 2 || tls_value != 99 || !SetEvent(gate)) return 4;
    if (WaitForMultipleObjects(2, threads, TRUE, 10000) != WAIT_OBJECT_0) return 5;
    if (!GetExitCodeThread(threads[0], &codes[0]) || !GetExitCodeThread(threads[1], &codes[1])) return 6;
    CloseHandle(threads[0]); CloseHandle(threads[1]); CloseHandle(gate);
    if (codes[0] || codes[1] || attaches != 2 || detaches != 2 || bad_tls || tls_value != 99) return 7;
    say("Provider threads and static TLS callbacks passed\r\n"); return 0;
}
int main(void) {
    if (check_modules()) return 20;
    if (check_xg()) return 21;
    if (check_system() || check_user()) return 22;
    if (check_tls()) return 23;
    HMODULE module = GetModuleHandleW(L"xgameruntime.dll");
    typedef void (WINAPI *Uninit)(void);
    typedef void (WINAPI *Report)(HRESULT, const char *);
    ((Report)GetProcAddress(module, "XErrorReport"))(0, NULL);
    ((Uninit)GetProcAddress(module, "UninitializeApiImpl"))();
    ((Uninit)GetProcAddress(module, "XGameRuntimeUninitialize"))();
    return 0;
}
