/* Normal Wine PE exercising the real title-provider relaunch vtable. */
#include <windows.h>
#include <stdio.h>
#include <string.h>

__declspec(dllimport) HRESULT WINAPI QueryApiImpl(const GUID *, const GUID *, void **);

typedef void (WINAPI *LaunchNewGame)(void *, const char *, const char *, const void *);
static const GUID title_provider = {0x973a344e, 0x24bf, 0x4d0f, {0x84, 0x57, 0x56, 0xc5, 0x34, 0x89, 0x2b, 0x29}};
static const GUID title_iid = {0x2549f142, 0x6419, 0x4a06, {0x97, 0xb5, 0x93, 0x1a, 0xab, 0x7c, 0x2f, 0x34}};
static const char sentinel[] = "first guest completed before relaunch";
typedef struct { unsigned short major, minor, build, revision; } Version;
typedef struct { Version os, hosting_os; char family[64], form[64]; } AnalyticsInfo;
typedef struct { size_t title_limit, title_used, tools_limit, tools_used; unsigned stack_used, physical_pages; size_t gpu_limit, gpu_used, tools_gpu_limit, tools_gpu_used; unsigned kernel_used, kernel_limit; } WorkingSet;
static int check_console(void) {
    const GUID id = {0xb884675d, 0xb738, 0x4a9c, {0x81,0x5d,0x9a,0x9a,0x1e,0x0c,0x6c,0x9b}};
    void *object = NULL;
    if (QueryApiImpl(&id, &id, &object) || !object) return 0;
    typedef AnalyticsInfo *(WINAPI *GetInfo)(void *, AnalyticsInfo *);
    AnalyticsInfo info;
    if (((GetInfo)(*(void ***)object)[3])(object, &info) != &info ||
        strcmp(info.form, "Xbox Series S")) return 0;
    HMODULE xmem = LoadLibraryW(L"xmem.dll");
    if (!xmem) return 0;
    typedef BOOL (WINAPI *GetStats)(unsigned, WorkingSet *);
    typedef BOOL (WINAPI *Transfer)(unsigned, size_t);
    GetStats get = (GetStats)GetProcAddress(xmem, "XMemGetWorkingSetStatistics");
    Transfer transfer = (Transfer)GetProcAddress(xmem, "XMemTransferMemory");
    WorkingSet stats;
    return get && transfer && get(0, &stats) && stats.title_limit == ((size_t)7776 << 20) &&
        stats.gpu_limit == 0 && transfer(0, 1) && get(0, &stats) &&
        stats.title_limit == ((size_t)7774 << 20);
}

static int env_path(const char *name, char *path, DWORD size) {
    DWORD len = GetEnvironmentVariableA(name, path, size);
    return len && len < size;
}

int main(int argc, char **argv) {
    char target[32768], marker[32768], image[32768], expected[32768], data[sizeof(sentinel)];
    DWORD count;
    HANDLE file;
    if (!check_console()) return 21;
    if (!env_path("VICTORY_RELAUNCH_TEST_TARGET", target, sizeof(target)) ||
        !env_path("VICTORY_RELAUNCH_TEST_MARKER", marker, sizeof(marker))) return 10;
    if (argc == 1) {
        void *object = NULL;
        if (QueryApiImpl(&title_provider, &title_iid, &object) != 0 || !object) return 11;
        file = CreateFileA(marker, GENERIC_WRITE, 0, NULL, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, NULL);
        if (file == INVALID_HANDLE_VALUE) return 12;
        DWORD first_pid = GetCurrentProcessId();
        int ok = WriteFile(file, sentinel, sizeof(sentinel), &count, NULL) && count == sizeof(sentinel) &&
            WriteFile(file, &first_pid, sizeof(first_pid), &count, NULL) && count == sizeof(first_pid);
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
    DWORD expected_len = GetFullPathNameA(target, sizeof(expected), expected, NULL);
    if (!length || length >= sizeof(image) || !expected_len || expected_len >= sizeof(expected) || _stricmp(image, expected)) {
        fprintf(stderr, "image path mismatch: image=%s target=%s expected=%s\n", image, target, expected);
        return 16;
    }
    file = CreateFileA(marker, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE) return 17;
    DWORD first_pid = 0;
    int ok = ReadFile(file, data, sizeof(data), &count, NULL) && count == sizeof(sentinel) &&
        !memcmp(data, sentinel, sizeof(sentinel)) &&
        ReadFile(file, &first_pid, sizeof(first_pid), &count, NULL) && count == sizeof(first_pid) && first_pid;
    CloseHandle(file);
    if (!ok) return 18;
    HANDLE first = OpenProcess(SYNCHRONIZE, FALSE, first_pid);
    if (first) {
        DWORD state = WaitForSingleObject(first, 0);
        CloseHandle(first);
        if (state != WAIT_OBJECT_0) return 19;
    } else if (GetLastError() != ERROR_INVALID_PARAMETER) return 20;
    puts("relaunch second guest image, arguments and sentinel passed"); fflush(stdout);
    return 42;
}
