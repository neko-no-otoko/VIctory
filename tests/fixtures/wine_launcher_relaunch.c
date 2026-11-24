/* Normal Wine PE exercising the real title-provider relaunch vtable. */
#include <windows.h>
#include <stdio.h>
#include <string.h>

__declspec(dllimport) HRESULT WINAPI QueryApiImpl(const GUID *, const GUID *, void **);

typedef void (WINAPI *LaunchNewGame)(void *, const char *, const char *, const void *);
static const GUID title_provider = {0x973a344e, 0x24bf, 0x4d0f, {0x84, 0x57, 0x56, 0xc5, 0x34, 0x89, 0x2b, 0x29}};
static const GUID title_iid = {0x2549f142, 0x6419, 0x4a06, {0x97, 0xb5, 0x93, 0x1a, 0xab, 0x7c, 0x2f, 0x34}};
static const char sentinel[] = "first guest completed before relaunch";

static int env_path(const char *name, char *path, DWORD size) {
    DWORD len = GetEnvironmentVariableA(name, path, size);
    return len && len < size;
}

int main(int argc, char **argv) {
    char target[32768], marker[32768], image[32768], expected[32768], data[sizeof(sentinel)];
    DWORD count;
    HANDLE file;
    if (!env_path("VICTORY_RELAUNCH_TEST_TARGET", target, sizeof(target)) ||
        !env_path("VICTORY_RELAUNCH_TEST_MARKER", marker, sizeof(marker))) return 10;
    if (argc == 1) {
        void *object = NULL;
        if (QueryApiImpl(&title_provider, &title_iid, &object) != 0 || !object) return 11;
        file = CreateFileA(marker, GENERIC_WRITE, 0, NULL, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, NULL);
        if (file == INVALID_HANDLE_VALUE) return 12;
        int ok = WriteFile(file, sentinel, sizeof(sentinel), &count, NULL) && count == sizeof(sentinel);
        CloseHandle(file);
        if (!ok) return 13;
        puts("relaunch first guest sentinel written"); fflush(stdout);
        void **vtable = *(void ***)object;
        LaunchNewGame launch = (LaunchNewGame)vtable[0x20 / sizeof(void *)];
        launch(object, target, "--phase second", NULL);
        fputs("XLaunchNewGame unexpectedly returned\n", stderr);
        return 14;
    }
    if (argc != 3 || strcmp(argv[1], "--phase") || strcmp(argv[2], "second")) return 15;
    DWORD length = GetModuleFileNameA(NULL, image, sizeof(image));
    if (!length || length >= sizeof(image) ||
        !GetFullPathNameA(target, sizeof(expected), expected, NULL) || _stricmp(image, expected)) return 16;
    file = CreateFileA(marker, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE) return 17;
    int ok = ReadFile(file, data, sizeof(data), &count, NULL) && count == sizeof(sentinel) &&
        !memcmp(data, sentinel, sizeof(sentinel));
    CloseHandle(file);
    if (!ok) return 18;
    puts("relaunch second guest image, arguments and sentinel passed"); fflush(stdout);
    return 42;
}
