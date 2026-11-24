/* Self-authored PE guest for the experimental private Wine/Metal bridge. */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdint.h>

__declspec(dllimport) int __stdcall VictoryMetalClearRgba8(
    unsigned width, unsigned height, unsigned pitch, unsigned char *pixels,
    unsigned len, unsigned color);

#define TLS_INITIAL UINT32_C(0x13579bdf)

static _Thread_local uint32_t tls_value = TLS_INITIAL;
static _Thread_local uint32_t tls_expected;
static _Thread_local unsigned tls_phase;
static volatile LONG process_attaches;
static volatile LONG thread_attaches;
static volatile LONG thread_detaches;
static volatile LONG callback_errors;

static void NTAPI on_tls(PVOID module, DWORD reason, PVOID reserved)
{
    (void)module;
    (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        if (tls_value != TLS_INITIAL || tls_expected != 0 || tls_phase != 0)
            InterlockedIncrement(&callback_errors);
        tls_phase = 1;
        if (reason == DLL_PROCESS_ATTACH)
            InterlockedIncrement(&process_attaches);
        else
            InterlockedIncrement(&thread_attaches);
    } else if (reason == DLL_THREAD_DETACH) {
        if (tls_phase != 2 || tls_expected == 0 || tls_value != tls_expected)
            InterlockedIncrement(&callback_errors);
        InterlockedIncrement(&thread_detaches);
    }
}

/* LLVM-MinGW's PE CRT gathers this pointer into the image TLS callback list. */
__attribute__((used, section(".CRT$XLB")))
static PIMAGE_TLS_CALLBACK const tls_callback = on_tls;

struct worker_args {
    HANDLE ready;
    HANDLE start;
    volatile LONG *ready_count;
    uint32_t tls_marker;
    uint32_t color;
};

static DWORD WINAPI worker(void *argument)
{
    struct worker_args *args = (struct worker_args *)argument;
    unsigned char pixels[16];
    unsigned i;
    int ok = tls_phase == 1 && tls_value == TLS_INITIAL && tls_expected == 0;

    tls_value = args->tls_marker;
    tls_expected = args->tls_marker;
    for (i = 0; i < sizeof(pixels); ++i)
        pixels[i] = (unsigned char)(0x40u + i);

    /* Both threads have changed their TLS before either begins the call. */
    if (InterlockedIncrement(args->ready_count) == 2 && !SetEvent(args->ready))
        ok = 0;
    if (WaitForSingleObject(args->start, INFINITE) != WAIT_OBJECT_0)
        ok = 0;
    if (tls_value != args->tls_marker || tls_expected != args->tls_marker ||
        tls_phase != 1)
        ok = 0;
    if (VictoryMetalClearRgba8(2, 2, 8, pixels, sizeof(pixels), args->color) != 0)
        ok = 0;
    for (i = 0; i < sizeof(pixels); ++i) {
        unsigned char expected = (unsigned char)(args->color >> (8 * (i % 4)));
        if (pixels[i] != expected)
            ok = 0;
    }
    if (tls_value != args->tls_marker || tls_expected != args->tls_marker ||
        tls_phase != 1)
        ok = 0;
    tls_phase = 2;
    return ok ? 0 : 1;
}

int main(void)
{
    HANDLE ready = NULL, start = NULL, threads[2] = { NULL, NULL };
    HANDLE wait_handles[3];
    volatile LONG ready_count = 0;
    struct worker_args args[2];
    DWORD wait_result, exit_code;
    unsigned i;
    int ok = 1;

    if (process_attaches != 1 || callback_errors != 0 || tls_phase != 1 ||
        tls_value != TLS_INITIAL || tls_expected != 0)
        return 1;
    tls_value = UINT32_C(0x2468ace0);
    ready = CreateEventW(NULL, TRUE, FALSE, NULL);
    start = CreateEventW(NULL, TRUE, FALSE, NULL);
    if (ready == NULL || start == NULL) {
        ok = 0;
        goto cleanup;
    }
    args[0] = (struct worker_args){ ready, start, &ready_count,
                                    UINT32_C(0x11112222), UINT32_C(0xff0000ff) };
    args[1] = (struct worker_args){ ready, start, &ready_count,
                                    UINT32_C(0x33334444), UINT32_C(0xffff0000) };
    for (i = 0; i < 2; ++i) {
        threads[i] = CreateThread(NULL, 0, worker, &args[i], 0, NULL);
        if (threads[i] == NULL) {
            ok = 0;
            goto release_workers;
        }
    }
    wait_handles[0] = ready;
    wait_handles[1] = threads[0];
    wait_handles[2] = threads[1];
    wait_result = WaitForMultipleObjects(3, wait_handles, FALSE, INFINITE);
    if (wait_result != WAIT_OBJECT_0)
        ok = 0;

release_workers:
    if (!SetEvent(start))
        ok = 0;
    for (i = 0; i < 2; ++i) {
        if (threads[i] != NULL) {
            if (WaitForSingleObject(threads[i], INFINITE) != WAIT_OBJECT_0 ||
                !GetExitCodeThread(threads[i], &exit_code) || exit_code != 0)
                ok = 0;
        }
    }
    if (thread_attaches != 2 || thread_detaches != 2 || callback_errors != 0 ||
        tls_phase != 1 || tls_value != UINT32_C(0x2468ace0) || tls_expected != 0)
        ok = 0;

cleanup:
    for (i = 0; i < 2; ++i) {
        if (threads[i] != NULL)
            CloseHandle(threads[i]);
    }
    if (start != NULL)
        CloseHandle(start);
    if (ready != NULL)
        CloseHandle(ready);
    return ok ? 0 : 1;
}
