/* Synthetic x86-64 probe. Each vector instruction runs in its own child. */
#include <cpuid.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/sysctl.h>
#include <sys/wait.h>
#include <unistd.h>

__attribute__((target("avx"), noinline))
static int test_avx(void) {
    uint32_t result[8];
    __asm__ volatile("vxorps %%ymm0, %%ymm0, %%ymm0\n\t"
                     "vmovups %%ymm0, %0\n\t"
                     "vzeroupper" : "=m"(result) : : "ymm0");
    for (int i = 0; i < 8; ++i) if (result[i] != 0) return 1;
    return 0;
}

__attribute__((target("avx2"), noinline))
static int test_avx2(void) {
    uint32_t result[8];
    __asm__ volatile("vpcmpeqd %%ymm0, %%ymm0, %%ymm0\n\t"
                     "vmovdqu %%ymm0, %0\n\t"
                     "vzeroupper" : "=m"(result) : : "ymm0");
    for (int i = 0; i < 8; ++i) if (result[i] != UINT32_MAX) return 1;
    return 0;
}

static int run_child(const char *name, int (*fn)(void)) {
    pid_t pid = fork();
    if (pid < 0) { perror("fork"); return 1; }
    if (!pid) { signal(SIGILL, SIG_DFL); _exit(fn()); }
    int status;
    if (waitpid(pid, &status, 0) < 0) { perror("waitpid"); return 1; }
    if (WIFSIGNALED(status)) {
        int sig = WTERMSIG(status);
        printf("%s: terminated by signal %d%s\n", name, sig,
               sig == SIGILL ? " (SIGILL)" : "");
        return sig != SIGILL;
    }
    if (WIFEXITED(status)) {
        int code = WEXITSTATUS(status);
        printf("%s: %s (exit %d)\n", name,
               code == 0 ? "executed and validated" : "failed", code);
        return code != 0;
    }
    printf("%s: unexpected child status\n", name);
    return 1;
}

int main(void) {
    int translated = 0;
    size_t size = sizeof(translated);
    if (sysctlbyname("sysctl.proc_translated", &translated, &size, NULL, 0)) {
        perror("sysctl.proc_translated");
        return 1;
    }
    unsigned a, b, c, d;
    if (__get_cpuid_max(0, NULL) < 7) {
        fprintf(stderr, "CPUID leaf 7 unavailable\n");
        return 1;
    }
    __cpuid(1, a, b, c, d);
    unsigned avx = (c >> 28) & 1, osxsave = (c >> 27) & 1;
    __cpuid_count(7, 0, a, b, c, d);
    printf("translated=%d CPUID: AVX=%u OSXSAVE=%u AVX2=%u\n",
           translated, avx, osxsave, (b >> 5) & 1);
    fflush(stdout);
    int failed = run_child("AVX vxorps ymm", test_avx);
    failed |= run_child("AVX2 vpcmpeqd ymm", test_avx2);
    return failed;
}
