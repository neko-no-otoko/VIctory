/* A live DSP connection keeps a dynamically loaded provider resident. */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <wchar.h>
#pragma pack(push, 1)
typedef struct { uint32_t kind, options, frames, channels, ir_floats; const float *ir; } Desc;
#pragma pack(pop)
typedef struct { uint32_t submitted, completed, reserved; HRESULT result; } Status;
typedef struct { float gains[2]; const float *input; float *output; uint64_t reserved; } Command;
typedef HRESULT (WINAPI *Connect)(void *, uint32_t, uint32_t, uint32_t, uint32_t, void **);
typedef HRESULT (WINAPI *Disconnect)(void *);
typedef HRESULT (WINAPI *Activate)(void *, const Desc *, const Status **, void **);
typedef HRESULT (WINAPI *Deactivate)(void *);
typedef HRESULT (WINAPI *Submit)(void *, const Command *, uint32_t *);
int main(void) {
    HMODULE module = LoadLibraryW(L"chad.dll");
    WCHAR actual[32768], expected[32768];
    DWORD n = module ? GetModuleFileNameW(module, actual, 32768) : 0;
    DWORD e = GetEnvironmentVariableW(L"VICTORY_EXPECT_CHAD_DLL", expected, 32768);
    if (!n || n >= 32768 || !e || e >= 32768 || _wcsicmp(actual, expected)) return 1;
    Connect connect = (Connect)GetProcAddress(module, "XDspConnectWithMaximumStreamLimit");
    Disconnect disconnect = (Disconnect)GetProcAddress(module, "XDspDisconnect");
    Activate activate = (Activate)GetProcAddress(module, "XDspActivate");
    Deactivate deactivate = (Deactivate)GetProcAddress(module, "XDspDeactivate");
    Submit submit = (Submit)GetProcAddress(module, "XDspSubmitCommand");
    char memory[0x10000]; void *dsp = NULL, *stream = NULL; const Status *status = NULL;
    if (!connect || !disconnect || !activate || !deactivate || !submit ||
        connect(memory, 0x10000, 0, 1, 1, &dsp) || !dsp) return 2;
    if (!FreeLibrary(module) || GetModuleHandleW(L"chad.dll") != module) return 3;
    Desc desc = {1, 0, 0x200, 1, 0, NULL}; float output[512];
    for (unsigned i = 0; i < 512; ++i) output[i] = 1.0f;
    Command cmd = {{1.0f, 0.0f}, NULL, output, 0}; uint32_t ticket = 0;
    if (activate(dsp, &desc, &status, &stream) || !status || !stream ||
        submit(stream, &cmd, &ticket) || ticket != 1 || status->completed != 1 || status->result) return 4;
    for (unsigned i = 0; i < 512; ++i) if (output[i] != 0.0f) return 5;
    if (deactivate(stream) || disconnect(dsp)) return 6;
    puts("chad dynamic provider remains pinned");
    return 0;
}
