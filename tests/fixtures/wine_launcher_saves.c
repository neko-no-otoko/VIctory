/* Normally launched PE exercising the real offline XGameSave provider. */
#include <windows.h>
#include <stdint.h>
#include <string.h>

#ifdef DYNAMIC_RUNTIME
static HRESULT (WINAPI *QueryApiImpl)(const GUID *, const GUID *, void **);
#else
__declspec(dllimport) HRESULT WINAPI QueryApiImpl(const GUID *, const GUID *, void **);
#endif
#define SLOT(o, t, n) ((t)(*(void ***)(o))[n])
#define NO_ACCESS 0x80830002u
#define BLOB_NOT_FOUND 0x80830008u
static const GUID save_id = {0x704c3f58,0xe629,0x4cc2,{0xb1,0x97,0x30,0x51,0x1b,0x99,0x6f,0xe2}};
static const GUID async_id = {0x073b7dcb,0x1fcf,0x4030,{0x94,0xbe,0xe3,0xc9,0xeb,0x62,0x34,0x28}};
static const GUID title_id = {0x973a344e,0x24bf,0x4d0f,{0x84,0x57,0x56,0xc5,0x34,0x89,0x2b,0x29}};
static const GUID title_iid = {0x2549f142,0x6419,0x4a06,{0x97,0xb5,0x93,0x1a,0xab,0x7c,0x2f,0x34}};
static const GUID storage_id = {0xf4faf4d4,0x2d04,0x4fce,{0xb3,0xe0,0x47,0x4a,0x71,0x3a,0x3e,0x84}};
static const GUID storage_iid = {0xd29411df,0x0794,0x4553,{0x8b,0x27,0x95,0xfc,0x02,0xd0,0xf7,0x5d}};
static const GUID package_id = {0xaf406016,0xe850,0x4aa8,{0xa8,0x8d,0x2f,0x3d,0xcb,0x9d,0xac,0x7e}};
static const GUID package_iid = {0x0b9d556e,0xab70,0x43f4,{0xba,0xa0,0x99,0x99,0xac,0x4c,0xb0,0x40}};
typedef struct { void *queue, *context, *callback; uintptr_t internal[4]; } Async;
typedef struct { const char *name; uint32_t size; const void *data; } Blob;
typedef HRESULT (WINAPI *Init)(void *, const void *, const char *, unsigned char, Async *);
typedef HRESULT (WINAPI *InitResult)(void *, Async *, void **);
typedef HRESULT (WINAPI *Status)(void *, Async *, unsigned char);
typedef HRESULT (WINAPI *Create)(void *, void *, const char *, void **);
typedef HRESULT (WINAPI *Write)(void *, void *, const char *, const void *, size_t);
typedef HRESULT (WINAPI *Submit)(void *, void *, Async *);
typedef HRESULT (WINAPI *Result)(void *, Async *);
typedef HRESULT (WINAPI *Read)(void *, void *, const char *const *, uint32_t *, size_t, void *);
typedef void (WINAPI *Close)(void *, void *);
typedef HRESULT (WINAPI *Unsupported)(void *);
typedef HRESULT (WINAPI *TitleId)(void *, uint32_t *);
typedef HRESULT (WINAPI *PackageId)(void *, size_t, char *);
typedef HRESULT (WINAPI *PathSize)(void *, size_t *);
typedef HRESULT (WINAPI *Space)(void *, void *);
typedef HRESULT (WINAPI *Prompt)(void *, uint64_t, Async *);
typedef ULONG (WINAPI *Release)(void *);
static const char payload[] = "persistent offline save\0with binary data";
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
__attribute__((used, section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK save_tls_callback = tls_callback;
static DWORD WINAPI worker(void *arg) {
    int value = (int)(INT_PTR)arg;
    if (tls_value != 17) return 1;
    tls_value = value; InterlockedIncrement(&ready);
    if (WaitForSingleObject(gate, 10000) != WAIT_OBJECT_0) return 2;
    return tls_value != value;
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
    return codes[0] || codes[1] || attaches != 2 || detaches != 2 || bad_tls || tls_value != 99;
}
static void say(const char *message) {
    DWORD written; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), message, (DWORD)strlen(message), &written, NULL);
}
int main(void) {
    char mode[32]; void *save = NULL, *queue = NULL, *title = NULL, *package = NULL, *storage = NULL, *provider = NULL, *container = NULL, *update = NULL;
    uint32_t id = 0, count = 1; unsigned char buffer[512]; const char *names[] = {"blob"};
    Async block = {0}; HRESULT hr;
#ifdef DYNAMIC_RUNTIME
    HMODULE runtime_module = LoadLibraryW(L"xgameruntime.dll");
    if (!runtime_module) return 25;
    QueryApiImpl = (void *)GetProcAddress(runtime_module, "QueryApiImpl");
    if (!QueryApiImpl) return 26;
#endif
    if (!GetEnvironmentVariableA("VICTORY_SAVE_TEST_MODE", mode, sizeof(mode)) || check_tls()) return 10;
    int no_root = !strcmp(mode, "no-root") || !strcmp(mode, "no-manifest");
    uint32_t expected_id = !strcmp(mode, "no-manifest") ? 0 : 0x639aa8dd;
    {
        char configured[32768];
        DWORD n = GetEnvironmentVariableA("VICTORY_WINE_TITLE_DATA", configured, sizeof(configured));
        if ((no_root && n != 0) ||
            (!no_root && (!n || n >= sizeof(configured) ||
             !strcmp(configured, "Z:\\inherited-should-not-be-used")))) return 23;
    }
    if (QueryApiImpl(&title_id, &title_iid, &title) || !title ||
        SLOT(title, TitleId, 3)(title, &id) || id != expected_id) return 11;
#ifdef DYNAMIC_RUNTIME
    if (!FreeLibrary(runtime_module) || !GetModuleHandleW(L"xgameruntime.dll")) return 27;
#endif
    if (SLOT(title, TitleId, 3)(title, &id) || id != expected_id) return 28;
    SLOT(title, Release, 2)(title);
    char package_name[33];
    if (QueryApiImpl(&package_id, &package_iid, &package) || !package ||
        SLOT(package, PackageId, 3)(package, sizeof(package_name), package_name) ||
        strcmp(package_name, expected_id ? "639AA8DD" : "00000000")) return 31;
    SLOT(package, Release, 2)(package);
    size_t path_size = 0;
    if (QueryApiImpl(&storage_id, &storage_iid, &storage) || !storage ||
        (uint32_t)SLOT(storage, PathSize, 3)(storage, &path_size) != 0x80070032u) return 24;
    if (QueryApiImpl(&save_id, &save_id, &save) || !save || QueryApiImpl(&async_id, &async_id, &queue) || !queue) return 12;
    if (!no_root) {
        uint64_t space[4] = {1, 2, 3, 4};
        if ((uint32_t)SLOT(storage, Space, 5)(storage, space) != 0x80070032u ||
            space[0] != 1 || space[1] != 2 || space[2] != 3 || space[3] != 4) return 29;
        if (SLOT(storage, Prompt, 6)(storage, 1, &block) ||
            (uint32_t)SLOT(queue, Status, 3)(queue, &block, 1) != 0x80070032u) return 30;
        memset(&block, 0, sizeof(block));
    }
    SLOT(storage, Release, 2)(storage);
    if ((uint32_t)SLOT(save, Unsupported, 3)(save) != 0x80004001u) return 13;
    if (SLOT(save, Init, 4)(save, NULL, "fixture-scid", 0, &block)) return 14;
    hr = SLOT(queue, Status, 3)(queue, &block, 1);
    if (no_root) {
        if ((uint32_t)hr != NO_ACCESS) return 15;
        say("unconfigured save root denied\r\n");
    } else {
        if (hr || SLOT(save, InitResult, 5)(save, &block, &provider) || !provider) return 16;
        if (SLOT(save, Create, 16)(save, provider, "fixture", &container) || !container) return 17;
        if (!strcmp(mode, "write")) {
            if (SLOT(save, Create, 23)(save, container, "Fixture", &update) || !update ||
                SLOT(save, Write, 25)(save, update, "blob", payload, sizeof(payload))) return 18;
            memset(&block, 0, sizeof(block));
            if (SLOT(save, Submit, 28)(save, update, &block) || SLOT(queue, Status, 3)(queue, &block, 1) ||
                SLOT(save, Result, 29)(save, &block)) return 19;
            SLOT(save, Close, 24)(save, update);
            say("save written\r\n");
        } else {
            hr = SLOT(save, Read, 20)(save, container, names, &count, sizeof(buffer), buffer);
            if (!strcmp(mode, "missing")) {
                if ((uint32_t)hr != BLOB_NOT_FOUND) return 20;
                say("save absent\r\n");
            } else if (!strcmp(mode, "read")) {
                Blob *blob = (Blob *)buffer;
                if (hr || count != 1 || strcmp(blob->name, "blob") || blob->size != sizeof(payload) ||
                    memcmp(blob->data, payload, sizeof(payload))) return 21;
                say("save verified\r\n");
            } else return 22;
        }
        SLOT(save, Close, 17)(save, container);
        SLOT(save, Close, 6)(save, provider);
    }
    SLOT(queue, Release, 2)(queue); SLOT(save, Release, 2)(save);
    say("title id, unsupported blocking slot and TLS passed\r\n");
    return 0;
}
