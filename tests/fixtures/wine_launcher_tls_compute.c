/* One ordinary PE image exercises the real imported providers and Metal path. */
#define main provider_main
#include "wine_process_providers.c"
#undef main
#undef SLOT
#define mainCRTStartup compute_main
#include "windows_d3d12x_compute.c"
#undef mainCRTStartup

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
    return result ? result : (int)compute_main(NULL);
}
