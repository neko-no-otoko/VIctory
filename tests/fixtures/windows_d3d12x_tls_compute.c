/* Controlled Wine/VIctory TLS and D3D12.X compute fixture. The loader still
 * uses its existing EXE-as-DLL TLS redirection; this exercises one compiler
 * output, not arbitrary PE instruction streams. */
#define mainCRTStartup compute_entry
#include "windows_d3d12x_compute.c"
#undef mainCRTStartup

static _Thread_local int tls_value = 17;
static volatile LONG tls_attach, tls_detach, tls_bad, ready;
static HANDLE gate;

static void NTAPI tls_callback(PVOID image, DWORD reason, PVOID reserved) {
    (void)image; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        if (tls_value != 17) InterlockedIncrement(&tls_bad);
        if (reason == DLL_THREAD_ATTACH) InterlockedIncrement(&tls_attach);
    } else if (reason == DLL_THREAD_DETACH) {
        InterlockedIncrement(&tls_detach);
    }
}
__attribute__((used, section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK tls_callback_slot = tls_callback;

static DWORD WINAPI tls_worker(void *arg) {
    int value = (int)(INT_PTR)arg;
    if (tls_value != 17) return 1;
    tls_value = value;
    InterlockedIncrement(&ready);
    if (WaitForSingleObject(gate, 5000) != WAIT_OBJECT_0) return 2;
    return tls_value == value ? 0 : 3;
}

unsigned mainCRTStartup(void *unused) {
    HANDLE threads[2]; DWORD codes[2];
    if (tls_value != 17 || tls_bad) { SAY("TLS initial state failed\r\n"); return 20; }
    tls_value = 99;
    gate = CreateEventW(NULL, TRUE, FALSE, NULL);
    if (!gate) return 21;
    threads[0] = CreateThread(NULL, 0, tls_worker, (void *)(INT_PTR)111, 0, NULL);
    threads[1] = CreateThread(NULL, 0, tls_worker, (void *)(INT_PTR)222, 0, NULL);
    if (!threads[0] || !threads[1]) return 22;
    ULONGLONG deadline = GetTickCount64() + 5000;
    while (InterlockedCompareExchange(&ready, 0, 0) != 2 && GetTickCount64() < deadline) Sleep(1);
    if (ready != 2 || tls_value != 99 || !SetEvent(gate)) return 23;
    if (WaitForMultipleObjects(2, threads, TRUE, 5000) != WAIT_OBJECT_0) return 24;
    if (!GetExitCodeThread(threads[0], &codes[0]) || !GetExitCodeThread(threads[1], &codes[1])) return 25;
    CloseHandle(threads[0]); CloseHandle(threads[1]); CloseHandle(gate);
    if (codes[0] || codes[1] || tls_attach < 2 || tls_detach < 2 || tls_bad || tls_value != 99) {
        SAY("TLS thread state failed\r\n"); return 26;
    }
    SAY("TLS compiler state and thread callbacks passed\r\n");
    return compute_entry(unused);
}
