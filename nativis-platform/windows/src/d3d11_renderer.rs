use anyhow::{bail, Result};

#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use tracing::info;

#[cfg(windows)]
use windows::{
    core::{Interface, PCSTR},
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{
            Direct3D::{
                Fxc::{D3DCompile, D3DCOMPILE_ENABLE_STRICTNESS, D3DCOMPILE_OPTIMIZATION_LEVEL3},
                ID3DBlob, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_11_1, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
                D3D11_SRV_DIMENSION_TEXTURE2D,
            },
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
                ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState,
                ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
                D3D11_BIND_SHADER_RESOURCE, D3D11_COMPARISON_NEVER,
                D3D11_CPU_ACCESS_WRITE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_FLOAT32_MAX, D3D11_MAP_WRITE_DISCARD,
                D3D11_MAPPED_SUBRESOURCE, D3D11_SAMPLER_DESC, D3D11_SDK_VERSION,
                D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_TEXTURE2D_DESC,
                D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC,
                D3D11_VIEWPORT,
            },
            Dxgi::{
                Common::{
                    DXGI_ALPHA_MODE_UNSPECIFIED, DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_FORMAT_NV12, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8_UNORM,
                    DXGI_FORMAT_R8_UNORM, DXGI_SAMPLE_DESC,
                },
                IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
                DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
                DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
        },
    },
};

#[cfg(windows)]
const VS_SOURCE: &[u8] = b"
struct VSOutput {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

VSOutput main(uint vertex_id : SV_VertexID) {
    VSOutput output;
    output.uv = float2((vertex_id << 1) & 2, vertex_id & 2);
    output.pos = float4(output.uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return output;
}
\0";

#[cfg(windows)]
const PS_NV12_SOURCE: &[u8] = b"
Texture2D y_tex : register(t0);
Texture2D uv_tex : register(t1);
SamplerState sam : register(s0);

struct VSOutput {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

float4 main(VSOutput input) : SV_Target {
    float y = y_tex.Sample(sam, input.uv).r;
    float2 uv = uv_tex.Sample(sam, input.uv).rg - float2(0.5, 0.5);

    // BT.709 color conversion matrix
    float r = y + 1.5748 * uv.y;
    float g = y - 0.1873 * uv.x - 0.4681 * uv.y;
    float b = y + 1.8556 * uv.x;

    return float4(saturate(float3(r, g, b)), 1.0);
}
\0";

#[cfg(windows)]
const PS_RGBA_SOURCE: &[u8] = b"
Texture2D tex : register(t0);
SamplerState sam : register(s0);

struct VSOutput {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

float4 main(VSOutput input) : SV_Target {
    return tex.Sample(sam, input.uv);
}
\0";

#[cfg(windows)]
unsafe fn compile_shader(source: &[u8], entry_point: &str, target: &str) -> Result<ID3DBlob> {
    let mut code_blob: Option<ID3DBlob> = None;
    let mut error_blob: Option<ID3DBlob> = None;

    let entry_cstr = std::ffi::CString::new(entry_point)?;
    let target_cstr = std::ffi::CString::new(target)?;

    let hr = D3DCompile(
        source.as_ptr() as _,
        source.len().saturating_sub(1),
        PCSTR::null(),
        None,
        None,
        PCSTR::from_raw(entry_cstr.as_ptr() as _),
        PCSTR::from_raw(target_cstr.as_ptr() as _),
        D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_OPTIMIZATION_LEVEL3,
        0,
        &mut code_blob,
        Some(&mut error_blob),
    );

    if let Err(e) = hr {
        if let Some(err) = error_blob {
            let msg = std::slice::from_raw_parts(err.GetBufferPointer() as *const u8, err.GetBufferSize());
            let err_str = String::from_utf8_lossy(msg);
            bail!("Shader compilation failed ({}): {}", target, err_str);
        }
        return Err(e.into());
    }

    Ok(code_blob.context("No shader bytecode produced")?)
}

#[cfg(windows)]
pub struct D3D11Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain1,
    render_target_view: Option<ID3D11RenderTargetView>,
    sampler: ID3D11SamplerState,

    vertex_shader: ID3D11VertexShader,
    pixel_shader_nv12: ID3D11PixelShader,
    pixel_shader_rgba: ID3D11PixelShader,

    // NV12 dynamic textures
    y_texture: Option<ID3D11Texture2D>,
    y_srv: Option<ID3D11ShaderResourceView>,
    uv_texture: Option<ID3D11Texture2D>,
    uv_srv: Option<ID3D11ShaderResourceView>,
    last_video_w: u32,
    last_video_h: u32,

    // RGBA dynamic texture
    rgba_texture: Option<ID3D11Texture2D>,
    rgba_srv: Option<ID3D11ShaderResourceView>,
    last_rgba_w: u32,
    last_rgba_h: u32,

    // Zero-Copy NV12 GPU texture & SRVs
    nv12_gpu_texture: Option<ID3D11Texture2D>,
    nv12_gpu_y_srv: Option<ID3D11ShaderResourceView>,
    nv12_gpu_uv_srv: Option<ID3D11ShaderResourceView>,
    last_zero_copy_w: u32,
    last_zero_copy_h: u32,

    width: u32,
    height: u32,
}

#[cfg(windows)]
impl D3D11Renderer {
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        info!("Initializing Direct3D 11 renderer for HWND: 0x{:X} ({}x{})...", hwnd.0 as usize, width, height);

        let width = if width == 0 { 1920 } else { width };
        let height = if height == 0 { 1080 } else { height };

        unsafe {
            // 1. Create D3D11 Device & Immediate Context
            let mut device_opt: Option<ID3D11Device> = None;
            let mut context_opt: Option<ID3D11DeviceContext> = None;
            let mut feature_level = D3D_FEATURE_LEVEL_11_0;

            D3D11CreateDevice(
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device_opt),
                Some(&mut feature_level),
                Some(&mut context_opt),
            ).context("D3D11CreateDevice failed")?;

            let device = device_opt.context("D3D11Device was null")?;
            let context = context_opt.context("D3D11DeviceContext was null")?;

            // Enable multithread protection for concurrent decode and render threads
            if let Ok(multithread) = context.cast::<ID3D11Multithread>() {
                let _ = multithread.SetMultithreadProtected(true);
                info!("Direct3D 11 Multithread protection enabled on device context");
            }

            info!("Direct3D 11 device created successfully (Feature Level: {:?})", feature_level);

            // 2. Query DXGI Factory and create Flip Discard SwapChain
            let dxgi_device: IDXGIDevice = device.cast()?;
            let dxgi_adapter = dxgi_device.GetAdapter()?;
            let dxgi_factory: IDXGIFactory2 = dxgi_adapter.GetParent()?;

            let swap_chain_desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2, // Modern 2-buffer Flip Discard Ring
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
                Flags: 0,
            };

            let swap_chain = dxgi_factory.CreateSwapChainForHwnd(
                &device,
                hwnd,
                &swap_chain_desc,
                None,
                None,
            ).context("CreateSwapChainForHwnd failed")?;

            let _ = dxgi_factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);

            // 3. Create Render Target View for Back Buffer
            let back_buffer: ID3D11Texture2D = swap_chain.GetBuffer(0)?;
            let mut rtv_opt: Option<ID3D11RenderTargetView> = None;
            device.CreateRenderTargetView(&back_buffer, None, Some(&mut rtv_opt))?;
            let render_target_view = rtv_opt.context("CreateRenderTargetView failed")?;

            // 4. Set Initial Viewport
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: width as f32,
                Height: height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            context.RSSetViewports(Some(&[viewport]));

            // 5. Create Linear Clamp Sampler
            let sampler_desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                BorderColor: [0.0; 4],
                MinLOD: 0.0,
                MaxLOD: D3D11_FLOAT32_MAX,
            };
            let mut sampler_opt: Option<ID3D11SamplerState> = None;
            device.CreateSamplerState(&sampler_desc, Some(&mut sampler_opt))?;
            let sampler = sampler_opt.context("CreateSamplerState failed")?;

            // 6. Compile Shaders
            let vs_blob = compile_shader(VS_SOURCE, "main", "vs_5_0")?;
            let mut vs_opt: Option<ID3D11VertexShader> = None;
            device.CreateVertexShader(
                std::slice::from_raw_parts(vs_blob.GetBufferPointer() as _, vs_blob.GetBufferSize()),
                None,
                Some(&mut vs_opt),
            )?;
            let vertex_shader = vs_opt.context("CreateVertexShader failed")?;

            let ps_nv12_blob = compile_shader(PS_NV12_SOURCE, "main", "ps_5_0")?;
            let mut ps_nv12_opt: Option<ID3D11PixelShader> = None;
            device.CreatePixelShader(
                std::slice::from_raw_parts(ps_nv12_blob.GetBufferPointer() as _, ps_nv12_blob.GetBufferSize()),
                None,
                Some(&mut ps_nv12_opt),
            )?;
            let pixel_shader_nv12 = ps_nv12_opt.context("CreatePixelShader (NV12) failed")?;

            let ps_rgba_blob = compile_shader(PS_RGBA_SOURCE, "main", "ps_5_0")?;
            let mut ps_rgba_opt: Option<ID3D11PixelShader> = None;
            device.CreatePixelShader(
                std::slice::from_raw_parts(ps_rgba_blob.GetBufferPointer() as _, ps_rgba_blob.GetBufferSize()),
                None,
                Some(&mut ps_rgba_opt),
            )?;
            let pixel_shader_rgba = ps_rgba_opt.context("CreatePixelShader (RGBA) failed")?;

            info!("Direct3D 11 SwapChain (Flip Discard) & Shaders initialized successfully.");

            Ok(Self {
                device,
                context,
                swap_chain,
                render_target_view: Some(render_target_view),
                sampler,
                vertex_shader,
                pixel_shader_nv12,
                pixel_shader_rgba,
                y_texture: None,
                y_srv: None,
                uv_texture: None,
                uv_srv: None,
                last_video_w: 0,
                last_video_h: 0,
                rgba_texture: None,
                rgba_srv: None,
                last_rgba_w: 0,
                last_rgba_h: 0,
                nv12_gpu_texture: None,
                nv12_gpu_y_srv: None,
                nv12_gpu_uv_srv: None,
                last_zero_copy_w: 0,
                last_zero_copy_h: 0,
                width,
                height,
            })
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 || (self.width == width && self.height == height) {
            return Ok(());
        }

        self.width = width;
        self.height = height;

        unsafe {
            // Drop RTV before resizing swapchain buffers
            self.context.OMSetRenderTargets(None, None);
            self.render_target_view = None;

            self.swap_chain.ResizeBuffers(
                2,
                width,
                height,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;

            let back_buffer: ID3D11Texture2D = self.swap_chain.GetBuffer(0)?;
            let mut rtv_opt: Option<ID3D11RenderTargetView> = None;
            self.device.CreateRenderTargetView(&back_buffer, None, Some(&mut rtv_opt))?;
            self.render_target_view = Some(rtv_opt.context("Re-creating RenderTargetView failed")?);

            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: width as f32,
                Height: height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            self.context.RSSetViewports(Some(&[viewport]));
        }

        Ok(())
    }

    pub fn draw_nv12(
        &mut self,
        y_data: &[u8],
        y_stride: u32,
        y_w: u32,
        y_h: u32,
        uv_data: &[u8],
        uv_stride: u32,
        uv_w: u32,
        uv_h: u32,
    ) -> Result<()> {
        unsafe {
            // 1. Recreate Y & UV dynamic textures if dimensions changed
            if self.last_video_w != y_w || self.last_video_h != y_h || self.y_texture.is_none() {
                let y_desc = D3D11_TEXTURE2D_DESC {
                    Width: y_w,
                    Height: y_h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                    MiscFlags: 0,
                };
                let mut y_tex: Option<ID3D11Texture2D> = None;
                self.device.CreateTexture2D(&y_desc, None, Some(&mut y_tex))?;
                let y_texture = y_tex.unwrap();

                let mut y_srv: Option<ID3D11ShaderResourceView> = None;
                self.device.CreateShaderResourceView(&y_texture, None, Some(&mut y_srv))?;

                let uv_desc = D3D11_TEXTURE2D_DESC {
                    Width: uv_w,
                    Height: uv_h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8G8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                    MiscFlags: 0,
                };
                let mut uv_tex: Option<ID3D11Texture2D> = None;
                self.device.CreateTexture2D(&uv_desc, None, Some(&mut uv_tex))?;
                let uv_texture = uv_tex.unwrap();

                let mut uv_srv: Option<ID3D11ShaderResourceView> = None;
                self.device.CreateShaderResourceView(&uv_texture, None, Some(&mut uv_srv))?;

                self.y_texture = Some(y_texture);
                self.y_srv = y_srv;
                self.uv_texture = Some(uv_texture);
                self.uv_srv = uv_srv;
                self.last_video_w = y_w;
                self.last_video_h = y_h;
            }

            // 2. Upload Y plane data via Map/Unmap
            let y_tex = self.y_texture.as_ref().unwrap();
            let mut mapped_y = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(y_tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped_y))?;
            let dst_y = mapped_y.pData as *mut u8;
            for row in 0..y_h {
                let src_offset = (row * y_stride) as usize;
                let dst_offset = (row * mapped_y.RowPitch) as usize;
                if src_offset + (y_w as usize) <= y_data.len() {
                    std::ptr::copy_nonoverlapping(
                        y_data.as_ptr().add(src_offset),
                        dst_y.add(dst_offset),
                        y_w as usize,
                    );
                }
            }
            self.context.Unmap(y_tex, 0);

            // 3. Upload UV plane data via Map/Unmap
            let uv_tex = self.uv_texture.as_ref().unwrap();
            let mut mapped_uv = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(uv_tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped_uv))?;
            let dst_uv = mapped_uv.pData as *mut u8;
            let uv_row_bytes = (uv_w * 2) as usize;
            for row in 0..uv_h {
                let src_offset = (row * uv_stride) as usize;
                let dst_offset = (row * mapped_uv.RowPitch) as usize;
                if src_offset + uv_row_bytes <= uv_data.len() {
                    std::ptr::copy_nonoverlapping(
                        uv_data.as_ptr().add(src_offset),
                        dst_uv.add(dst_offset),
                        uv_row_bytes,
                    );
                }
            }
            self.context.Unmap(uv_tex, 0);

            // 4. Render to back buffer
            self.context.OMSetRenderTargets(Some(&[self.render_target_view.clone()]), None);
            self.context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader_nv12, None);
            self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.PSSetShaderResources(0, Some(&[self.y_srv.clone(), self.uv_srv.clone()]));
            self.context.Draw(3, 0);
        }

        Ok(())
    }

    pub fn draw_rgba(&mut self, data: &[u8], width: u32, height: u32) -> Result<()> {
        unsafe {
            if self.last_rgba_w != width || self.last_rgba_h != height || self.rgba_texture.is_none() {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: width,
                    Height: height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                    MiscFlags: 0,
                };
                let mut tex: Option<ID3D11Texture2D> = None;
                self.device.CreateTexture2D(&desc, None, Some(&mut tex))?;
                let texture = tex.unwrap();

                let mut srv: Option<ID3D11ShaderResourceView> = None;
                self.device.CreateShaderResourceView(&texture, None, Some(&mut srv))?;

                self.rgba_texture = Some(texture);
                self.rgba_srv = srv;
                self.last_rgba_w = width;
                self.last_rgba_h = height;
            }

            let tex = self.rgba_texture.as_ref().unwrap();
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            let dst = mapped.pData as *mut u8;
            let row_bytes = (width * 4) as usize;
            for row in 0..height {
                let src_offset = (row as usize) * row_bytes;
                let dst_offset = (row as usize) * (mapped.RowPitch as usize);
                if src_offset + row_bytes <= data.len() {
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr().add(src_offset),
                        dst.add(dst_offset),
                        row_bytes,
                    );
                }
            }
            self.context.Unmap(tex, 0);

            self.context.OMSetRenderTargets(Some(&[self.render_target_view.clone()]), None);
            self.context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader_rgba, None);
            self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.PSSetShaderResources(0, Some(&[self.rgba_srv.clone()]));
            self.context.Draw(3, 0);
        }

        Ok(())
    }

    pub fn device(&self) -> ID3D11Device {
        self.device.clone()
    }

    pub fn context(&self) -> ID3D11DeviceContext {
        self.context.clone()
    }

    pub fn draw_d3d11_texture(
        &mut self,
        src_texture: &ID3D11Texture2D,
        subresource_index: u32,
        width: u32,
        height: u32,
    ) -> Result<()> {
        unsafe {
            // 1. Recreate NV12 staging texture and SRVs if video size changed
            if self.last_zero_copy_w != width || self.last_zero_copy_h != height || self.nv12_gpu_texture.is_none() {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: width,
                    Height: height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_NV12,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                };
                let mut tex: Option<ID3D11Texture2D> = None;
                self.device.CreateTexture2D(&desc, None, Some(&mut tex))?;
                let texture = tex.unwrap();

                let mut y_srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC::default();
                y_srv_desc.Format = DXGI_FORMAT_R8_UNORM;
                y_srv_desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
                y_srv_desc.Anonymous.Texture2D.MostDetailedMip = 0;
                y_srv_desc.Anonymous.Texture2D.MipLevels = 1;
                let mut y_srv: Option<ID3D11ShaderResourceView> = None;
                self.device.CreateShaderResourceView(&texture, Some(&y_srv_desc), Some(&mut y_srv))?;

                let mut uv_srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC::default();
                uv_srv_desc.Format = DXGI_FORMAT_R8G8_UNORM;
                uv_srv_desc.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
                uv_srv_desc.Anonymous.Texture2D.MostDetailedMip = 0;
                uv_srv_desc.Anonymous.Texture2D.MipLevels = 1;
                let mut uv_srv: Option<ID3D11ShaderResourceView> = None;
                self.device.CreateShaderResourceView(&texture, Some(&uv_srv_desc), Some(&mut uv_srv))?;

                self.nv12_gpu_texture = Some(texture);
                self.nv12_gpu_y_srv = y_srv;
                self.nv12_gpu_uv_srv = uv_srv;
                self.last_zero_copy_w = width;
                self.last_zero_copy_h = height;
            }

            // 2. Hardware GPU DMA copy (VRAM to VRAM) from decoder slice to our texture
            let dst_tex = self.nv12_gpu_texture.as_ref().unwrap();
            self.context.CopySubresourceRegion(
                dst_tex,
                0,
                0, 0, 0,
                src_texture,
                subresource_index,
                None,
            );

            // 3. Render NV12 using BT.709 pixel shader
            self.context.OMSetRenderTargets(Some(&[self.render_target_view.clone()]), None);
            self.context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader_nv12, None);
            self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.PSSetShaderResources(0, Some(&[self.nv12_gpu_y_srv.clone(), self.nv12_gpu_uv_srv.clone()]));
            self.context.Draw(3, 0);
        }
        Ok(())
    }

    pub fn present(&mut self) -> Result<()> {
        unsafe {
            // Present with 1 = sync to display refresh rate (vsync)
            self.swap_chain.Present(1, DXGI_PRESENT(0))
                .ok()
                .context("SwapChain Present(1, 0) failed")?;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
pub struct D3D11Renderer;

#[cfg(not(windows))]
impl D3D11Renderer {
    pub fn new(_hwnd: isize, _width: u32, _height: u32) -> Result<Self> {
        bail!("D3D11Renderer is only supported on Windows targets");
    }
}
