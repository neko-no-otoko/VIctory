/* No static XApu import: the live connection must pin its provider. */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <wchar.h>
typedef struct { uint32_t process, streams, commands, memory, frames, options; } Input;
typedef struct { void *memory; HANDLE event; } Output;
#pragma pack(push, 1)
typedef struct { uint32_t kind, stream, sequence, options; uint64_t context; } Id;
typedef struct { Id id; uint32_t state; uint64_t state_data, usec; HRESULT error; uint32_t frames, output_length; uint8_t *output; float pitch; uint32_t input_length; const uint8_t *input; } Result;
#pragma pack(pop)
typedef HRESULT (WINAPI *Connect)(const Input *, Output *, void **);
typedef HRESULT (WINAPI *Disconnect)(void *);
typedef HRESULT (WINAPI *Enqueue)(void *, const Id *, uint32_t *);
typedef HRESULT (WINAPI *Dequeue)(void *, Result *);
typedef uint32_t (WINAPI *Count)(void *, uint32_t *);
int main(void) {
    HMODULE module = LoadLibraryW(L"xapu.dll");
    WCHAR actual[32768], expected[32768];
    DWORD n = module ? GetModuleFileNameW(module, actual, 32768) : 0;
    DWORD e = GetEnvironmentVariableW(L"VICTORY_EXPECT_XAPU_DLL", expected, 32768);
    if (!n || n >= 32768 || !e || e >= 32768 || _wcsicmp(actual, expected)) return 1;
    Connect connect = (Connect)GetProcAddress(module, "XApuConnect");
    Disconnect disconnect = (Disconnect)GetProcAddress(module, "XApuDisconnect");
    Enqueue enqueue = (Enqueue)GetProcAddress(module, "XApuEnqueueCommand");
    Dequeue dequeue = (Dequeue)GetProcAddress(module, "XApuDequeueResult");
    Count count = (Count)GetProcAddress(module, "XApuGetResultCount");
    Input input = {1, 1, 1, 256, 0, 0}; Output out; void *connection = NULL;
    if (!connect || !disconnect || !enqueue || !dequeue || !count || connect(&input, &out, &connection) || !connection || !out.memory) return 2;
    if (!FreeLibrary(module) || GetModuleHandleW(L"xapu.dll") != module) return 3;
    Id id = {4, 0, 42, 0, 0x1234}; Result result; uint32_t total = 0;
    if (enqueue(connection, &id, &total) || total != 1 || count(connection, NULL) != 1 ||
        dequeue(connection, &result) || result.error || result.id.sequence != 42 || result.id.context != 0x1234 ||
        count(connection, &total) || total != 1 || disconnect(connection)) return 4;
    puts("XApu dynamic provider remains pinned");
    return 0;
}
