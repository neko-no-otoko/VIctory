/* No static GameInput import: its sole loader reference is released below. */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
typedef HRESULT (WINAPI *Create)(void **);
typedef uint64_t (WINAPI *Clock)(void *);
int main(void) {
    HMODULE module = LoadLibraryW(L"GameInput.dll");
    if (!module) return 1;
    Create create = (Create)GetProcAddress(module, "GameInputCreate");
    void *object = NULL;
    if (!create || create(&object) || !object) return 2;
    Clock clock = (Clock)(*(void ***)object)[3];
    uint64_t before = clock(object);
    if (!FreeLibrary(module) || GetModuleHandleW(L"GameInput.dll") != module) return 3;
    Sleep(30); /* The HLE's polling worker must also survive the release. */
    if (clock(object) < before) return 4;
    puts("GameInput dynamic load remains pinned after FreeLibrary");
    return 0;
}
