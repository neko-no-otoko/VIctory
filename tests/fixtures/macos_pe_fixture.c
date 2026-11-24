/* Self-authored PE fixture: a real import and a rebased data pointer. */
__declspec(dllimport) int host_adjust(int value);
__declspec(dllexport) const int fixture_value = 40;
__declspec(dllexport) const int *volatile fixture_pointer = &fixture_value;
__declspec(dllexport) int fixture_call(void) {
    return host_adjust(*fixture_pointer);
}

__declspec(dllexport) int fixture_attaches;
__declspec(dllexport) int fixture_detaches;
__declspec(dllexport) int fixture_fail_attach;
__declspec(dllexport) void *fixture_module;
__declspec(dllexport) void *fixture_reserved;

int DllMain(void *module, unsigned long reason, void *reserved) {
    fixture_module = module;
    fixture_reserved = reserved;
    if (reason == 1) {
        ++fixture_attaches;
        return !fixture_fail_attach;
    }
    if (reason == 0) ++fixture_detaches;
    return 1;
}
