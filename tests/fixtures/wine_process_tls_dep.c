/* Separate, normally imported PE image with its own compiler-managed TLS. */
#include <windows.h>

static _Thread_local int value = 31;
static volatile LONG process_attach, thread_attach, thread_detach, bad;

static void NTAPI dep_callback(PVOID image, DWORD reason, PVOID reserved) {
    (void)image; (void)reserved;
    if (reason == DLL_PROCESS_ATTACH || reason == DLL_THREAD_ATTACH) {
        if (value != 31) InterlockedIncrement(&bad);
        if (reason == DLL_PROCESS_ATTACH) InterlockedIncrement(&process_attach);
        else InterlockedIncrement(&thread_attach);
    } else if (reason == DLL_THREAD_DETACH) {
        InterlockedIncrement(&thread_detach);
    } else if (reason == DLL_PROCESS_DETACH && value == 199) {
        static const char message[] = "DLL TLS process detach preserved state\r\n";
        DWORD written;
        WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), message, sizeof(message) - 1, &written, NULL);
    }
}
__attribute__((used, section(".CRT$XLB"))) PIMAGE_TLS_CALLBACK dep_callback_slot = dep_callback;

__declspec(dllexport) int dep_get(void) { return value; }
__declspec(dllexport) void dep_set(int v) { value = v; }
__declspec(dllexport) int dep_callbacks_ok(void) {
    return process_attach == 1 && thread_attach == 2 && thread_detach == 2 && bad == 0;
}
