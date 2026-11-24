use std::f32::consts::PI;

use tiny_skia::{
    FillRule, GradientStop, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, Point,
    RadialGradient, SpreadMode, Stroke, Transform,
};
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
};

#[derive(Debug, Clone)]
pub enum EffectRequest {
    BlackHole {
        x: i32,
        y: i32,
        size: u32,
        seconds: f32,
    },
    Crack {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        seconds: f32,
        dpi: f32,
    },
}

#[derive(Clone, Copy)]
struct BlackHolePixel {
    p_y: f32,
    r: f32,
    base_angle: f32,
    out_x: f32,
    out_y: f32,
    rim_smooth: f32,
    edge_alpha: f32,
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let denom = edge1 - edge0;
    if denom.abs() < 1e-6 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / denom).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn capture_screen_patch(x: i32, y: i32, w: u32, h: u32) -> Option<Vec<[u8; 4]>> {
    if w == 0 || h == 0 {
        return None;
    }
    unsafe {
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return None;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return None;
        }

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
        let hbmp = CreateDIBSection(
            mem_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );

        let mut pixels = None;
        if !hbmp.is_null() && !bits.is_null() {
            let old = SelectObject(mem_dc, hbmp);
            if BitBlt(mem_dc, 0, 0, w as i32, h as i32, screen_dc, x, y, SRCCOPY | CAPTUREBLT) != 0
            {
                let src = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                let out: Vec<[u8; 4]> = src
                    .chunks_exact(4)
                    .map(|bgra| [bgra[2], bgra[1], bgra[0], 255])
                    .collect();
                pixels = Some(out);
            }
            SelectObject(mem_dc, old);
        }

        if !hbmp.is_null() {
            DeleteObject(hbmp);
        }
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        pixels
    }
}

pub struct BlackHoleEffect {
    width: u32,
    height: u32,
    seconds: f32,
    screen: Option<Vec<[u8; 4]>>,
    table: Vec<BlackHolePixel>,
}

impl BlackHoleEffect {
    pub fn new(width: u32, height: u32, seconds: f32, screen: Option<Vec<[u8; 4]>>) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        let mut table = Vec::with_capacity((w * h) as usize);
        let inv_w = 2.0 / w as f32;
        let inv_h = 2.0 / h as f32;

        for py in 0..h {
            // p.y in [-1, 1] with +1 at top, matching WebGL blackhole.js
            let fy = 1.0 - (py as f32 + 0.5) * inv_h;
            for px in 0..w {
                let fx = (px as f32 + 0.5) * inv_w - 1.0;
                let r = (fx * fx + fy * fy).sqrt();
                let denom = r.max(0.0001);
                table.push(BlackHolePixel {
                    p_y: fy,
                    r,
                    base_angle: fy.atan2(fx),
                    out_x: fx / denom,
                    out_y: fy / denom,
                    rim_smooth: smoothstep(1.0, 0.7, r),
                    edge_alpha: smoothstep(1.0, 0.94, r),
                });
            }
        }

        Self {
            width: w,
            height: h,
            seconds,
            screen,
            table,
        }
    }

    #[inline]
    fn sample_screen(&self, screen: &[[u8; 4]], at_x: f32, at_y: f32) -> [f32; 3] {
        let u = (at_x * 0.5 + 0.5).clamp(0.0, 1.0);
        let v = (0.5 - at_y * 0.5).clamp(0.0, 1.0);

        let w = self.width as usize;
        let h = self.height as usize;
        let fx = u * (w as f32 - 1.0);
        let fy = v * (h as f32 - 1.0);
        let x0 = (fx.floor() as usize).min(w - 1);
        let y0 = (fy.floor() as usize).min(h - 1);
        let x1 = (x0 + 1).min(w - 1);
        let y1 = (y0 + 1).min(h - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        let p00 = screen[y0 * w + x0];
        let p10 = screen[y0 * w + x1];
        let p01 = screen[y1 * w + x0];
        let p11 = screen[y1 * w + x1];

        let w00 = (1.0 - tx) * (1.0 - ty) * (1.0 / 255.0);
        let w10 = tx * (1.0 - ty) * (1.0 / 255.0);
        let w01 = (1.0 - tx) * ty * (1.0 / 255.0);
        let w11 = tx * ty * (1.0 / 255.0);

        [
            p00[0] as f32 * w00 + p10[0] as f32 * w10 + p01[0] as f32 * w01 + p11[0] as f32 * w11,
            p00[1] as f32 * w00 + p10[1] as f32 * w10 + p01[1] as f32 * w01 + p11[1] as f32 * w11,
            p00[2] as f32 * w00 + p10[2] as f32 * w10 + p01[2] as f32 * w01 + p11[2] as f32 * w11,
        ]
    }

    pub fn render(&self, t: f32, out_bgra: &mut [u8]) -> bool {
        if t >= self.seconds {
            return false;
        }
        const GROW: f32 = 0.9;
        const SHRINK: f32 = 0.8;
        let open = (t / GROW).min((self.seconds - t) / SHRINK).clamp(0.0, 1.0);
        let power = open.powf(0.6);
        let horizon = 0.2 * power;
        let ring_denom = horizon * 0.3 + 0.0001;
        let has_screen = self.screen.is_some();

        for (px, dst) in self.table.iter().zip(out_bgra.chunks_exact_mut(4)) {
            if px.r > 1.01 {
                dst.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let reach = px.rim_smooth * power;
            let from = px.r + reach * (0.1 / (px.r + 0.1) + 0.02 * (px.r * 26.0 + t * 7.0).sin());
            let wind = reach * (1.4 / (px.r * 5.0 + 0.5));
            let angle = px.base_angle + wind;
            let mut at_x = angle.cos() * from;
            let at_y = angle.sin() * from;
            if (px.p_y * 31.0 + t * 23.0).sin() >= 0.93 {
                at_x += reach * 0.03;
            }

            let (cr, cg, cb) = if let Some(screen) = &self.screen {
                let split_x = px.out_x * reach * 0.03;
                let split_y = px.out_y * reach * 0.03;
                let c_r = self.sample_screen(screen, at_x + split_x, at_y + split_y)[0];
                let c_g = self.sample_screen(screen, at_x, at_y)[1];
                let c_b = self.sample_screen(screen, at_x - split_x, at_y - split_y)[2];
                (c_r, c_g, c_b)
            } else {
                (0.0, 0.0, 0.0)
            };

            let shade = smoothstep(horizon, horizon * 2.8, px.r);
            let ring_diff = (px.r - horizon * 1.45) / ring_denom;
            let ring = (-(ring_diff * ring_diff)).exp() * power;
            let flare = 0.65 + 0.35 * (angle * 3.0 - t * 9.0).sin();
            let rf = ring * flare;
            let glow_r = rf;
            let glow_g = (0.45 + 0.50 * ring) * rf;
            let glow_b = (0.10 + 0.70 * ring) * rf;

            let inside = if px.r >= horizon { 1.0 } else { 0.0 };
            let alpha = if has_screen {
                px.edge_alpha
            } else {
                (1.0 - shade).max(ring)
            };
            let has_f = if has_screen { 1.0 } else { 0.0 };
            let r = ((cr * has_f * shade * inside + glow_r * inside) * alpha * 255.0)
                .clamp(0.0, 255.0) as u8;
            let g = ((cg * has_f * shade * inside + glow_g * inside) * alpha * 255.0)
                .clamp(0.0, 255.0) as u8;
            let b = ((cb * has_f * shade * inside + glow_b * inside) * alpha * 255.0)
                .clamp(0.0, 255.0) as u8;
            let a = (alpha * 255.0).clamp(0.0, 255.0) as u8;

            dst[0] = b;
            dst[1] = g;
            dst[2] = r;
            dst[3] = a;
        }
        true
    }
}

pub enum CrackRenderResult {
    Updated,
    Unchanged,
    Finished,
}

pub struct CrackEffect {
    width: u32,
    height: u32,
    seconds: f32,
    dpi: f32,
    impact: [f32; 2],
    reach: f32,
    cracks: Vec<Vec<[f32; 2]>>,
    shards: Vec<[[f32; 2]; 3]>,
    pixmap: Pixmap,
    static_drawn: bool,
}

#[inline]
fn rand_range(min: f32, max: f32) -> f32 {
    min + fastrand::f32() * (max - min)
}

fn jagged(mut x: f32, mut y: f32, mut angle: f32, length: f32, dpi: f32) -> Vec<[f32; 2]> {
    let mut points = vec![[x, y]];
    let mut gone = 0.0;
    while gone < length {
        let stride = rand_range(14.0 * dpi, 34.0 * dpi);
        angle += rand_range(-0.32, 0.32);
        x += angle.cos() * stride;
        y -= angle.sin() * stride;
        gone += stride;
        points.push([x, y]);
    }
    points
}

impl CrackEffect {
    pub fn new(width: u32, height: u32, seconds: f32, dpi: f32) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        let wf = w as f32;
        let hf = h as f32;
        let impact = [wf * 0.5, hf];
        let reach = (wf * 0.5).min(hf);

        let mut cracks = Vec::new();
        let mut spokes = Vec::new();
        let count = 11;
        for i in 0..count {
            let angle = PI * ((i as f32 + rand_range(0.2, 0.8)) / count as f32);
            let len = reach * rand_range(0.55, 1.0) * (0.75 + 0.25 * angle.cos().abs());
            let spoke = jagged(impact[0], impact[1], angle, len, dpi);
            for at in (2..spoke.len().saturating_sub(1)).step_by(2) {
                if fastrand::f32() < 0.55 {
                    let sign = if fastrand::bool() { -1.0 } else { 1.0 };
                    let fork_angle = angle + rand_range(0.5, 1.1) * sign;
                    let fork_len = reach * rand_range(0.12, 0.3);
                    cracks.push(jagged(spoke[at][0], spoke[at][1], fork_angle, fork_len, dpi));
                }
            }
            cracks.push(spoke.clone());
            spokes.push(spoke);
        }

        for step in [1usize, 2, 4] {
            for i in 0..spokes.len().saturating_sub(1) {
                let a = spokes[i][step.min(spokes[i].len() - 1)];
                let b = spokes[i + 1][step.min(spokes[i + 1].len() - 1)];
                if fastrand::f32() < 0.8 {
                    let mid = [
                        (a[0] + b[0]) * 0.5 + rand_range(-6.0 * dpi, 6.0 * dpi),
                        (a[1] + b[1]) * 0.5 + rand_range(-6.0 * dpi, 6.0 * dpi),
                    ];
                    cracks.push(vec![a, mid, b]);
                }
            }
        }

        let shards = (0..9)
            .map(|_| {
                let angle = rand_range(0.15, PI - 0.15);
                let far = reach * rand_range(0.08, 0.4);
                let x = impact[0] + angle.cos() * far;
                let y = impact[1] - angle.sin() * far;
                [
                    [x, y],
                    [
                        x + rand_range(-22.0 * dpi, 22.0 * dpi),
                        y - rand_range(6.0 * dpi, 26.0 * dpi),
                    ],
                    [
                        x + rand_range(-22.0 * dpi, 22.0 * dpi),
                        y + rand_range(-8.0 * dpi, 14.0 * dpi),
                    ],
                ]
            })
            .collect();

        let pixmap = Pixmap::new(w, h).expect("crack pixmap");
        Self {
            width: w,
            height: h,
            seconds,
            dpi,
            impact,
            reach,
            cracks,
            shards,
            pixmap,
            static_drawn: false,
        }
    }

    pub fn render(&mut self, t: f32, out_bgra: &mut [u8]) -> CrackRenderResult {
        const SPREAD: f32 = 0.28;
        const FADE: f32 = 1.0;

        if t >= self.seconds {
            return CrackRenderResult::Finished;
        }
        let in_static_phase = t >= SPREAD && t <= self.seconds - FADE;
        if in_static_phase && self.static_drawn {
            return CrackRenderResult::Unchanged;
        }

        self.pixmap.fill(tiny_skia::Color::TRANSPARENT);
        let front = self.reach * 1.6 * (t / SPREAD).min(1.0);
        let alpha_mul = ((self.seconds - t) / FADE).clamp(0.0, 1.0);

        let mask = if front < self.reach * 1.59 {
            let mut m = Mask::new(self.width, self.height);
            if let Some(m_ref) = m.as_mut() {
                if let Some(clip_path) = PathBuilder::from_circle(self.impact[0], self.impact[1], front.max(0.5)) {
                    m_ref.fill_path(&clip_path, FillRule::Winding, true, Transform::identity());
                }
            }
            m
        } else {
            None
        };

        let mut shard_paint = Paint::default();
        shard_paint.set_color_rgba8(255, 255, 255, (0.16 * 255.0) as u8);
        shard_paint.anti_alias = true;
        for shard in &self.shards {
            let mut pb = PathBuilder::new();
            pb.move_to(shard[0][0], shard[0][1]);
            pb.line_to(shard[1][0], shard[1][1]);
            pb.line_to(shard[2][0], shard[2][1]);
            pb.close();
            if let Some(path) = pb.finish() {
                self.pixmap.fill_path(
                    &path,
                    &shard_paint,
                    FillRule::Winding,
                    Transform::identity(),
                    mask.as_ref(),
                );
            }
        }

        let layers = [
            ((0u8, 0u8, 0u8, (0.60 * 255.0) as u8), 2.6 * self.dpi, 0.0_f32),
            ((255u8, 255u8, 255u8, (0.92 * 255.0) as u8), 1.1 * self.dpi, 1.2 * self.dpi),
        ];

        for ((r, g, b, a), width, shift) in layers {
            let mut pb = PathBuilder::new();
            for crack in &self.cracks {
                if let Some(first) = crack.first() {
                    pb.move_to(first[0] + shift, first[1] - shift);
                    for p in &crack[1..] {
                        pb.line_to(p[0] + shift, p[1] - shift);
                    }
                }
            }
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.set_color_rgba8(r, g, b, a);
                paint.anti_alias = true;
                let stroke = Stroke {
                    width,
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..Default::default()
                };
                self.pixmap.stroke_path(
                    &path,
                    &paint,
                    &stroke,
                    Transform::identity(),
                    mask.as_ref(),
                );
            }
        }

        if t < 0.18 {
            let flash_a = (0.8 * (1.0 - t / 0.18) * 255.0).clamp(0.0, 255.0) as u8;
            if let Some(shader) = RadialGradient::new(
                Point::from_xy(self.impact[0], self.impact[1]),
                Point::from_xy(self.impact[0], self.impact[1]),
                (self.reach * 0.7).max(1.0),
                vec![
                    GradientStop::new(0.0, tiny_skia::Color::from_rgba8(255, 255, 255, flash_a)),
                    GradientStop::new(1.0, tiny_skia::Color::from_rgba8(255, 255, 255, 0)),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                let mut paint = Paint::default();
                paint.shader = shader;
                if let Some(rect) = tiny_skia::Rect::from_xywh(0.0, 0.0, self.width as f32, self.height as f32) {
                    self.pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                }
            }
        }

        // Convert premultiplied RGBA -> premultiplied BGRA with fade alpha_mul
        let am = (alpha_mul * 256.0).round() as u32;
        for (src, dst) in self.pixmap.data().chunks_exact(4).zip(out_bgra.chunks_exact_mut(4)) {
            if src[3] == 0 {
                dst.copy_from_slice(&[0, 0, 0, 0]);
            } else if am >= 256 {
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
                dst[3] = src[3];
            } else {
                dst[0] = ((src[2] as u32 * am) >> 8) as u8;
                dst[1] = ((src[1] as u32 * am) >> 8) as u8;
                dst[2] = ((src[0] as u32 * am) >> 8) as u8;
                dst[3] = ((src[3] as u32 * am) >> 8) as u8;
            }
        }

        if in_static_phase {
            self.static_drawn = true;
        }
        CrackRenderResult::Updated
    }
}
