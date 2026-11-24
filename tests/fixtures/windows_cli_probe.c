/* Self-authored PE entry point for the Windows CLI integration test. */
typedef unsigned long DWORD;
__declspec(dllimport) void * __stdcall GetStdHandle(DWORD);
__declspec(dllimport) int __stdcall WriteFile(void *, const void *, DWORD, DWORD *, void *);
DWORD mainCRTStartup(void *unused) {
    static const char text[] = "VIctory synthetic entry executed\r\n";
    DWORD written = 0;
    (void)unused;
    return WriteFile(GetStdHandle((DWORD)-11), text, sizeof(text)-1, &written, 0)
        && written == sizeof(text)-1 ? 42 : 1;
}
