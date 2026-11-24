/* Self-authored PE fixture: a real import and a rebased data pointer. */
__declspec(dllimport) int host_adjust(int value);
__declspec(dllexport) const int fixture_value = 40;
__declspec(dllexport) const int *volatile fixture_pointer = &fixture_value;
__declspec(dllexport) int fixture_call(void) {
    return host_adjust(*fixture_pointer);
}
