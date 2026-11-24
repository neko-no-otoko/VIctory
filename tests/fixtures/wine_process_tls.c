/* Synthetic, normally launched Wine process: compiler static TLS and the
 * independently built D3D12.X import DLL, not the in-process PE loader. */
#include <windows.h>
#include <wchar.h>

__declspec(dllimport) HRESULT WINAPI D3D12XboxCreateDevice(const void *, const unsigned *, const IID *, void **);
__declspec(dllimport) int dep_get(void);
__declspec(dllimport) void dep_set(int);
__declspec(dllimport) int dep_callbacks_ok(void);

static _Thread_local int tls_value = 17;
static volatile LONG attaches, detaches, bad_tls, ready, process_attach;
static HANDLE gate;
static unsigned locale_callbacks, locale_bad;

static BOOL CALLBACK locale_callback(LPWSTR locale) {
    if (!locale || tls_value != 99 || dep_get() != 199) ++locale_bad;
    ++locale_callbacks;
    return TRUE;
}

static void NTAPI tls_callback(PVOID image, DWORD reason, PVOID reserved) {
    (void)image; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        if (tls_value != 17) InterlockedIncrement(&bad_tls);
        if (reason == DLL_PROCESS_ATTACH) InterlockedIncrement(&process_attach);
        else InterlockedIncrement(&attaches);
    } else if (reason == DLL_THREAD_DETACH) {
        InterlockedIncrement(&detaches);
    }
}
__attribute__((used, section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK wine_tls_callback = tls_callback;

static void say(const char *s) {
    DWORD written, len = 0;
    while (s[len]) ++len;
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), s, len, &written, NULL);
}

static DWORD WINAPI worker(void *arg) {
    int value = (int)(INT_PTR)arg;
    if (tls_value != 17 || dep_get() != 31) return 1;
    tls_value = value;
    dep_set(value + 1000);
    InterlockedIncrement(&ready);
    if (WaitForSingleObject(gate, 10000) != WAIT_OBJECT_0) return 2;
    return tls_value == value && dep_get() == value + 1000 ? 0 : 3;
}

static int check_tls(void) {
    HANDLE threads[2]; DWORD codes[2];
    if (tls_value != 17 || dep_get() != 31 || bad_tls || process_attach != 1) return 1;
    tls_value = 99;
    dep_set(199);
    if (!EnumSystemLocalesW(locale_callback, LCID_INSTALLED) || !locale_callbacks || locale_bad ||
        tls_value != 99 || dep_get() != 199) return 8;
    say("Wine locale callback preserved both TLS images\r\n");
    gate = CreateEventW(NULL, TRUE, FALSE, NULL);
    if (!gate) return 2;
    threads[0] = CreateThread(NULL, 0, worker, (void *)(INT_PTR)111, 0, NULL);
    threads[1] = CreateThread(NULL, 0, worker, (void *)(INT_PTR)222, 0, NULL);
    if (!threads[0] || !threads[1]) return 3;
    ULONGLONG deadline = GetTickCount64() + 10000;
    while (InterlockedCompareExchange(&ready, 0, 0) != 2 && GetTickCount64() < deadline) Sleep(1);
    if (ready != 2 || tls_value != 99 || dep_get() != 199 || !SetEvent(gate)) return 4;
    if (WaitForMultipleObjects(2, threads, TRUE, 10000) != WAIT_OBJECT_0) return 5;
    if (!GetExitCodeThread(threads[0], &codes[0]) || !GetExitCodeThread(threads[1], &codes[1])) return 6;
    CloseHandle(threads[0]); CloseHandle(threads[1]); CloseHandle(gate);
    if (codes[0] || codes[1] || attaches != 2 || detaches != 2 || bad_tls || tls_value != 99 ||
        dep_get() != 199 || !dep_callbacks_ok()) return 7;
    say("TLS compiler state and thread callbacks passed\r\n");
    say("Separate DLL TLS state and thread callbacks passed\r\n");
    return 0;
}

static int check_dll(void) {
    static const char *exports[] = {
        "D3D12ConfigureVirtualMemoryX", "D3D12XboxCreateDevice",
        "D3D12SerializeRootSignature", "D3D12SerializeVersionedRootSignature",
        "D3D12CreateRootSignatureDeserializer", "IID_ID3D12Device"
    };
    static const GUID iid = {0x189819f1, 0x1db6, 0x4b57, {0xbe,0x54,0x18,0x21,0x33,0x9b,0x85,0xf7}};
    WCHAR actual[32768], expected[32768];
    HMODULE dll = GetModuleHandleW(L"d3d12_xs.dll");
    DWORD n = dll ? GetModuleFileNameW(dll, actual, 32768) : 0;
    DWORD e = GetEnvironmentVariableW(L"VICTORY_EXPECT_D3D12X_DLL", expected, 32768);
    if (!n || n >= 32768 || !e || e >= 32768 || _wcsicmp(actual, expected)) {
        say("D3D12X DLL path mismatch\r\n"); return 1;
    }
    for (unsigned i = 0; i < sizeof(exports)/sizeof(exports[0]); ++i)
        if (!GetProcAddress(dll, exports[i])) return 2;
    const GUID *exported = (const GUID *)GetProcAddress(dll, "IID_ID3D12Device");
    if (!IsEqualGUID(exported, &iid)) return 3;
    if (GetProcAddress(dll, "VIctoryUnsupportedImport")) return 4;
    if (D3D12XboxCreateDevice(NULL, NULL, NULL, NULL) != E_INVALIDARG) return 5;
    typedef HRESULT (WINAPI *Configure)(void *);
    typedef HRESULT (WINAPI *Serialize)(const void *, unsigned, void **, void **);
    typedef HRESULT (WINAPI *SerializeVersioned)(const void *, void **, void **);
    typedef HRESULT (WINAPI *Deserialize)(const void *, SIZE_T, const GUID *, void **);
    struct VirtualMemory { unsigned flags, pages, preserved; ULONGLONG lowest, highest; } vm = {0};
    Configure configure = (Configure)GetProcAddress(dll, exports[0]);
    Serialize serialize = (Serialize)GetProcAddress(dll, exports[2]);
    SerializeVersioned versioned = (SerializeVersioned)GetProcAddress(dll, exports[3]);
    Deserialize deserialize = (Deserialize)GetProcAddress(dll, exports[4]);
    if (configure(NULL) != E_INVALIDARG || configure(&vm) != S_OK ||
        !vm.lowest || vm.highest <= vm.lowest) return 6;
    if (serialize(NULL, 0, NULL, NULL) != E_INVALIDARG ||
        versioned(NULL, NULL, NULL) != E_INVALIDARG ||
        deserialize(NULL, 0, NULL, NULL) != E_INVALIDARG) return 7;
    say("D3D12X DLL path, exports and error passed\r\n");
    return 0;
}

#ifndef WINE_PROCESS_TLS_NO_MAIN
int main(void) {
    if (check_dll()) return 20;
    if (check_tls()) return 21;
    return 0;
}
#endif
