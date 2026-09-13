//! In-kernel HDL v0 software rasterizer (`raster_soft`).
//!
//! Interprets a validated frame into [`GpuDriver`] using embedded-graphics
//! (and a nearest-sample blit for images). Compiles for virt and D1.
//! Live path: D1 and the unpublished-mailbox kill-switch (`?hdl=0` / QEMU)
//! present the same scene list the host GPU would raster.

#![allow(dead_code)]

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_7X14, FONT_9X15_BOLD},
        MonoFont, MonoTextStyle,
    },
    pixelcolor::Rgb888,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle, RoundedRectangle},
    text::Text,
};

use crate::platform::d1_display::{self, GpuDriver};
use crate::ui::hdl::{self, Frame, Op};

static LOGO: &[u8] = include_bytes!("logo.raw");
const LOGO_W: u32 = 64;
const LOGO_H: u32 = 64;

static LOGO_SMALL: &[u8] = include_bytes!("logo_small.raw");
const LOGO_SMALL_W: u32 = 24;
const LOGO_SMALL_H: u32 = 24;

/// Resource-pack cursor (tex 2). Values match `GpuDriver::draw_cursor_bitmap`:
/// 0 transparent, 1 black, 2 white. 12×16.
const CURSOR_W: u32 = 12;
const CURSOR_H: u32 = 16;
const CURSOR: [u8; 12 * 16] = [
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 1, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 2,
    2, 1, 0, 0, 0, 0, 0, 0, 1, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 1, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0,
    1, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 1, 2, 2, 2, 2, 1, 1, 1,
    1, 1, 1, 0, 1, 2, 2, 1, 2, 1, 0, 0, 0, 0, 0, 0, 1, 2, 1, 0, 1, 2, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0,
    1, 2, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0,
];

#[derive(Clone, Copy)]
struct Clip {
    x: i32,
    y: i32,
    x2: i32,
    y2: i32,
}

impl Clip {
    fn from_wh(w: u16, h: u16) -> Self {
        Self {
            x: 0,
            y: 0,
            x2: w as i32,
            y2: h as i32,
        }
    }

    fn from_xywh(x: i16, y: i16, w: u16, h: u16) -> Self {
        Self {
            x: x as i32,
            y: y as i32,
            x2: x as i32 + w as i32,
            y2: y as i32 + h as i32,
        }
    }

    fn intersect(self, other: Self) -> Self {
        Self {
            x: self.x.max(other.x),
            y: self.y.max(other.y),
            x2: self.x2.min(other.x2),
            y2: self.y2.min(other.y2),
        }
    }

    fn is_empty(self) -> bool {
        self.x >= self.x2 || self.y >= self.y2
    }

    fn to_eg(self) -> Option<Rectangle> {
        if self.is_empty() {
            return None;
        }
        Some(Rectangle::new(
            Point::new(self.x, self.y),
            Size::new((self.x2 - self.x) as u32, (self.y2 - self.y) as u32),
        ))
    }
}

/// Decode `bytes` and raster. Unused on the live desktop (Phase 1 debug path).
pub fn raster_bytes(gpu: &mut GpuDriver, bytes: &[u8]) -> Result<(), hdl::Reject> {
    let frame = hdl::decode(bytes)?;
    raster(gpu, &frame);
    Ok(())
}

/// Paint a validated HDL frame into `gpu`. Painter order, src-over, integer
/// half-open clips. Does not flush the scanout.
pub fn raster(gpu: &mut GpuDriver, frame: &Frame<'_>) {
    let full = Clip::from_wh(frame.header.width, frame.header.height);
    let gpu_bounds = Clip {
        x: 0,
        y: 0,
        x2: gpu.width() as i32,
        y2: gpu.height() as i32,
    };
    let mut stack = [full; 8];
    let mut depth: usize = 0;

    d1_display::begin_pixel_batch();

    for op in frame.ops() {
        let clip = if depth == 0 {
            full.intersect(gpu_bounds)
        } else {
            stack[depth - 1].intersect(gpu_bounds)
        };

        match op {
            Op::Nop => {}
            Op::Clear { color } => {
                let (a, r, g, b) = hdl::unpack_aarrggbb(color);
                let _ = a;
                fill_sharp(gpu, full.intersect(gpu_bounds), r, g, b);
            }
            Op::FillRect {
                x,
                y,
                w,
                h,
                color,
                radius,
            } => {
                let (a, r, g, b) = hdl::unpack_aarrggbb(color);
                let _ = a;
                let rect = Clip::from_xywh(x, y, w, h).intersect(clip);
                if radius == 0 {
                    fill_sharp(gpu, rect, r, g, b);
                } else if let Some(area) = clip.to_eg() {
                    let mut target = gpu.clipped(&area);
                    let shape = RoundedRectangle::with_equal_corners(
                        Rectangle::new(
                            Point::new(x as i32, y as i32),
                            Size::new(w as u32, h as u32),
                        ),
                        Size::new(radius as u32, radius as u32),
                    );
                    let _ = shape
                        .into_styled(PrimitiveStyle::with_fill(Rgb888::new(r, g, b)))
                        .draw(&mut target);
                }
            }
            Op::Line {
                x0,
                y0,
                x1,
                y1,
                color,
                width,
            } => {
                if width == 0 {
                    continue;
                }
                let Some(area) = clip.to_eg() else { continue };
                let (_, r, g, b) = hdl::unpack_aarrggbb(color);
                let mut target = gpu.clipped(&area);
                let _ = Line::new(
                    Point::new(x0 as i32, y0 as i32),
                    Point::new(x1 as i32, y1 as i32),
                )
                .into_styled(PrimitiveStyle::with_stroke(
                    Rgb888::new(r, g, b),
                    width as u32,
                ))
                .draw(&mut target);
            }
            Op::GlyphRun {
                x,
                y,
                color,
                atlas_id,
                count,
                glyphs_off,
            } => {
                draw_glyphs(gpu, frame, clip, x, y, color, atlas_id, count, glyphs_off);
            }
            Op::Image {
                x,
                y,
                w,
                h,
                tex_id,
                flags: _,
                u0,
                v0,
                u1,
                v1,
            } => {
                blit_image(gpu, clip, x, y, w, h, tex_id, u0, v0, u1, v1);
            }
            Op::ClipPush { x, y, w, h } => {
                if depth >= 8 {
                    continue;
                }
                let parent = if depth == 0 { full } else { stack[depth - 1] };
                stack[depth] = parent.intersect(Clip::from_xywh(x, y, w, h));
                depth += 1;
            }
            Op::ClipPop => {
                if depth > 0 {
                    depth -= 1;
                }
            }
        }
    }

    d1_display::end_pixel_batch();
    d1_display::mark_all_dirty();
}

fn fill_sharp(gpu: &mut GpuDriver, rect: Clip, r: u8, g: u8, b: u8) {
    if rect.is_empty() {
        return;
    }
    let x = rect.x.max(0) as u32;
    let y = rect.y.max(0) as u32;
    let x2 = rect.x2.max(0) as u32;
    let y2 = rect.y2.max(0) as u32;
    if x2 <= x || y2 <= y {
        return;
    }
    gpu.fill_rect(x, y, x2 - x, y2 - y, r, g, b);
}

fn atlas(id: u16) -> &'static MonoFont<'static> {
    if id == 1 {
        &FONT_9X15_BOLD
    } else {
        &FONT_7X14
    }
}

fn draw_glyphs(
    gpu: &mut GpuDriver,
    frame: &Frame<'_>,
    clip: Clip,
    x: i16,
    y: i16,
    color: u32,
    atlas_id: u16,
    count: u16,
    glyphs_off: u32,
) {
    let Some(area) = clip.to_eg() else { return };
    let Some(ids) = frame.glyph_ids(glyphs_off, count) else {
        return;
    };
    let (a, r, g, b) = hdl::unpack_aarrggbb(color);
    let font = atlas(atlas_id);
    let adv = font.character_size.width as i32 + font.character_spacing as i32;
    let style = MonoTextStyle::new(font, Rgb888::new(r, g, b));
    let mut pen_x = x as i32;
    let baseline_y = y as i32;

    if a == 0 {
        return;
    }

    if a == 255 {
        let mut target = gpu.clipped(&area);
        for gid in ids {
            draw_one_glyph(&mut target, &style, pen_x, baseline_y, gid);
            pen_x += adv;
        }
    } else {
        let mut blend = BlendTarget { gpu, alpha: a };
        let mut target = blend.clipped(&area);
        for gid in ids {
            draw_one_glyph(&mut target, &style, pen_x, baseline_y, gid);
            pen_x += adv;
        }
    }
}

fn draw_one_glyph<D>(target: &mut D, style: &MonoTextStyle<'_, Rgb888>, x: i32, y: i32, gid: u16)
where
    D: DrawTarget<Color = Rgb888>,
{
    let ch = char::from_u32(0x20 + gid as u32).unwrap_or(' ');
    let mut tmp = [0u8; 4];
    let s = ch.encode_utf8(&mut tmp);
    let _ = Text::new(s, Point::new(x, y), *style).draw(target);
}

fn blit_image(
    gpu: &mut GpuDriver,
    clip: Clip,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
    tex_id: u16,
    u0: u16,
    v0: u16,
    u1: u16,
    v1: u16,
) {
    if w == 0 || h == 0 || clip.is_empty() {
        return;
    }
    let dest = Clip::from_xywh(x, y, w, h).intersect(clip);
    if dest.is_empty() {
        return;
    }

    let (tex_w, tex_h, kind) = match tex_id {
        0 => (LOGO_W, LOGO_H, Tex::Logo),
        1 => (LOGO_SMALL_W, LOGO_SMALL_H, Tex::LogoSmall),
        _ => (CURSOR_W, CURSOR_H, Tex::Cursor),
    };
    if tex_w == 0 || tex_h == 0 {
        return;
    }

    let origin_x = x as i32;
    let origin_y = y as i32;
    let dw = w as i32;
    let dh = h as i32;

    for py in dest.y..dest.y2 {
        for px in dest.x..dest.x2 {
            let dx = px - origin_x;
            let dy = py - origin_y;
            if dx < 0 || dy < 0 || dx >= dw || dy >= dh {
                continue;
            }
            let u = lerp_unorm(u0, u1, dx as u32, w as u32);
            let v = lerp_unorm(v0, v1, dy as u32, h as u32);
            let sx = unorm_to_texel(u, tex_w);
            let sy = unorm_to_texel(v, tex_h);
            let (sr, sg, sb, sa) = sample(kind, sx, sy);
            src_over(gpu, px as u32, py as u32, sr, sg, sb, sa);
        }
    }
}

#[derive(Clone, Copy)]
enum Tex {
    Logo,
    LogoSmall,
    Cursor,
}

fn sample(kind: Tex, sx: u32, sy: u32) -> (u8, u8, u8, u8) {
    match kind {
        Tex::Logo => rgba_at(LOGO, LOGO_W, LOGO_H, sx, sy),
        Tex::LogoSmall => rgba_at(LOGO_SMALL, LOGO_SMALL_W, LOGO_SMALL_H, sx, sy),
        Tex::Cursor => {
            if sx >= CURSOR_W || sy >= CURSOR_H {
                return (0, 0, 0, 0);
            }
            let i = (sy * CURSOR_W + sx) as usize;
            match CURSOR.get(i).copied().unwrap_or(0) {
                1 => (0, 0, 0, 255),
                2 => (255, 255, 255, 255),
                _ => (0, 0, 0, 0),
            }
        }
    }
}

fn rgba_at(data: &[u8], w: u32, h: u32, sx: u32, sy: u32) -> (u8, u8, u8, u8) {
    if sx >= w || sy >= h {
        return (0, 0, 0, 0);
    }
    let i = ((sy * w + sx) * 4) as usize;
    match data.get(i..i + 4) {
        Some(p) => (p[0], p[1], p[2], p[3]),
        None => (0, 0, 0, 0),
    }
}

fn lerp_unorm(a: u16, b: u16, t: u32, dim: u32) -> u16 {
    if dim == 0 {
        return a;
    }
    let a = a as i32;
    let b = b as i32;
    let u = a + (b - a) * (t as i32) / (dim as i32);
    u.clamp(0, 65535) as u16
}

fn unorm_to_texel(u: u16, tex_dim: u32) -> u32 {
    if tex_dim == 0 {
        return 0;
    }
    let x = (u as u32 * tex_dim) >> 16;
    x.min(tex_dim - 1)
}

fn src_over(gpu: &mut GpuDriver, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
    if a == 0 {
        return;
    }
    if a == 255 {
        gpu.set_pixel(x, y, r, g, b);
        return;
    }
    let dst = gpu.get_pixel(x, y);
    let dr = dst as u8;
    let dg = (dst >> 8) as u8;
    let db = (dst >> 16) as u8;
    let ia = 255u16 - a as u16;
    let nr = (r as u16 * a as u16 + dr as u16 * ia + 127) / 255;
    let ng = (g as u16 * a as u16 + dg as u16 * ia + 127) / 255;
    let nb = (b as u16 * a as u16 + db as u16 * ia + 127) / 255;
    gpu.set_pixel(x, y, nr as u8, ng as u8, nb as u8);
}

struct BlendTarget<'a> {
    gpu: &'a mut GpuDriver,
    alpha: u8,
}

impl OriginDimensions for BlendTarget<'_> {
    fn size(&self) -> Size {
        Size::new(self.gpu.width(), self.gpu.height())
    }
}

impl DrawTarget for BlendTarget<'_> {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels {
            if coord.x >= 0 && coord.y >= 0 {
                src_over(
                    self.gpu,
                    coord.x as u32,
                    coord.y as u32,
                    color.r(),
                    color.g(),
                    color.b(),
                    self.alpha,
                );
            }
        }
        Ok(())
    }
}
