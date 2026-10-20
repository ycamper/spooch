//! DXGI Desktop Duplication capture, kept warm between captures.
//!
//! Each monitor's output is duplicated once at startup. DXGI keeps the
//! duplicated image current while we are not looking, so a capture only
//! has to pick up the latest frame (if the screen changed) and read back a
//! staging texture we already hold. That avoids GDI's per-capture screen
//! readback, which varied from 35 to 73 ms per monitor on a test machine.
//!
//! The default backend. It costs about 80 MB of committed memory for the
//! D3D device and the duplication, though the working set stays small.
//! Not handled yet: rotated outputs (frames arrive unrotated) and HDR
//! outputs (DuplicateOutput1 with a float format).

use std::time::Instant;

use super::capture::{CaptureBackend, CapturedFrame, Image};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::core::Interface;

use super::monitors::Monitor;

/// How long to wait for the very first frame of a fresh duplication.
const FIRST_FRAME_TIMEOUT_MS: u32 = 500;
/// Frames to try for a picture: duplication can be lost and recreated, or
/// offer frames that only moved the pointer.
const REFRESH_TRIES: usize = 4;

struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

struct Output {
    monitor: HMONITOR,
    gpu: usize,
    output: IDXGIOutput1,
    dup: Option<IDXGIOutputDuplication>,
    /// CPU-readable copy of the latest frame.
    staging: Option<ID3D11Texture2D>,
}

pub struct DxgiCapture {
    pub monitors: Vec<Monitor>,
    gpus: Vec<Gpu>,
    outputs: Vec<Output>,
}

impl DxgiCapture {
    pub fn new() -> windows::core::Result<Self> {
        let mut this = Self {
            monitors: Vec::new(),
            gpus: Vec::new(),
            outputs: Vec::new(),
        };
        this.rebuild()?;
        Ok(this)
    }

    /// Enumerate every adapter's desktop outputs and duplicate each one.
    /// Hybrid-GPU machines get one device per adapter that drives a display.
    fn rebuild(&mut self) -> windows::core::Result<()> {
        self.outputs.clear();
        self.gpus.clear();
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
        let mut a = 0;
        while let Ok(adapter) = unsafe { factory.EnumAdapters1(a) } {
            a += 1;
            let mut gpu = None;
            let mut o = 0;
            while let Ok(output) = unsafe { adapter.EnumOutputs(o) } {
                o += 1;
                let desc = unsafe { output.GetDesc()? };
                if !desc.AttachedToDesktop.as_bool() {
                    continue;
                }
                let gpu_index = match gpu {
                    Some(i) => i,
                    None => {
                        self.gpus.push(create_gpu(&adapter)?);
                        gpu = Some(self.gpus.len() - 1);
                        self.gpus.len() - 1
                    }
                };
                let mut out = Output {
                    monitor: desc.Monitor,
                    gpu: gpu_index,
                    output: output.cast()?,
                    dup: None,
                    staging: None,
                };
                // Failing here (for example on a secure desktop) is retried
                // on the next capture.
                let _ = duplicate(&self.gpus[gpu_index], &mut out);
                self.outputs.push(out);
            }
        }
        Ok(())
    }

    fn capture_monitor(&mut self, m: &Monitor) -> windows::core::Result<CapturedFrame> {
        let started = Instant::now();
        let index = match self.outputs.iter().position(|o| o.monitor == m.handle) {
            Some(i) => i,
            None => {
                // Displays changed since startup.
                self.rebuild()?;
                self.outputs
                    .iter()
                    .position(|o| o.monitor == m.handle)
                    .ok_or_else(|| {
                        windows::core::Error::new(
                            windows::Win32::Foundation::E_FAIL,
                            "no DXGI output for this monitor",
                        )
                    })?
            }
        };
        let gpu = &self.gpus[self.outputs[index].gpu];
        let out = &mut self.outputs[index];
        refresh(gpu, out)?;
        let staging = out
            .staging
            .as_ref()
            .expect("refresh leaves a staging texture");
        let image = read_staging(gpu, staging)?;
        Ok(CapturedFrame {
            display: m.display.clone(),
            image,
            elapsed: started.elapsed(),
        })
    }
}

fn create_gpu(adapter: &IDXGIAdapter1) -> windows::core::Result<Gpu> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    Ok(Gpu {
        device: device.expect("D3D11CreateDevice returned no device"),
        context: context.expect("D3D11CreateDevice returned no context"),
    })
}

fn duplicate(gpu: &Gpu, out: &mut Output) -> windows::core::Result<()> {
    out.staging = None;
    out.dup = None;
    out.dup = Some(unsafe { out.output.DuplicateOutput(&gpu.device)? });
    Ok(())
}

/// Bring the staging texture up to date with the screen.
///
/// If the screen changed since the last capture, DXGI has a frame pending
/// and we copy it. If not, the staging texture is already current. A frame
/// that only moved the pointer has no new image (and right after
/// duplicating, its texture can be black), so it's skipped. A lost
/// duplication (mode change, secure desktop, exclusive fullscreen) is
/// recreated.
fn refresh(gpu: &Gpu, out: &mut Output) -> windows::core::Result<()> {
    for _ in 0..REFRESH_TRIES {
        if out.dup.is_none() {
            duplicate(gpu, out)?;
        }
        let dup = out.dup.as_ref().expect("duplicated above");
        let timeout = if out.staging.is_some() {
            0
        } else {
            FIRST_FRAME_TIMEOUT_MS
        };
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        match unsafe { dup.AcquireNextFrame(timeout, &mut info, &mut resource) } {
            Ok(()) if info.LastPresentTime == 0 => {
                unsafe {
                    let _ = dup.ReleaseFrame();
                }
                if out.staging.is_some() {
                    return Ok(());
                }
            }
            Ok(()) => {
                let copied = resource
                    .ok_or_else(|| {
                        windows::core::Error::new(
                            windows::Win32::Foundation::E_FAIL,
                            "AcquireNextFrame returned no resource",
                        )
                    })
                    .and_then(|r| r.cast::<ID3D11Texture2D>())
                    .and_then(|texture| copy_to_staging(gpu, out, &texture));
                unsafe {
                    let _ = out.dup.as_ref().expect("duplicated above").ReleaseFrame();
                }
                return copied;
            }
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT && out.staging.is_some() => {
                return Ok(());
            }
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                out.dup = None;
                out.staging = None;
            }
            Err(e) => return Err(e),
        }
    }
    Err(windows::core::Error::new(
        DXGI_ERROR_ACCESS_LOST,
        "desktop duplication gave no picture",
    ))
}

fn copy_to_staging(
    gpu: &Gpu,
    out: &mut Output,
    texture: &ID3D11Texture2D,
) -> windows::core::Result<()> {
    if out.staging.is_none() {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        unsafe {
            gpu.device
                .CreateTexture2D(&desc, None, Some(&mut staging))?
        };
        out.staging = staging;
    }
    let staging = out.staging.as_ref().expect("created above");
    unsafe { gpu.context.CopyResource(staging, texture) };
    Ok(())
}

fn read_staging(gpu: &Gpu, staging: &ID3D11Texture2D) -> windows::core::Result<Image> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { staging.GetDesc(&mut desc) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        gpu.context
            .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?
    };
    let (w, h) = (desc.Width, desc.Height);
    let row_bytes = w as usize * 4;
    let pitch = mapped.RowPitch as usize;
    let mut data = Vec::with_capacity(row_bytes * h as usize);
    unsafe {
        let src = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * h as usize);
        for y in 0..h as usize {
            data.extend_from_slice(&src[y * pitch..y * pitch + row_bytes]);
        }
        gpu.context.Unmap(staging, 0);
    }
    Ok(Image {
        width: w,
        height: h,
        stride: row_bytes,
        data,
    })
}

impl CaptureBackend for DxgiCapture {
    fn name(&self) -> &'static str {
        "dxgi-desktop-duplication"
    }

    fn capture_all(&mut self) -> Result<Vec<CapturedFrame>, String> {
        let monitors = self.monitors.clone();
        monitors
            .iter()
            .map(|m| {
                self.capture_monitor(m)
                    .map_err(|e| format!("{}: {e}", m.display.name))
            })
            .collect()
    }
}
