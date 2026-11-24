/* Experimental private Wine-to-host transport, not a graphics API shim. */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdint.h>

#define BRIDGE_MAGIC UINT32_C(0x31424d56)
#define BRIDGE_VERSION_OP UINT32_C(1)
#define MAX_PAYLOAD (1024u * 1024u)

static SRWLOCK bridge_lock = SRWLOCK_INIT;
/* A truncated or invalid frame leaves the stream unusable for future calls. */
static int bridge_broken;

static void put_u32(unsigned char *p, uint32_t value)
{
    p[0] = (unsigned char)value;
    p[1] = (unsigned char)(value >> 8);
    p[2] = (unsigned char)(value >> 16);
    p[3] = (unsigned char)(value >> 24);
}

static uint32_t get_u32(const unsigned char *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static int write_exact(HANDLE handle, const unsigned char *data, DWORD length)
{
    while (length != 0) {
        DWORD written = 0;
        if (!WriteFile(handle, data, length, &written, NULL) || written == 0 ||
            written > length)
            return 0;
        data += written;
        length -= written;
    }
    return 1;
}

static int read_exact(HANDLE handle, unsigned char *data, DWORD length)
{
    while (length != 0) {
        DWORD received = 0;
        if (!ReadFile(handle, data, length, &received, NULL) || received == 0 ||
            received > length)
            return 0;
        data += received;
        length -= received;
    }
    return 1;
}

__declspec(dllexport) int __stdcall VictoryMetalClearRgba8(
    unsigned width, unsigned height, unsigned pitch, unsigned char *pixels,
    unsigned len, unsigned color)
{
    unsigned char request[28], response[12];
    uint64_t row = (uint64_t)width * 4;
    uint64_t required;
    HANDLE input, output;
    int result = 1;

    if (width == 0 || height == 0 || pixels == NULL || len > MAX_PAYLOAD ||
        row > pitch)
        return 1;
    required = (uint64_t)pitch * (height - 1) + row;
    if (required > len)
        return 1;

    put_u32(request + 0, BRIDGE_MAGIC);
    put_u32(request + 4, BRIDGE_VERSION_OP);
    put_u32(request + 8, width);
    put_u32(request + 12, height);
    put_u32(request + 16, pitch);
    put_u32(request + 20, len);
    put_u32(request + 24, color);

    AcquireSRWLockExclusive(&bridge_lock);
    if (bridge_broken)
        goto done;
    input = GetStdHandle(STD_INPUT_HANDLE);
    output = GetStdHandle(STD_OUTPUT_HANDLE);
    if (input == NULL || input == INVALID_HANDLE_VALUE || output == NULL ||
        output == INVALID_HANDLE_VALUE ||
        !write_exact(output, request, sizeof(request)) ||
        !write_exact(output, pixels, len) ||
        !read_exact(input, response, sizeof(response)))
        goto broken;
    if (get_u32(response) != BRIDGE_MAGIC)
        goto broken;
    if (get_u32(response + 4) != 0) {
        if (get_u32(response + 8) != 0)
            goto broken;
        goto done;
    }
    if (get_u32(response + 8) != len || !read_exact(input, pixels, len))
        goto broken;
    result = 0;
    goto done;

broken:
    bridge_broken = 1;
done:
    ReleaseSRWLockExclusive(&bridge_lock);
    return result;
}
