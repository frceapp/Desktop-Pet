use std::collections::HashMap;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
    SelectObject, SetBkMode, SetTextColor, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CALCRECT,
    DT_NOPREFIX, DT_SINGLELINE, FF_SWISS, FW_BOLD, FW_NORMAL, HBITMAP, HDC, HFONT,
    OUT_DEFAULT_PRECIS, TRANSPARENT,
};

use crate::sprite_view::composite_rgba_onto_bgra;

const INK_R: u8 = 0x4a;
const INK_G: u8 = 0x3b;
const INK_B: u8 = 0x35;

#[derive(Clone)]
pub struct AlphaGlyphBitmap {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
}

pub struct TextRenderer {
    hdc: HDC,
    cache: HashMap<(String, u32, bool), AlphaGlyphBitmap>,
}

impl TextRenderer {
    pub fn new() -> Self {
        let hdc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        Self {
            hdc,
            cache: HashMap::new(),
        }
    }

    pub fn rasterize(&mut self, text: &str, font_px: u32) -> &AlphaGlyphBitmap {
        self.rasterize_weighted(text, font_px, true)
    }

    pub fn rasterize_weighted(
        &mut self,
        text: &str,
        font_px: u32,
        bold: bool,
    ) -> &AlphaGlyphBitmap {
        let key = (text.to_string(), font_px.max(6), bold);
        if !self.cache.contains_key(&key) {
            let bmp = unsafe { self.render_gdi(&key.0, key.1, key.2) };
            self.cache.insert(key.clone(), bmp);
        }
        &self.cache[&key]
    }

    unsafe fn render_gdi(&self, text: &str, font_px: u32, bold: bool) -> AlphaGlyphBitmap {
        if self.hdc.is_null() {
            return AlphaGlyphBitmap {
                width: 1,
                height: 1,
                alpha: vec![0],
            };
        }
        let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
        let weight = if bold { FW_BOLD } else { FW_NORMAL } as i32;
        let hfont: HFONT = CreateFontW(
            -(font_px as i32),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            (DEFAULT_PITCH | FF_SWISS) as u32,
            face.as_ptr(),
        );
        let old_font = SelectObject(self.hdc, hfont);

        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        DrawTextW(
            self.hdc,
            wide.as_ptr(),
            wide.len() as i32,
            &mut rc,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );

        let w = (rc.right - rc.left + 2).max(1) as u32;
        let h = (rc.bottom - rc.top + 2).max(1) as u32;

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };

        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbmp: HBITMAP = CreateDIBSection(
            self.hdc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );

        let mut alpha = vec![0u8; (w * h) as usize];
        if !hbmp.is_null() && !bits.is_null() {
            let old_bmp = SelectObject(self.hdc, hbmp);
            std::ptr::write_bytes(bits as *mut u8, 0, (w * h * 4) as usize);
            SetBkMode(self.hdc, TRANSPARENT as i32);
            SetTextColor(self.hdc, 0x00FF_FFFF);

            let mut draw_rc = RECT {
                left: 1,
                top: 0,
                right: w as i32,
                bottom: h as i32,
            };
            DrawTextW(
                self.hdc,
                wide.as_ptr(),
                wide.len() as i32,
                &mut draw_rc,
                DT_SINGLELINE | DT_NOPREFIX,
            );

            let src = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
            for (dst_a, px) in alpha.iter_mut().zip(src.chunks_exact(4)) {
                *dst_a = px[0].max(px[1]).max(px[2]);
            }
            SelectObject(self.hdc, old_bmp);
        }

        if !hbmp.is_null() {
            DeleteObject(hbmp);
        }
        SelectObject(self.hdc, old_font);
        if !hfont.is_null() {
            DeleteObject(hfont);
        }

        AlphaGlyphBitmap {
            width: w,
            height: h,
            alpha,
        }
    }
}

impl Drop for TextRenderer {
    fn drop(&mut self) {
        if !self.hdc.is_null() {
            unsafe {
                DeleteDC(self.hdc);
            }
        }
    }
}

pub fn draw_glyph_on_pixmap(
    pix: &mut Pixmap,
    glyph: &AlphaGlyphBitmap,
    x: i32,
    y: i32,
    color_rgba: [u8; 4],
) {
    let pw = pix.width() as i32;
    let ph = pix.height() as i32;
    let data = pix.data_mut();
    for gy in 0..glyph.height as i32 {
        let dy = y + gy;
        if dy < 0 || dy >= ph {
            continue;
        }
        let g_row = gy as usize * glyph.width as usize;
        let d_row = dy as usize * pw as usize * 4;
        for gx in 0..glyph.width as i32 {
            let dx = x + gx;
            if dx < 0 || dx >= pw {
                continue;
            }
            let ga = glyph.alpha[g_row + gx as usize];
            if ga == 0 {
                continue;
            }
            let eff_a = (ga as u32 * color_rgba[3] as u32 + 127) / 255;
            if eff_a == 0 {
                continue;
            }
            let sr = (color_rgba[0] as u32 * eff_a + 127) / 255;
            let sg = (color_rgba[1] as u32 * eff_a + 127) / 255;
            let sb = (color_rgba[2] as u32 * eff_a + 127) / 255;
            let inv = 255 - eff_a;
            let off = d_row + dx as usize * 4;
            data[off] = (sr + (data[off] as u32 * inv + 127) / 255).min(255) as u8;
            data[off + 1] = (sg + (data[off + 1] as u32 * inv + 127) / 255).min(255) as u8;
            data[off + 2] = (sb + (data[off + 2] as u32 * inv + 127) / 255).min(255) as u8;
            data[off + 3] = (eff_a + (data[off + 3] as u32 * inv + 127) / 255).min(255) as u8;
        }
    }
}

fn append_rounded_rect(pb: &mut PathBuilder, x: f32, y: f32, w: f32, h: f32, r: f32) {
    let r = r.min(w * 0.5).min(h * 0.5);
    let k = 0.5522848 * r;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
}

/// Evaluates an overshoot pop curve from 0.4 -> 1.0 over `u in [0, 1]`.
#[inline]
fn pop_scale_and_alpha(age_sec: f32) -> (f32, f32) {
    const DUR: f32 = 0.22;
    if age_sec >= DUR {
        return (1.0, 1.0);
    }
    let u = (age_sec / DUR).clamp(0.0, 1.0);
    // Smooth cubic overshoot peaking ~1.08 around u=0.68 before settling at 1.0
    let inv = 1.0 - u;
    let overshoot = 1.0 - inv * inv * inv + 1.35 * u * inv * inv;
    let scale = 0.4 + 0.6 * overshoot;
    let alpha = (u * 2.5).min(1.0);
    (scale, alpha)
}

#[allow(clippy::too_many_arguments)]
pub fn draw_bubble(
    text_renderer: &mut TextRenderer,
    dst_bgra: &mut [u8],
    win_w: usize,
    win_h: usize,
    pet_px: f32,
    dpi: f32,
    text: &str,
    age_sec: f32,
) {
    let font_px = (15.0 * dpi).round().max(9.0) as u32;
    let glyph = text_renderer.rasterize(text, font_px).clone();

    let border = (3.0 * dpi).max(1.5);
    let radius = 14.0 * dpi;
    let pad_x = 10.0 * dpi;
    let pad_top = 4.0 * dpi;
    let pad_bot = 5.0 * dpi;
    let tail_half = 8.5 * dpi; // half diagonal of 12px square rotated 45deg

    let box_w = glyph.width as f32 + pad_x * 2.0 + border * 2.0;
    let box_h = glyph.height as f32 + pad_top + pad_bot + border * 2.0;
    let canvas_w = (box_w + border * 2.0).ceil().min(win_w as f32 - 4.0).max(24.0) as u32;
    let canvas_h = (box_h + tail_half + border * 2.0).ceil().max(24.0) as u32;

    let Some(mut pix) = Pixmap::new(canvas_w, canvas_h) else {
        return;
    };

    let bx = ((canvas_w as f32 - box_w) * 0.5).max(border);
    let bw = (canvas_w as f32 - border * 2.0).min(box_w);
    let by = border;
    let bh = box_h;

    let tail_cx = canvas_w as f32 * 0.5;
    let tail_cy = by + bh - 1.5 * dpi;

    let mut rect_pb = PathBuilder::new();
    append_rounded_rect(&mut rect_pb, bx, by, bw, bh, radius);
    let rect_path = rect_pb.finish();

    let mut tail_pb = PathBuilder::new();
    tail_pb.move_to(tail_cx, tail_cy - tail_half);
    tail_pb.line_to(tail_cx + tail_half, tail_cy);
    tail_pb.line_to(tail_cx, tail_cy + tail_half);
    tail_pb.line_to(tail_cx - tail_half, tail_cy);
    tail_pb.close();
    let tail_path = tail_pb.finish();

    let mut stroke_paint = Paint::default();
    stroke_paint.set_color_rgba8(INK_R, INK_G, INK_B, 255);
    stroke_paint.anti_alias = true;

    let mut fill_paint = Paint::default();
    fill_paint.set_color_rgba8(255, 255, 255, 255);
    fill_paint.anti_alias = true;

    let stroke = Stroke {
        width: border,
        line_join: tiny_skia::LineJoin::Round,
        ..Default::default()
    };

    // Pass 1: stroke both rect and tail border; Pass 2: fill both white to hide internal seam.
    if let Some(p) = &rect_path {
        pix.stroke_path(p, &stroke_paint, &stroke, Transform::identity(), None);
    }
    if let Some(p) = &tail_path {
        pix.stroke_path(p, &stroke_paint, &stroke, Transform::identity(), None);
    }
    if let Some(p) = &rect_path {
        pix.fill_path(p, &fill_paint, FillRule::Winding, Transform::identity(), None);
    }
    if let Some(p) = &tail_path {
        pix.fill_path(p, &fill_paint, FillRule::Winding, Transform::identity(), None);
    }

    // Stamp text centered inside bubble box
    let text_x = (bx + (bw - glyph.width as f32) * 0.5).round() as i32;
    let text_y = (by + border + pad_top).round() as i32;
    let cw = canvas_w as i32;
    let ch = canvas_h as i32;
    let pix_data = pix.data_mut();
    for gy in 0..glyph.height as i32 {
        let dy = text_y + gy;
        if dy < 0 || dy >= ch {
            continue;
        }
        let g_row = gy as usize * glyph.width as usize;
        let p_row = dy as usize * canvas_w as usize * 4;
        for gx in 0..glyph.width as i32 {
            let dx = text_x + gx;
            if dx < 0 || dx >= cw {
                continue;
            }
            let ga = glyph.alpha[g_row + gx as usize];
            if ga == 0 {
                continue;
            }
            let off = p_row + dx as usize * 4;
            let a = ga as u16;
            let inv = 255 - a;
            pix_data[off] = ((INK_R as u16 * a + pix_data[off] as u16 * inv + 127) / 255) as u8;
            pix_data[off + 1] =
                ((INK_G as u16 * a + pix_data[off + 1] as u16 * inv + 127) / 255) as u8;
            pix_data[off + 2] =
                ((INK_B as u16 * a + pix_data[off + 2] as u16 * inv + 127) / 255) as u8;
            pix_data[off + 3] =
                (ga as u16 + ((pix_data[off + 3] as u16 * inv + 127) / 255)).min(255) as u8;
        }
    }

    let (pop_scale, pop_alpha) = pop_scale_and_alpha(age_sec);
    let anchor_x = win_w as f32 * 0.5;
    let bottom_y = win_h as f32 - pet_px - 10.0 * dpi;

    if (pop_scale - 1.0).abs() < 0.002 && pop_alpha >= 0.999 {
        let dx = (anchor_x - canvas_w as f32 * 0.5)
            .clamp(2.0, (win_w as f32 - canvas_w as f32 - 2.0).max(2.0))
            .round() as i32;
        let dy = (bottom_y - canvas_h as f32).max(2.0).round() as i32;
        composite_rgba_onto_bgra(
            pix.data(),
            canvas_w as usize,
            canvas_h as usize,
            dst_bgra,
            win_w,
            win_h,
            dx,
            dy,
        );
    } else {
        let scaled_w = ((canvas_w as f32 * pop_scale).ceil() as u32).max(1);
        let scaled_h = ((canvas_h as f32 * pop_scale).ceil() as u32).max(1);
        if let Some(mut scaled) = Pixmap::new(scaled_w, scaled_h) {
            let paint = tiny_skia::PixmapPaint {
                opacity: pop_alpha,
                blend_mode: tiny_skia::BlendMode::SourceOver,
                quality: tiny_skia::FilterQuality::Bilinear,
            };
            scaled.draw_pixmap(
                0,
                0,
                pix.as_ref(),
                &paint,
                Transform::from_scale(pop_scale, pop_scale),
                None,
            );
            let dx = (anchor_x - scaled_w as f32 * 0.5)
                .clamp(2.0, (win_w as f32 - scaled_w as f32 - 2.0).max(2.0))
                .round() as i32;
            let dy = (bottom_y - scaled_h as f32).max(2.0).round() as i32;
            composite_rgba_onto_bgra(
                scaled.data(),
                scaled_w as usize,
                scaled_h as usize,
                dst_bgra,
                win_w,
                win_h,
                dx,
                dy,
            );
        }
    }
}

pub fn draw_zzz(
    text_renderer: &mut TextRenderer,
    dst_bgra: &mut [u8],
    win_w: usize,
    win_h: usize,
    pet_px: f32,
    dpi: f32,
    sleep_time: f32,
) {
    let base_font = (pet_px * 0.14 + 4.0 * dpi).max(10.0);
    let hi_font = (base_font * 1.5).round() as u32;
    let glyph = text_renderer.rasterize("z", hi_font).clone();
    let gw = glyph.width as f32;
    let gh = glyph.height as f32;

    let origin_x = win_w as f32 * 0.5 + pet_px * 0.22;
    let origin_y = win_h as f32 - pet_px * 0.72;

    for delay in [0.0_f32, 1.0, 2.0] {
        let p = ((sleep_time - delay).rem_euclid(3.0)) / 3.0;
        let alpha = if p < 0.15 {
            p / 0.15
        } else if p > 0.80 {
            (1.0 - p) / 0.20
        } else {
            1.0
        };
        if alpha <= 0.01 {
            continue;
        }
        let scale = (0.6 + 0.9 * p) / 1.5;
        let cx = origin_x + pet_px * 0.22 * p;
        let cy = origin_y - pet_px * 0.40 * p;
        let dw = (gw * scale).ceil().max(1.0) as i32;
        let dh = (gh * scale).ceil().max(1.0) as i32;
        let x0 = cx.round() as i32;
        let y0 = (cy - dh as f32).round() as i32;

        for dy in 0..dh {
            let py = y0 + dy;
            if py < 0 || py >= win_h as i32 {
                continue;
            }
            let sy = ((dy as f32 + 0.5) / scale - 0.5).clamp(0.0, gh - 1.0);
            let iy0 = sy.floor() as usize;
            let iy1 = (iy0 + 1).min(glyph.height as usize - 1);
            let ty = sy - iy0 as f32;

            for dx in 0..dw {
                let px = x0 + dx;
                if px < 0 || px >= win_w as i32 {
                    continue;
                }
                let sx = ((dx as f32 + 0.5) / scale - 0.5).clamp(0.0, gw - 1.0);
                let ix0 = sx.floor() as usize;
                let ix1 = (ix0 + 1).min(glyph.width as usize - 1);
                let tx = sx - ix0 as f32;

                let a00 = glyph.alpha[iy0 * glyph.width as usize + ix0] as f32;
                let a10 = glyph.alpha[iy0 * glyph.width as usize + ix1] as f32;
                let a01 = glyph.alpha[iy1 * glyph.width as usize + ix0] as f32;
                let a11 = glyph.alpha[iy1 * glyph.width as usize + ix1] as f32;
                let ga = ((a00 * (1.0 - tx) + a10 * tx) * (1.0 - ty)
                    + (a01 * (1.0 - tx) + a11 * tx) * ty)
                    * alpha;
                let sa = ga.round().clamp(0.0, 255.0) as u8;
                if sa == 0 {
                    continue;
                }
                let af = sa as u16;
                let sb = ((INK_B as u16 * af + 127) / 255) as u8;
                let sg = ((INK_G as u16 * af + 127) / 255) as u8;
                let sr = ((INK_R as u16 * af + 127) / 255) as u8;

                let off = py as usize * win_w * 4 + px as usize * 4;
                if sa == 255 || dst_bgra[off + 3] == 0 {
                    dst_bgra[off] = sb;
                    dst_bgra[off + 1] = sg;
                    dst_bgra[off + 2] = sr;
                    dst_bgra[off + 3] = sa;
                } else {
                    let inv = 255 - af;
                    dst_bgra[off] = sb.saturating_add(((dst_bgra[off] as u16 * inv + 127) / 255) as u8);
                    dst_bgra[off + 1] =
                        sg.saturating_add(((dst_bgra[off + 1] as u16 * inv + 127) / 255) as u8);
                    dst_bgra[off + 2] =
                        sr.saturating_add(((dst_bgra[off + 2] as u16 * inv + 127) / 255) as u8);
                    dst_bgra[off + 3] =
                        sa.saturating_add(((dst_bgra[off + 3] as u16 * inv + 127) / 255) as u8);
                }
            }
        }
    }
}
