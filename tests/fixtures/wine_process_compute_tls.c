/* A separate normal Wine PE using the synthetic GPU compute workload. */
#define mainCRTStartup compute_entry
#include "windows_d3d12x_compute.c"
#undef mainCRTStartup
#define WINE_PROCESS_TLS_NO_MAIN
#include "wine_process_tls.c"

int main(void) {
    if (check_dll()) return 20;
    if (check_tls()) return 21;
    return (int)compute_entry(NULL);
}
