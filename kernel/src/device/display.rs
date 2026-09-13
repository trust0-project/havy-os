//! Display pixels go through `platform::d1_display::GpuDriver` (reserved
//! scanout + doorbell). This unused third path was deleted so there is one
//! guest present ABI.

/// Pixel format for framebuffer documentation.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Argb8888,
    Xrgb8888,
    Rgb565,
    Rgb888,
}
