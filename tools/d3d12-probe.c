/* Synthetic D3D12 capability diagnostic; no game data is used. */
#define COBJMACROS
#include <windows.h>
#include <dxgi1_4.h>
#include <d3d12.h>
#include <stdio.h>

int main(void) {
    IDXGIFactory1 *factory = NULL;
    HRESULT hr = CreateDXGIFactory1(&IID_IDXGIFactory1, (void **)&factory);
    printf("CreateDXGIFactory1: 0x%08lx\n", (unsigned long)hr);
    if (SUCCEEDED(hr)) {
        for (UINT i = 0;; ++i) {
            IDXGIAdapter1 *adapter = NULL;
            hr = IDXGIFactory1_EnumAdapters1(factory, i, &adapter);
            if (hr == DXGI_ERROR_NOT_FOUND) break;
            if (FAILED(hr)) {
                printf("EnumAdapters1(%u): 0x%08lx\n", i, (unsigned long)hr);
                break;
            }
            DXGI_ADAPTER_DESC1 desc;
            hr = IDXGIAdapter1_GetDesc1(adapter, &desc);
            if (SUCCEEDED(hr)) {
                char name[512];
                if (!WideCharToMultiByte(CP_UTF8, 0, desc.Description, -1,
                                         name, sizeof(name), NULL, NULL)) {
                    name[0] = '\0';
                }
                printf("adapter %u: %s, vendor 0x%04x, device 0x%04x, flags 0x%x\n",
                       i, name, desc.VendorId, desc.DeviceId, desc.Flags);
            } else {
                printf("GetDesc1(%u): 0x%08lx\n", i, (unsigned long)hr);
            }
            ID3D12Device *device = NULL;
            hr = D3D12CreateDevice((IUnknown *)adapter, D3D_FEATURE_LEVEL_11_0,
                                   &IID_ID3D12Device, (void **)&device);
            printf("D3D12CreateDevice(adapter %u, 11_0): 0x%08lx\n", i, (unsigned long)hr);
            if (device) ID3D12Device_Release(device);
            IDXGIAdapter1_Release(adapter);
        }
        IDXGIFactory1_Release(factory);
    }

    ID3D12Device *device = NULL;
    hr = D3D12CreateDevice(NULL, D3D_FEATURE_LEVEL_11_0, &IID_ID3D12Device,
                           (void **)&device);
    printf("D3D12CreateDevice(default, 11_0): 0x%08lx\n", (unsigned long)hr);
    if (FAILED(hr)) return 1;

    D3D12_FEATURE_DATA_SHADER_MODEL shader = { D3D_SHADER_MODEL_6_6 };
    hr = ID3D12Device_CheckFeatureSupport(device, D3D12_FEATURE_SHADER_MODEL,
                                          &shader, sizeof(shader));
    printf("Shader model 6.6 query: 0x%08lx", (unsigned long)hr);
    if (SUCCEEDED(hr)) printf("; highest=0x%x", shader.HighestShaderModel);
    putchar('\n');

    D3D12_FEATURE_DATA_D3D12_OPTIONS7 options = {0};
    hr = ID3D12Device_CheckFeatureSupport(device, D3D12_FEATURE_D3D12_OPTIONS7,
                                          &options, sizeof(options));
    printf("Options7 query: 0x%08lx", (unsigned long)hr);
    if (SUCCEEDED(hr)) printf("; mesh shader tier=%u", options.MeshShaderTier);
    putchar('\n');
    ID3D12Device_Release(device);
    return 0;
}
