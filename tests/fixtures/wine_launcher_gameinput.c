/* A single normal Wine PE exercises both GameInput import forms and Metal. */
#define Ref InputRef
#define Find InputFind
#define main gameinput_main
#include "wine_gameinput.c"
#undef main
#undef Ref
#undef Find
#undef SLOT
#define main provider_main
#include "wine_process_providers.c"
#undef main
#undef SLOT
#define mainCRTStartup compute_main
#include "windows_d3d12x_compute.c"
#undef mainCRTStartup

int main(void) {
    WCHAR path[32768], expected[32768];
    DWORD length = GetModuleFileNameW(NULL, path, 32768);
    DWORD expected_length = GetEnvironmentVariableW(L"VICTORY_EXPECT_GUEST_EXE", expected, 32768);
    CHECK(length && length < 32768 && expected_length && expected_length < 32768);
    CHECK(!_wcsicmp(path, expected));
    WCHAR *name = wcsrchr(path, L'\\');
    CHECK(name && (size_t)(name - path) + 12 < 32768);
    wcscpy(name + 1, L"asset.txt");
    HANDLE asset = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    CHECK(asset != INVALID_HANDLE_VALUE);
    char content[8] = {0};
    DWORD read = 0;
    CHECK(ReadFile(asset, content, sizeof(content), &read, NULL));
    CHECK(CloseHandle(asset) && read == 5 && !memcmp(content, "asset", 5));
    puts("Guest image path and adjacent resource passed");
    int result = provider_main();
    if (result) return result;
    result = gameinput_main();
    return result ? result : (int)compute_main(NULL);
}
