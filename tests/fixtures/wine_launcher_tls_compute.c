/* One ordinary PE image exercises the real imported providers and Metal path. */
#define main provider_main
#include "wine_process_providers.c"
#undef main
#undef SLOT
#define mainCRTStartup compute_main
#include "windows_d3d12x_compute.c"
#undef mainCRTStartup

static int check_proxy_adapter(void) {
    static const GUID dxgi_ids[] = {
        {0x54ec77fa,0x1377,0x44e6,{0x8c,0x32,0x88,0xfd,0x5f,0x44,0xc8,0x4c}},
        {0x77db970f,0x6276,0x48ba,{0xba,0x28,0x07,0x01,0x43,0xb4,0x39,0x2c}},
        {0x05008617,0xfbfd,0x4051,{0xa7,0x90,0x14,0x48,0x84,0xb4,0xf6,0xa9}},
    };
    const GUID unknown = {0,0,0,{0xc0,0,0,0,0,0,0,0x46}};
    typedef HRESULT (WINAPI *Query)(void *, const GUID *, void **);
    typedef HRESULT (WINAPI *GetAdapter)(void *, void **);
    typedef ULONG (WINAPI *Release)(void *);
    typedef struct { WCHAR name[128]; unsigned vendor, device, subsystem, revision; size_t video, system, shared; unsigned luid[2]; } Description;
    typedef HRESULT (WINAPI *Describe)(void *, Description *);
    unsigned version = 0x00020004;
    void *device = NULL, *dxgi = NULL, *adapter = NULL, *identity = NULL;
    if (D3D12XboxCreateDevice(NULL, &version, &IID_ID3D12Device, &device) || !device) return 0;
    Query query = SLOT(device, 0, Query);
    for (unsigned i = 0; i < 3; ++i) {
        void *candidate = NULL;
        if (query(device, &dxgi_ids[i], &candidate) || !candidate) return 0;
        if (i == 0) dxgi = candidate;
        else { if (candidate != dxgi) return 0; SLOT(candidate, 0x10, Release)(candidate); }
    }
    if (SLOT(dxgi, 0, Query)(dxgi, &unknown, &identity) || identity != device) return 0;
    SLOT(identity, 0x10, Release)(identity);
    if (SLOT(dxgi, 0x38, GetAdapter)(dxgi, &adapter) || !adapter) return 0;
    SLOT(device, 0x10, Release)(device);
    if (SLOT(dxgi, 0, Query)(dxgi, &IID_ID3D12Device, &identity) || identity != device) return 0;
    SLOT(identity, 0x10, Release)(identity);
    SLOT(dxgi, 0x10, Release)(dxgi);
    Description desc;
    if (SLOT(adapter, 0x40, Describe)(adapter, &desc)) return 0;
    char expected[16];
    DWORD n = GetEnvironmentVariableA("VICTORY_TEST_EXPECT_CONSOLE", expected, sizeof(expected));
    int series_s = n && n < sizeof(expected) && !strcmp(expected, "series-s");
    if (desc.vendor != 0x1002 || desc.video != (series_s ? 0 : ((size_t)10 << 30)) ||
        desc.shared != (series_s ? ((size_t)7776 << 20) : ((size_t)7 << 29))) return 0;
    if (SLOT(adapter, 0, Query)(adapter, &unknown, &identity) || identity != adapter) return 0;
    SLOT(identity, 0x10, Release)(identity);
    SLOT(adapter, 0x10, Release)(adapter);
    say("Proxy DXGI model and COM lifetime passed\r\n");
    return 1;
}

int main(int argc, char **argv) {
    static const char *const d3d_exports[] = {"D3D12XboxCreateDevice"};
    char addr[128], token[128], present[8];
    if (argc == 2 && !strcmp(argv[1], "--wait-for-host-exit")) {
        WCHAR path[32768], tmp[32768];
        DWORD len = GetEnvironmentVariableW(L"VICTORY_TEST_GUEST_READY", path, 32768);
        DWORD addr_len = GetEnvironmentVariableA("VICTORY_WINE_METAL_ADDR", addr, sizeof(addr));
        DWORD token_len = GetEnvironmentVariableA("VICTORY_WINE_METAL_TOKEN", token, sizeof(token));
        if (!len || len >= 32764 || !addr_len || addr_len >= sizeof(addr) || token_len != 32) return 27;
        wcscpy(tmp, path);
        wcscat(tmp, L".tmp");
        HANDLE file = CreateFileW(tmp, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
        if (file == INVALID_HANDLE_VALUE) return 28;
        DWORD written;
        int ok = WriteFile(file, addr, addr_len, &written, NULL) && written == addr_len &&
            WriteFile(file, "\n", 1, &written, NULL) && written == 1 &&
            WriteFile(file, token, token_len, &written, NULL) && written == token_len;
        CloseHandle(file);
        if (!ok || !MoveFileExW(tmp, path, MOVEFILE_REPLACE_EXISTING)) return 28;
        Sleep(120000);
        return 29;
    }
    if (argc != 3 || strcmp(argv[1], "argument with spaces") || strcmp(argv[2], "--guest-option")) {
        say("Guest arguments mismatch\r\n"); return 24;
    }
    if (!module_ok(L"d3d12_xs.dll", L"VICTORY_EXPECT_D3D12X_DLL", d3d_exports, 1)) {
        say("D3D12X DLL path or exports mismatch\r\n"); return 25;
    }
    if (!GetEnvironmentVariableA("VICTORY_WINE_METAL_ADDR", addr, sizeof(addr)) ||
        strncmp(addr, "127.0.0.1:", 10) || !strcmp(addr + 10, "0") ||
        GetEnvironmentVariableA("VICTORY_WINE_METAL_TOKEN", token, sizeof(token)) != 32 ||
        !GetEnvironmentVariableA("VICTORY_WINE_METAL_PRESENT", present, sizeof(present)) ||
        strcmp(present, "0")) {
        say("Metal connection environment mismatch\r\n"); return 26;
    }
    char inherited[64];
    if (!GetEnvironmentVariableA("VICTORY_TEST_LAUNCHER_INHERITED", inherited, sizeof(inherited)) ||
        strcmp(inherited, "preserved")) {
        say("Unrelated environment was not preserved\r\n"); return 30;
    }
    say("D3D12X DLL identity and launcher environment passed\r\n");
    int result = provider_main();
    if (result) return result;
    if (!check_proxy_adapter()) { say("Proxy DXGI check failed\r\n"); return 31; }
    return (int)compute_main(NULL);
}
