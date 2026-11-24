/* Self-authored PE entry point for the Windows CLI integration test. */
typedef unsigned long DWORD;
__declspec(dllimport) void * __stdcall GetStdHandle(DWORD);
__declspec(dllimport) int __stdcall WriteFile(void *, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) long __stdcall XGameRuntimeInitialize(void);
DWORD mainCRTStartup(void *unused) {
    static const char text[] = "VIctory synthetic entry executed\r\n";
    static const char hle_ok[] = "XGameRuntimeInitialize returned S_OK\r\n";
    DWORD written = 0;
    (void)unused;
    if (XGameRuntimeInitialize() != 0) return 2;
    if (!WriteFile(GetStdHandle((DWORD)-11), hle_ok, sizeof(hle_ok)-1, &written, 0)
        || written != sizeof(hle_ok)-1) return 3;
    written = 0;
    return WriteFile(GetStdHandle((DWORD)-11), text, sizeof(text)-1, &written, 0)
        && written == sizeof(text)-1 ? 42 : 1;
}
