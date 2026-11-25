/* Ordinary PE coverage for the chad DSP ABI. */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>
#ifdef UNSUPPORTED_IMPORT
__declspec(dllimport) void WINAPI VIctoryUnsupportedImport(void);
int main(void) { VIctoryUnsupportedImport(); return 0; }
#else
#define INVALID ((HRESULT)0x80070057u)
#define POINTER ((HRESULT)0x80004003u)
#define OUT_OF_MEMORY ((HRESULT)0x8007000eu)
#pragma pack(push, 1)
typedef struct { uint32_t kind, options, frames, channels, ir_floats; const float *ir; } Desc;
#pragma pack(pop)
typedef struct { uint32_t submitted, completed, reserved; HRESULT result; } Status;
typedef struct { float gains[2]; const float *input; float *output; uint64_t reserved; } Command;
__declspec(dllimport) HRESULT WINAPI XDspConnectWithMaximumStreamLimit(void *, uint32_t, uint32_t, uint32_t, uint32_t, void **);
__declspec(dllimport) HRESULT WINAPI XDspDisconnect(void *);
__declspec(dllimport) HRESULT WINAPI XDspActivate(void *, const Desc *, const Status **, void **);
__declspec(dllimport) HRESULT WINAPI XDspDeactivate(void *);
__declspec(dllimport) HRESULT WINAPI XDspSubmitCommand(void *, const Command *, uint32_t *);
#define CHECK(x) do { if (!(x)) { printf("chad check failed line %d\n", __LINE__); return 1; } } while (0)
static const char *const exports[] = {"XDspConnectWithMaximumStreamLimit", "XDspDisconnect", "XDspActivate", "XDspDeactivate", "XDspSubmitCommand"};
static int named_imports(void) {
    unsigned char *base = (unsigned char *)GetModuleHandleW(NULL);
    IMAGE_NT_HEADERS64 *nt = (IMAGE_NT_HEADERS64 *)(base + ((IMAGE_DOS_HEADER *)base)->e_lfanew);
    IMAGE_IMPORT_DESCRIPTOR *d = (IMAGE_IMPORT_DESCRIPTOR *)(base + nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].VirtualAddress);
    unsigned seen = 0;
    for (; d->Name; ++d) {
        if (_stricmp((char *)(base + d->Name), "chad.dll")) continue;
        if (!d->OriginalFirstThunk) return 0;
        IMAGE_THUNK_DATA64 *t = (IMAGE_THUNK_DATA64 *)(base + d->OriginalFirstThunk);
        for (; t->u1.AddressOfData; ++t) {
            if (IMAGE_SNAP_BY_ORDINAL64(t->u1.Ordinal)) return 0;
            const char *name = (const char *)((IMAGE_IMPORT_BY_NAME *)(base + t->u1.AddressOfData))->Name;
            unsigned i; for (i = 0; i < 5; ++i) if (!strcmp(name, exports[i])) break;
            if (i == 5 || (seen & (1u << i))) return 0;
            seen |= 1u << i;
        }
    }
    return seen == 31;
}
int main(void) {
    HMODULE module = GetModuleHandleW(L"chad.dll");
    WCHAR actual[32768], expected[32768];
    DWORD n = module ? GetModuleFileNameW(module, actual, 32768) : 0;
    DWORD e = GetEnvironmentVariableW(L"VICTORY_EXPECT_CHAD_DLL", expected, 32768);
    CHECK(n && n < 32768 && e && e < 32768 && !_wcsicmp(actual, expected) && named_imports());
    for (unsigned i = 0; i < 5; ++i) CHECK(GetProcAddress(module, exports[i]));
    CHECK(!GetProcAddress(module, "VIctoryUnsupportedImport"));
    char memory[0x10000]; void *dsp = NULL, *stream = NULL, *other = NULL;
    const Status *status = NULL, *other_status = NULL;
    Desc desc = {1, 0, 0x200, 2, 0, NULL};
    CHECK(XDspConnectWithMaximumStreamLimit(NULL, 0x10000, 0, 1, 2, &dsp) == INVALID);
    CHECK(XDspConnectWithMaximumStreamLimit(memory, 0xffff, 0, 1, 2, &dsp) == INVALID);
    CHECK(XDspConnectWithMaximumStreamLimit(memory, 0x1f400001, 0, 1, 2, &dsp) == INVALID);
    CHECK(XDspConnectWithMaximumStreamLimit(memory, 0x10000, 129, 1, 2, &dsp) == INVALID);
    CHECK(XDspConnectWithMaximumStreamLimit(memory, 0x10000, 0, 257, 2, &dsp) == INVALID);
    CHECK(XDspConnectWithMaximumStreamLimit(memory, 0x10000, 0, 1, 3, &dsp) == INVALID);
    CHECK(XDspConnectWithMaximumStreamLimit(memory, 0x10000, 0, 1, 2, NULL) == POINTER);
    CHECK(XDspDisconnect(NULL) == INVALID && XDspDeactivate(NULL) == INVALID);
    CHECK(XDspConnectWithMaximumStreamLimit(memory, 0x10000, 0, 1, 2, &dsp) == 0 && dsp);
    CHECK(XDspActivate(dsp, &desc, NULL, &stream) == POINTER);
    CHECK(XDspActivate(dsp, &desc, &status, NULL) == POINTER);
    CHECK(XDspActivate(NULL, &desc, &status, &stream) == INVALID);
    CHECK(XDspActivate(dsp, NULL, &status, &stream) == INVALID);
    Desc bad = desc; bad.frames = 0x800; CHECK(XDspActivate(dsp, &bad, &status, &stream) == INVALID);
    bad = desc; bad.options = 4; CHECK(XDspActivate(dsp, &bad, &status, &stream) == INVALID);
    bad = desc; bad.channels = 1; bad.options = 2; CHECK(XDspActivate(dsp, &bad, &status, &stream) == INVALID);
    bad = desc; bad.kind = 0; CHECK(XDspActivate(dsp, &bad, &status, &stream) == INVALID);
    CHECK(XDspActivate(dsp, &desc, &status, &stream) == 0 && status && stream);
    CHECK(!status->submitted && !status->completed && !status->reserved && !status->result);
    CHECK(XDspActivate(dsp, &desc, &other_status, &other) == OUT_OF_MEMORY);
    float input[1024], output[1026];
    for (unsigned i = 0; i < 1024; ++i) input[i] = 1.0f;
    for (unsigned i = 0; i < 1026; ++i) output[i] = 7.0f;
    Command cmd = {{1.0f, 0.5f}, input, output + 1, 0}; uint32_t ticket = 0;
    CHECK(XDspSubmitCommand(NULL, &cmd, &ticket) == INVALID);
    CHECK(XDspSubmitCommand(stream, NULL, &ticket) == INVALID);
    CHECK(XDspSubmitCommand(stream, &cmd, &ticket) == 0 && ticket == 1);
    CHECK(status->submitted == 1 && status->completed == 1 && !status->result && !status->reserved);
    CHECK(output[0] == 7.0f && output[1025] == 7.0f);
    for (unsigned i = 1; i <= 1024; ++i) CHECK(output[i] == 0.0f);
    cmd.output = NULL; CHECK(XDspSubmitCommand(stream, &cmd, NULL) == 0);
    CHECK(status->submitted == 2 && status->completed == 2 && !status->result);
    CHECK(XDspDeactivate(stream) == 0);
    desc.kind = 0; desc.frames = 0x800; desc.channels = 1;
    CHECK(XDspActivate(dsp, &desc, &other_status, &other) == 0 && other_status && other);
    CHECK(!other_status->submitted && !other_status->completed);
    CHECK(XDspDeactivate(other) == 0 && XDspDisconnect(dsp) == 0);
    puts("chad named imports, lifecycle and silence passed");
    return 0;
}
#endif
