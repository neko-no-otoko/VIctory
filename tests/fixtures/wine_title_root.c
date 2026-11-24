/* Synthetic normal PE: exercise Wine's Win32 file APIs, not DirectStorage. */
#include <windows.h>
#include <string.h>

static _Thread_local int tls_value = 17;
static volatile LONG process_attaches, thread_attaches, thread_detaches, bad_tls, ready;
static HANDLE gate;
static const char payload[] = "VIctory title-root fixture: UTF-8 \xc3\xa9\n";

static void NTAPI tls_callback(PVOID image, DWORD reason, PVOID reserved) {
    (void)image; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        if (tls_value != 17) InterlockedIncrement(&bad_tls);
        if (reason == DLL_PROCESS_ATTACH) InterlockedIncrement(&process_attaches);
        else InterlockedIncrement(&thread_attaches);
    } else if (reason == DLL_THREAD_DETACH) {
        InterlockedIncrement(&thread_detaches);
    }
}
__attribute__((used, section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK title_tls_callback = tls_callback;

static int read_file(const WCHAR *path) {
    char buffer[sizeof(payload) + 8];
    DWORD count = 0;
    LARGE_INTEGER size;
    HANDLE file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING,
                              FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE) return 1;
    int ok = GetFileSizeEx(file, &size) && size.QuadPart == sizeof(payload) - 1 &&
        ReadFile(file, buffer, sizeof(buffer), &count, NULL) &&
        count == sizeof(payload) - 1 && !memcmp(buffer, payload, count);
    CloseHandle(file);
    return ok ? 0 : 2;
}

static DWORD WINAPI worker(void *arg) {
    int value = (int)(INT_PTR)arg;
    if (tls_value != 17) return 1;
    tls_value = value;
    InterlockedIncrement(&ready);
    if (WaitForSingleObject(gate, 10000) != WAIT_OBJECT_0) return 2;
    if (tls_value != value || read_file(L"G:\\mixed\\caf\u00e9.dat")) return 3;
    return 0;
}

int main(void) {
    HANDLE threads[2], first, second, denied;
    DWORD codes[2], written;
    if (tls_value != 17 || process_attaches != 1 || bad_tls) return 10;
    tls_value = 99;
    if (!SetCurrentDirectoryW(L"G:\\MiXeD")) return 11;
    if (GetFileAttributesW(L"G:\\mixed\\caf\u00e9.dat") == INVALID_FILE_ATTRIBUTES ||
        read_file(L"G:\\mixed\\caf\u00e9.dat") || read_file(L".\\CAF\u00e9.DAT")) return 12;
    if (GetFileAttributesW(L"missing.dat") != INVALID_FILE_ATTRIBUTES ||
        GetLastError() != ERROR_FILE_NOT_FOUND) return 13;
    denied = CreateFileW(L"missing.dat", GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    if (denied != INVALID_HANDLE_VALUE) { CloseHandle(denied); return 14; }
    if (GetLastError() != ERROR_FILE_NOT_FOUND) return 15;
    if (GetFileAttributesW(L"G:\\missing-directory\\file.dat") != INVALID_FILE_ATTRIBUTES ||
        GetLastError() != ERROR_PATH_NOT_FOUND) return 26;
    first = CreateFileW(L"Caf\u00e9.DAT", GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    if (first == INVALID_HANDLE_VALUE) return 16;
    second = CreateFileW(L"caf\u00e9.dat", GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    if (second == INVALID_HANDLE_VALUE) { CloseHandle(first); return 17; }
    denied = CreateFileW(L"caf\u00e9.dat", GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                         NULL, OPEN_EXISTING, 0, NULL);
    if (denied != INVALID_HANDLE_VALUE) { CloseHandle(denied); return 18; }
    if (GetLastError() != ERROR_SHARING_VIOLATION) return 19;
    CloseHandle(second); CloseHandle(first);
    gate = CreateEventW(NULL, TRUE, FALSE, NULL);
    if (!gate) return 20;
    threads[0] = CreateThread(NULL, 0, worker, (void *)(INT_PTR)111, 0, NULL);
    threads[1] = CreateThread(NULL, 0, worker, (void *)(INT_PTR)222, 0, NULL);
    if (!threads[0] || !threads[1]) return 21;
    ULONGLONG deadline = GetTickCount64() + 10000;
    while (InterlockedCompareExchange(&ready, 0, 0) != 2 && GetTickCount64() < deadline) Sleep(1);
    if (ready != 2 || tls_value != 99 || !SetEvent(gate)) return 22;
    if (WaitForMultipleObjects(2, threads, TRUE, 10000) != WAIT_OBJECT_0) return 23;
    if (!GetExitCodeThread(threads[0], &codes[0]) || !GetExitCodeThread(threads[1], &codes[1])) return 24;
    CloseHandle(threads[0]); CloseHandle(threads[1]); CloseHandle(gate);
    if (codes[0] || codes[1] || thread_attaches != 2 || thread_detaches != 2 || bad_tls || tls_value != 99) return 25;
    static const char message[] = "title-root Win32 paths, sharing and TLS passed\r\n";
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), message, sizeof(message) - 1, &written, NULL);
    return 0;
}
