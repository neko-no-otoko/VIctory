/* Imported both by name and ordinal, using independently generated import libraries. */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>

__declspec(dllimport) HRESULT WINAPI GameInputCreate(void **);
__declspec(dllimport) HRESULT WINAPI OrdinalCreate(void **);
#define SLOT(o, t, n) ((t)(*(void ***)(o))[n])
#define CHECK(x) do { if (!(x)) { printf("GameInput check failed at line %d\n", __LINE__); return 1; } } while (0)
typedef ULONG (WINAPI *Ref)(void *);
typedef uint64_t (WINAPI *Clock)(void *);
typedef HRESULT (WINAPI *Current)(void *, unsigned, void *, void **);
typedef unsigned (WINAPI *Uint)(void *);
typedef uint64_t (WINAPI *Sequence)(void *, unsigned);
typedef void (WINAPI *GetDevice)(void *, void **);
typedef HRESULT (WINAPI *Out)(void *, void **);
typedef HRESULT (WINAPI *Aggregate)(void *, unsigned, void **);
typedef HRESULT (WINAPI *Find)(void *, const void *, void **);
typedef HRESULT (WINAPI *RegisterDevice)(void *, void *, unsigned, unsigned, unsigned, void *, void (WINAPI *)(uint64_t, void *, void *, uint64_t, unsigned, unsigned), uint64_t *);
typedef BOOLEAN (WINAPI *Unregister)(void *, uint64_t, uint64_t);
typedef HRESULT (WINAPI *RegisterReading)(void *, void *, unsigned, float, void *, void *, uint64_t *);
typedef BOOLEAN (WINAPI *PadState)(void *, void *);
static unsigned callbacks;
static void WINAPI on_device(uint64_t token, void *context, void *device, uint64_t time, unsigned current, unsigned previous) {
    (void)token; (void)time; (void)previous;
    if (context == &callbacks && device && (current & 1)) ++callbacks;
}
/* Inspect the loaded image's original import thunks, not GetProcAddress. */
static int imports(void) {
    unsigned char *base = (unsigned char *)GetModuleHandleW(NULL);
    IMAGE_DOS_HEADER *dos = (IMAGE_DOS_HEADER *)base;
    IMAGE_NT_HEADERS64 *nt = (IMAGE_NT_HEADERS64 *)(base + dos->e_lfanew);
    IMAGE_IMPORT_DESCRIPTOR *d = (IMAGE_IMPORT_DESCRIPTOR *)(base + nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].VirtualAddress);
    int named = 0, ordinal = 0;
    for (; d->Name; ++d) {
        if (_stricmp((char *)(base + d->Name), "GameInput.dll")) continue;
        if (!d->OriginalFirstThunk) return 0;
        IMAGE_THUNK_DATA64 *t = (IMAGE_THUNK_DATA64 *)(base + d->OriginalFirstThunk);
        for (; t->u1.AddressOfData; ++t) {
            if (IMAGE_SNAP_BY_ORDINAL64(t->u1.Ordinal)) {
                if (IMAGE_ORDINAL64(t->u1.Ordinal) != 1) return 0;
                ordinal++;
            } else {
                IMAGE_IMPORT_BY_NAME *n = (IMAGE_IMPORT_BY_NAME *)(base + t->u1.AddressOfData);
                if (strcmp((char *)n->Name, "GameInputCreate")) return 0;
                named++;
            }
        }
    }
    return named == 1 && ordinal == 1;
}
int main(void) {
    WCHAR actual[32768], expected[32768];
    HMODULE module = GetModuleHandleW(L"GameInput.dll");
    DWORD n = module ? GetModuleFileNameW(module, actual, 32768) : 0;
    DWORD e = GetEnvironmentVariableW(L"VICTORY_EXPECT_GAMEINPUT_DLL", expected, 32768);
    CHECK(n && n < 32768 && e && e < 32768 && !_wcsicmp(actual, expected));
    CHECK(imports());
    FARPROC named = GetProcAddress(module, "GameInputCreate");
    FARPROC ordinal = GetProcAddress(module, (LPCSTR)(uintptr_t)1);
    CHECK(named && named == ordinal && !GetProcAddress(module, (LPCSTR)(uintptr_t)2));
    CHECK((FARPROC)GameInputCreate == named && (FARPROC)OrdinalCreate == ordinal);
    CHECK((uint32_t)GameInputCreate(NULL) == 0x80070057u);
    void *a = NULL, *b = NULL;
    CHECK(!GameInputCreate(&a) && a && !OrdinalCreate(&b) && a == b);
    CHECK(SLOT(a, Ref, 1)(a) == 1 && SLOT(a, Ref, 2)(a) == 1);
    uint64_t before = SLOT(a, Clock, 3)(a); Sleep(2);
    CHECK(SLOT(a, Clock, 3)(a) >= before);
    void *reading = NULL, *device = NULL;
    CHECK(!SLOT(a, Current, 4)(a, 0x10, NULL, &reading) && reading);
    CHECK(SLOT(reading, Uint, 3)(reading) == 0x10);
    CHECK(SLOT(reading, Sequence, 4)(reading, 0x40000) == 0);
    CHECK(SLOT(reading, Clock, 5)(reading) <= SLOT(a, Clock, 3)(a));
    SLOT(reading, GetDevice, 6)(reading, &device); CHECK(device);
    CHECK((SLOT(device, Uint, 4)(device) & 1) != 0);
    CHECK(SLOT(device, Ref, 1)(device) == 1 && SLOT(device, Ref, 2)(device) == 1);
    ULONG refs = SLOT(reading, Ref, 1)(reading); CHECK(refs >= 2);
    CHECK(SLOT(reading, Ref, 2)(reading) == refs - 1);
    CHECK(!SLOT(reading, PadState, 22)(reading, NULL));
    void *out = (void *)(uintptr_t)1;
    CHECK((uint32_t)SLOT(a, Current, 4)(a, 0x20000000, NULL, &out) == 0x838a0003u && !out);
    out = (void *)(uintptr_t)1;
    CHECK((uint32_t)SLOT(device, Out, 5)(device, NULL) == 0x80004001u);
    CHECK((uint32_t)SLOT(a, Out, 14)(a, &out) == 0x80004001u && !out);
    out = (void *)(uintptr_t)1;
    CHECK((uint32_t)SLOT(a, Aggregate, 15)(a, 0x10, &out) == 0x80004001u && !out);
    out = (void *)(uintptr_t)1;
    CHECK((uint32_t)SLOT(a, Find, 17)(a, NULL, &out) == 0x838a0002u && !out);
    uint64_t token = 0;
    CHECK(!SLOT(a, RegisterDevice, 9)(a, NULL, 0x10, 1, 2, &callbacks, on_device, &token));
    CHECK(token && callbacks == 1 && SLOT(a, Unregister, 13)(a, token, 1000000));
    token = 0;
    CHECK(!SLOT(a, RegisterReading, 8)(a, NULL, 0x10, 0, NULL, NULL, &token) && token);
    CHECK(SLOT(a, Unregister, 13)(a, token, 0));
    HMODULE extra = LoadLibraryW(L"GameInput.dll"); CHECK(extra == module);
    CHECK(FreeLibrary(extra));
    CHECK(GetModuleHandleW(L"GameInput.dll") == module);
    CHECK(SLOT(reading, Uint, 3)(reading) == 0x10);
    CHECK((SLOT(device, Uint, 4)(device) & 1) != 0);
    CHECK(SLOT(a, Clock, 3)(a) >= before);
    SLOT(reading, Ref, 2)(reading);
    puts("GameInput imports, singleton, readings, callbacks and pinned lifetime passed");
    return 0;
}
