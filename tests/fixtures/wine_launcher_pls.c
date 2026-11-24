/* Normally launched, TLS-bearing PE using the real PLS provider. */
#include <windows.h>
#include <stdint.h>
#include <string.h>
__declspec(dllimport) HRESULT WINAPI QueryApiImpl(const GUID *, const GUID *, void **);
#define SLOT(o,t,n) ((t)(*(void ***)(o))[n])
#define NOT_SUPPORTED 0x80070032u
#define E_ABORT_CODE 0x80004004u
static const GUID provider = {0xf4faf4d4,0x2d04,0x4fce,{0xb3,0xe0,0x47,0x4a,0x71,0x3a,0x3e,0x84}};
static const GUID iid = {0xd29411df,0x0794,0x4553,{0x8b,0x27,0x95,0xfc,0x02,0xd0,0xf7,0x5d}};
static const GUID queue_id = {0x073b7dcb,0x1fcf,0x4030,{0x94,0xbe,0xe3,0xc9,0xeb,0x62,0x34,0x28}};
typedef struct { void *queue, *context, *callback; uintptr_t internal[4]; } Async;
typedef struct { uint64_t available, free, used, total; } Space;
typedef HRESULT (WINAPI *Size)(void *, size_t *);
typedef HRESULT (WINAPI *Path)(void *, size_t, char *, size_t *);
typedef HRESULT (WINAPI *Info)(void *, Space *);
typedef HRESULT (WINAPI *Prompt)(void *, uint64_t, Async *);
typedef HRESULT (WINAPI *Result)(void *, Async *);
typedef HRESULT (WINAPI *Status)(void *, Async *, unsigned char);
typedef ULONG (WINAPI *Release)(void *);
static _Thread_local volatile int tls = 17;
static volatile LONG attached;
static void NTAPI tls_callback(PVOID image, DWORD reason, PVOID reserved) {
    (void)image; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH && tls == 17) InterlockedIncrement(&attached);
}
__attribute__((used, section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK pls_tls_callback = tls_callback;
static const char payload[] = "wine persistent local storage";
int main(void) {
    char mode[32], path[16] = {0}, data[sizeof(payload)];
    size_t size = 0, used = 0; void *storage = NULL, *queue = NULL;
    Space space; Async block = {0}; DWORD count; HANDLE file;
    if (tls != 17) return 10;
    if (attached != 1) return 27;
    if (!GetEnvironmentVariableA("VICTORY_PLS_TEST_MODE", mode, sizeof(mode))) return 28;
    if (QueryApiImpl(&provider, &iid, &storage) || !storage || QueryApiImpl(&queue_id, &queue_id, &queue) || !queue) return 11;
    if (!strcmp(mode, "unsupported")) {
        if ((uint32_t)SLOT(storage, Size, 3)(storage, &size) != NOT_SUPPORTED) return 12;
        if ((uint32_t)SLOT(storage, Path, 4)(storage, sizeof(path), path, &used) != NOT_SUPPORTED) return 13;
        if ((uint32_t)SLOT(storage, Info, 5)(storage, &space) != NOT_SUPPORTED) return 14;
        if (SLOT(storage, Prompt, 6)(storage, 1, &block) || (uint32_t)SLOT(queue, Status, 3)(queue, &block, 1) != NOT_SUPPORTED) return 15;
    } else {
        if (SLOT(storage, Size, 3)(storage, &size) || size != 4) return 16;
        if ((uint32_t)SLOT(storage, Path, 4)(storage, 3, path, &used) != 0x8007007au) return 17;
        if ((uint32_t)SLOT(storage, Path, 4)(storage, sizeof(path), NULL, &used) != 0x8007007au) return 26;
        if (SLOT(storage, Path, 4)(storage, sizeof(path), path, &used) || strcmp(path, "P:\\") || used != 4) return 18;
        if (SLOT(storage, Info, 5)(storage, &space) || space.total != 1048576 || space.used != (!strcmp(mode, "read") ? sizeof(payload) : 0) || space.free != space.total - space.used || space.available > space.free) return 19;
        if (SLOT(storage, Prompt, 6)(storage, space.free, &block) || SLOT(queue, Status, 3)(queue, &block, 1) || SLOT(storage, Result, 7)(storage, &block)) return 20;
        memset(&block, 0, sizeof(block));
        if (SLOT(storage, Prompt, 6)(storage, space.free + 1, &block) || (uint32_t)SLOT(queue, Status, 3)(queue, &block, 1) != E_ABORT_CODE) return 21;
        file = CreateFileA("P:\\pls.bin", !strcmp(mode, "write") ? GENERIC_WRITE : GENERIC_READ, FILE_SHARE_READ, NULL, !strcmp(mode, "write") ? CREATE_ALWAYS : OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
        if (file == INVALID_HANDLE_VALUE) return 22;
        if (!strcmp(mode, "write")) {
            if (!WriteFile(file, payload, sizeof(payload), &count, NULL) || count != sizeof(payload)) return 23;
        } else if (!strcmp(mode, "read")) {
            if (!ReadFile(file, data, sizeof(data), &count, NULL) || count != sizeof(data) || memcmp(data, payload, sizeof(data))) return 24;
        } else return 25;
        CloseHandle(file);
    }
    SLOT(queue, Release, 2)(queue); SLOT(storage, Release, 2)(storage);
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), "PLS passed\n", 11, &count, NULL);
    return 0;
}
