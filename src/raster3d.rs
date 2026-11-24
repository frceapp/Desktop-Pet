use glam::{Mat4, Vec3};

#[derive(Clone)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 4]>,
}

#[derive(Clone)]
pub struct MipTexture {
    pub levels: Vec<MipLevel>,
}

impl MipTexture {
    /// Builds a mipmapped sRGB RGBA8 texture, setting RGB=255 on alpha=0 pixels before downscaling
    /// so transparent edges do not bleed black into strokes or outlines.
    pub fn from_rgba8(rgba: &[u8], mut width: u32, mut height: u32, max_base_size: u32) -> Self {
        let mut pixels: Vec<[u8; 4]> = rgba
            .chunks_exact(4)
            .map(|px| {
                if px[3] == 0 {
                    [255, 255, 255, 0]
                } else {
                    [px[0], px[1], px[2], px[3]]
                }
            })
            .collect();

        while width > max_base_size && width.is_multiple_of(2) && height.is_multiple_of(2) {
            pixels = downsample_2x(&pixels, width, height);
            width /= 2;
            height /= 2;
        }

        let mut levels = Vec::new();
        levels.push(MipLevel {
            width,
            height,
            pixels: pixels.clone(),
        });

        while width >= 32 && height >= 32 && width.is_multiple_of(2) && height.is_multiple_of(2) {
            pixels = downsample_2x(&pixels, width, height);
            width /= 2;
            height /= 2;
            levels.push(MipLevel {
                width,
                height,
                pixels: pixels.clone(),
            });
        }

        Self { levels }
    }

    #[inline]
    pub fn base_size(&self) -> (u32, u32) {
        let l = &self.levels[0];
        (l.width, l.height)
    }

    #[inline]
    pub fn sample_bilinear(&self, level_idx: usize, u: f32, v: f32) -> [f32; 4] {
        let lvl = &self.levels[level_idx.min(self.levels.len() - 1)];
        let w = lvl.width as i32;
        let h = lvl.height as i32;

        let uu = u.rem_euclid(1.0);
        let vv = v.rem_euclid(1.0);

        let fx = uu * w as f32 - 0.5;
        let fy = vv * h as f32 - 0.5;
        let x0 = fx.floor() as i32;
        let y0 = fy.floor() as i32;
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        let ix0 = x0.rem_euclid(w) as usize;
        let ix1 = (x0 + 1).rem_euclid(w) as usize;
        let iy0 = y0.rem_euclid(h) as usize;
        let iy1 = (y0 + 1).rem_euclid(h) as usize;

        let stride = w as usize;
        let p00 = lvl.pixels[iy0 * stride + ix0];
        let p10 = lvl.pixels[iy0 * stride + ix1];
        let p01 = lvl.pixels[iy1 * stride + ix0];
        let p11 = lvl.pixels[iy1 * stride + ix1];

        let w00 = (1.0 - tx) * (1.0 - ty) * (1.0 / 255.0);
        let w10 = tx * (1.0 - ty) * (1.0 / 255.0);
        let w01 = (1.0 - tx) * ty * (1.0 / 255.0);
        let w11 = tx * ty * (1.0 / 255.0);

        [
            p00[0] as f32 * w00 + p10[0] as f32 * w10 + p01[0] as f32 * w01 + p11[0] as f32 * w11,
            p00[1] as f32 * w00 + p10[1] as f32 * w10 + p01[1] as f32 * w01 + p11[1] as f32 * w11,
            p00[2] as f32 * w00 + p10[2] as f32 * w10 + p01[2] as f32 * w01 + p11[2] as f32 * w11,
            p00[3] as f32 * w00 + p10[3] as f32 * w10 + p01[3] as f32 * w01 + p11[3] as f32 * w11,
        ]
    }
}

fn downsample_2x(src: &[[u8; 4]], width: u32, height: u32) -> Vec<[u8; 4]> {
    let nw = (width / 2) as usize;
    let nh = (height / 2) as usize;
    let sw = width as usize;
    let mut dst = vec![[0u8; 4]; nw * nh];
    for y in 0..nh {
        let sy = y * 2;
        let row0 = sy * sw;
        let row1 = (sy + 1) * sw;
        let out_row = y * nw;
        for x in 0..nw {
            let sx = x * 2;
            let a = src[row0 + sx];
            let b = src[row0 + sx + 1];
            let c = src[row1 + sx];
            let d = src[row1 + sx + 1];
            dst[out_row + x] = [
                ((a[0] as u16 + b[0] as u16 + c[0] as u16 + d[0] as u16 + 2) >> 2) as u8,
                ((a[1] as u16 + b[1] as u16 + c[1] as u16 + d[1] as u16 + 2) >> 2) as u8,
                ((a[2] as u16 + b[2] as u16 + c[2] as u16 + d[2] as u16 + 2) >> 2) as u8,
                ((a[3] as u16 + b[3] as u16 + c[3] as u16 + d[3] as u16 + 2) >> 2) as u8,
            ];
        }
    }
    dst
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaMode3D {
    Opaque,
    Mask(f32),
    Blend,
}

/// 2x2 SSAA software unlit rasterizer outputting premultiplied BGRA.
pub struct RenderTarget3D {
    out_w: u32,
    out_h: u32,
    ssaa_w: u32,
    ssaa_h: u32,
    color: Vec<[u8; 4]>,
    depth: Vec<f32>,
    proj_buf: Vec<[f32; 3]>,
}

impl RenderTarget3D {
    pub fn new(out_w: u32, out_h: u32) -> Self {
        let mut rt = Self {
            out_w: 0,
            out_h: 0,
            ssaa_w: 0,
            ssaa_h: 0,
            color: Vec::new(),
            depth: Vec::new(),
            proj_buf: Vec::new(),
        };
        rt.ensure_size(out_w, out_h);
        rt
    }

    pub fn ensure_size(&mut self, out_w: u32, out_h: u32) {
        let w = out_w.max(1);
        let h = out_h.max(1);
        if self.out_w == w && self.out_h == h {
            return;
        }
        self.out_w = w;
        self.out_h = h;
        self.ssaa_w = w * 2;
        self.ssaa_h = h * 2;
        let len = (self.ssaa_w * self.ssaa_h) as usize;
        self.color.resize(len, [0; 4]);
        self.depth.resize(len, f32::INFINITY);
    }

    pub fn clear(&mut self) {
        self.color.fill([0, 0, 0, 0]);
        self.depth.fill(f32::INFINITY);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_mesh(
        &mut self,
        positions: &[[f32; 3]],
        uvs: &[[f32; 2]],
        indices: &[u32],
        pivot_mat: &Mat4,
        span: f32,
        base_color_srgb: [f32; 4],
        texture: Option<&MipTexture>,
        alpha_mode: AlphaMode3D,
        double_sided: bool,
    ) {
        if span <= 0.0 || positions.is_empty() || indices.len() < 3 {
            return;
        }
        let left = -span * 0.5;
        let top = span * 0.96;
        let inv_w = self.ssaa_w as f32 / span;
        let inv_h = self.ssaa_h as f32 / span;

        self.proj_buf.clear();
        self.proj_buf.reserve(positions.len());
        for p in positions {
            let wp = pivot_mat.transform_point3(Vec3::from_array(*p));
            self.proj_buf.push([
                (wp.x - left) * inv_w,
                (top - wp.y) * inv_h,
                10.0 - wp.z,
            ]);
        }

        let max_x = self.ssaa_w as i32 - 1;
        let max_y = self.ssaa_h as i32 - 1;
        let stride = self.ssaa_w as usize;

        let const_bgra = [
            (base_color_srgb[2] * base_color_srgb[3] * 255.0 + 0.5).clamp(0.0, 255.0) as u8,
            (base_color_srgb[1] * base_color_srgb[3] * 255.0 + 0.5).clamp(0.0, 255.0) as u8,
            (base_color_srgb[0] * base_color_srgb[3] * 255.0 + 0.5).clamp(0.0, 255.0) as u8,
            (base_color_srgb[3] * 255.0 + 0.5).clamp(0.0, 255.0) as u8,
        ];

        for tri in indices.chunks_exact(3) {
            let i0 = tri[0] as usize;
            let i1 = tri[1] as usize;
            let i2 = tri[2] as usize;

            let v0 = self.proj_buf[i0];
            let v1 = self.proj_buf[i1];
            let v2 = self.proj_buf[i2];

            if v0[2] < 0.1 || v1[2] < 0.1 || v2[2] < 0.1 || v0[2] > 100.0 || v1[2] > 100.0 || v2[2] > 100.0 {
                continue;
            }

            // In Y-down screen space, front-facing CCW 3D triangles have negative signed area.
            let area2 = (v1[0] - v0[0]) * (v2[1] - v0[1]) - (v1[1] - v0[1]) * (v2[0] - v0[0]);
            if !double_sided && area2 >= 0.0 {
                continue;
            }
            if area2.abs() < 1e-6 {
                continue;
            }

            let min_fx = v0[0].min(v1[0]).min(v2[0]);
            let max_fx = v0[0].max(v1[0]).max(v2[0]);
            let min_fy = v0[1].min(v1[1]).min(v2[1]);
            let max_fy = v0[1].max(v1[1]).max(v2[1]);

            let x0 = (min_fx.floor() as i32).max(0);
            let x1 = (max_fx.ceil() as i32).min(max_x);
            let y0 = (min_fy.floor() as i32).max(0);
            let y1 = (max_fy.ceil() as i32).min(max_y);
            if x0 > x1 || y0 > y1 {
                continue;
            }

            let inv_area = 1.0 / area2;
            let dw0_dx = (v1[1] - v2[1]) * inv_area;
            let dw0_dy = (v2[0] - v1[0]) * inv_area;
            let dw1_dx = (v2[1] - v0[1]) * inv_area;
            let dw1_dy = (v0[0] - v2[0]) * inv_area;
            let dw2_dx = (v0[1] - v1[1]) * inv_area;
            let dw2_dy = (v1[0] - v0[0]) * inv_area;

            let start_px = x0 as f32 + 0.5;
            let start_py = y0 as f32 + 0.5;
            let mut row_w0 = ((v2[0] - v1[0]) * (start_py - v1[1]) - (v2[1] - v1[1]) * (start_px - v1[0])) * inv_area;
            let mut row_w1 = ((v0[0] - v2[0]) * (start_py - v2[1]) - (v0[1] - v2[1]) * (start_px - v2[0])) * inv_area;
            let mut row_w2 = ((v1[0] - v0[0]) * (start_py - v0[1]) - (v1[1] - v0[1]) * (start_px - v0[0])) * inv_area;

            if let Some(tex) = texture {
                let uv0 = uvs[i0];
                let uv1 = uvs[i1];
                let uv2 = uvs[i2];
                let (tw, th) = tex.base_size();
                let e0_px = (v1[0] - v0[0]).hypot(v1[1] - v0[1]);
                let e1_px = (v2[0] - v1[0]).hypot(v2[1] - v1[1]);
                let e2_px = (v0[0] - v2[0]).hypot(v0[1] - v2[1]);
                let e0_uv = ((uv1[0] - uv0[0]) * tw as f32).hypot((uv1[1] - uv0[1]) * th as f32);
                let e1_uv = ((uv2[0] - uv1[0]) * tw as f32).hypot((uv2[1] - uv1[1]) * th as f32);
                let e2_uv = ((uv0[0] - uv2[0]) * tw as f32).hypot((uv0[1] - uv2[1]) * th as f32);
                let (max_px, max_uv) = if e0_px >= e1_px && e0_px >= e2_px {
                    (e0_px, e0_uv)
                } else if e1_px >= e2_px {
                    (e1_px, e1_uv)
                } else {
                    (e2_px, e2_uv)
                };
                let texels_per_px = if max_px > 0.25 {
                    (max_uv / max_px).max(1.0)
                } else {
                    4.0
                };
                let max_mip = (tex.levels.len() - 1).min(2);
                let mip = (texels_per_px.log2().max(0.0).floor() as usize).min(max_mip);

                for y in y0..=y1 {
                    let mut w0 = row_w0;
                    let mut w1 = row_w1;
                    let mut w2 = row_w2;
                    let row_off = y as usize * stride;
                    for x in x0..=x1 {
                        if w0 >= -1e-5 && w1 >= -1e-5 && w2 >= -1e-5 {
                            let z = w0 * v0[2] + w1 * v1[2] + w2 * v2[2];
                            let idx = row_off + x as usize;
                            if z < self.depth[idx] {
                                let u = w0 * uv0[0] + w1 * uv1[0] + w2 * uv2[0];
                                let v = w0 * uv0[1] + w1 * uv1[1] + w2 * uv2[1];
                                let s = tex.sample_bilinear(mip, u, v);
                                let a = s[3] * base_color_srgb[3];
                                match alpha_mode {
                                    AlphaMode3D::Mask(cutoff) => {
                                        if a >= cutoff {
                                            self.depth[idx] = z;
                                            let r = (s[0] * base_color_srgb[0] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                                            let g = (s[1] * base_color_srgb[1] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                                            let b = (s[2] * base_color_srgb[2] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                                            self.color[idx] = [b, g, r, 255];
                                        }
                                    }
                                    AlphaMode3D::Blend => {
                                        if a > 0.002 {
                                            let sr = s[0] * base_color_srgb[0] * a * 255.0;
                                            let sg = s[1] * base_color_srgb[1] * a * 255.0;
                                            let sb = s[2] * base_color_srgb[2] * a * 255.0;
                                            let sa = a * 255.0;
                                            let inv = 1.0 - a;
                                            let dst = self.color[idx];
                                            self.color[idx] = [
                                                (sb + dst[0] as f32 * inv + 0.5).clamp(0.0, 255.0) as u8,
                                                (sg + dst[1] as f32 * inv + 0.5).clamp(0.0, 255.0) as u8,
                                                (sr + dst[2] as f32 * inv + 0.5).clamp(0.0, 255.0) as u8,
                                                (sa + dst[3] as f32 * inv + 0.5).clamp(0.0, 255.0) as u8,
                                            ];
                                        }
                                    }
                                    AlphaMode3D::Opaque => {
                                        self.depth[idx] = z;
                                        let r = (s[0] * base_color_srgb[0] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                                        let g = (s[1] * base_color_srgb[1] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                                        let b = (s[2] * base_color_srgb[2] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                                        self.color[idx] = [b, g, r, 255];
                                    }
                                }
                            }
                        }
                        w0 += dw0_dx;
                        w1 += dw1_dx;
                        w2 += dw2_dx;
                    }
                    row_w0 += dw0_dy;
                    row_w1 += dw1_dy;
                    row_w2 += dw2_dy;
                }
            } else {
                for y in y0..=y1 {
                    let mut w0 = row_w0;
                    let mut w1 = row_w1;
                    let mut w2 = row_w2;
                    let row_off = y as usize * stride;
                    for x in x0..=x1 {
                        if w0 >= -1e-5 && w1 >= -1e-5 && w2 >= -1e-5 {
                            let z = w0 * v0[2] + w1 * v1[2] + w2 * v2[2];
                            let idx = row_off + x as usize;
                            if z < self.depth[idx] {
                                self.depth[idx] = z;
                                self.color[idx] = const_bgra;
                            }
                        }
                        w0 += dw0_dx;
                        w1 += dw1_dx;
                        w2 += dw2_dx;
                    }
                    row_w0 += dw0_dy;
                    row_w1 += dw1_dy;
                    row_w2 += dw2_dy;
                }
            }
        }
    }

    /// Downsamples the 2x2 SSAA premultiplied BGRA target and composites it into `dst_bgra`.
    pub fn downsample_2x_into(
        &self,
        dst_bgra: &mut [u8],
        dst_w: usize,
        dst_h: usize,
        dst_x: i32,
        dst_y: i32,
    ) {
        let ssaa_w = self.ssaa_w as usize;
        for y in 0..self.out_h as i32 {
            let py = dst_y + y;
            if py < 0 || py >= dst_h as i32 {
                continue;
            }
            let sy = (y as usize) * 2;
            let row0 = sy * ssaa_w;
            let row1 = (sy + 1) * ssaa_w;
            let dst_row = py as usize * dst_w * 4;
            for x in 0..self.out_w as i32 {
                let px = dst_x + x;
                if px < 0 || px >= dst_w as i32 {
                    continue;
                }
                let sx = (x as usize) * 2;
                let a = self.color[row0 + sx];
                let b = self.color[row0 + sx + 1];
                let c = self.color[row1 + sx];
                let d = self.color[row1 + sx + 1];

                let sa = ((a[3] as u16 + b[3] as u16 + c[3] as u16 + d[3] as u16 + 2) >> 2) as u8;
                if sa == 0 {
                    continue;
                }
                let sb = ((a[0] as u16 + b[0] as u16 + c[0] as u16 + d[0] as u16 + 2) >> 2) as u8;
                let sg = ((a[1] as u16 + b[1] as u16 + c[1] as u16 + d[1] as u16 + 2) >> 2) as u8;
                let sr = ((a[2] as u16 + b[2] as u16 + c[2] as u16 + d[2] as u16 + 2) >> 2) as u8;

                let off = dst_row + (px as usize) * 4;
                if sa == 255 || dst_bgra[off + 3] == 0 {
                    dst_bgra[off] = sb;
                    dst_bgra[off + 1] = sg;
                    dst_bgra[off + 2] = sr;
                    dst_bgra[off + 3] = sa;
                } else {
                    let inv = 255 - sa as u16;
                    dst_bgra[off] = sb.saturating_add(((dst_bgra[off] as u16 * inv + 127) / 255) as u8);
                    dst_bgra[off + 1] = sg.saturating_add(((dst_bgra[off + 1] as u16 * inv + 127) / 255) as u8);
                    dst_bgra[off + 2] = sr.saturating_add(((dst_bgra[off + 2] as u16 * inv + 127) / 255) as u8);
                    dst_bgra[off + 3] = sa.saturating_add(((dst_bgra[off + 3] as u16 * inv + 127) / 255) as u8);
                }
            }
        }
    }
}
