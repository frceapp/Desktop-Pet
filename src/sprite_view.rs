use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;

use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use tiny_skia::{BlendMode, FilterQuality, Pixmap, PixmapPaint, Transform};

use crate::config::SpritesConfig;

#[derive(Clone)]
struct SpriteFrame {
    delay_sec: f32,
    pixmap: Pixmap,
}

#[derive(Clone)]
struct SpriteAsset {
    frames: Vec<SpriteFrame>,
    total_sec: f32,
}

pub struct SpriteView {
    assets: HashMap<String, SpriteAsset>,
    state: String,
    sitting: bool,
    facing: i32,
    elapsed: f32,
    scratch: Option<Pixmap>,
}

impl SpriteView {
    pub fn load(char_dir: &Path, cfg: &SpritesConfig) -> Result<Self, String> {
        let mut entries: Vec<(&str, &str)> = vec![("idle", &cfg.idle)];
        if let Some(f) = &cfg.walk { entries.push(("walk", f)); }
        if let Some(f) = &cfg.talk { entries.push(("talk", f)); }
        if let Some(f) = &cfg.annoyed { entries.push(("annoyed", f)); }
        if let Some(f) = &cfg.cry { entries.push(("cry", f)); }
        if let Some(f) = &cfg.drag { entries.push(("drag", f)); }
        if let Some(f) = &cfg.sit { entries.push(("sit", f)); }
        if let Some(f) = &cfg.sleep { entries.push(("sleep", f)); }

        let mut file_cache: HashMap<String, SpriteAsset> = HashMap::new();
        let mut assets = HashMap::new();
        for (state, rel_file) in entries {
            if !file_cache.contains_key(rel_file) {
                let path = char_dir.join(rel_file);
                let asset = load_sprite_file(&path)?;
                file_cache.insert(rel_file.to_string(), asset);
            }
            if let Some(asset) = file_cache.get(rel_file) {
                assets.insert(state.to_string(), asset.clone());
            }
        }

        if !assets.contains_key("idle") {
            return Err("Missing idle sprite".to_string());
        }

        Ok(Self {
            assets,
            state: "idle".to_string(),
            sitting: false,
            facing: 1,
            elapsed: 0.0,
            scratch: None,
        })
    }

    #[inline]
    pub fn width_ratio(&self) -> f32 {
        0.8
    }

    #[inline]
    pub fn height_ratio(&self) -> f32 {
        0.95
    }

    pub fn set_state(&mut self, next: &str) {
        if self.state != next {
            self.state = next.to_string();
            self.elapsed = 0.0;
        }
    }

    pub fn set_facing(&mut self, dir: i32) {
        self.facing = if dir >= 0 { 1 } else { -1 };
    }

    pub fn set_sitting(&mut self, sitting: bool) {
        self.sitting = sitting;
    }

    pub fn update(&mut self, dt: f32) {
        self.elapsed += dt;
    }

    pub fn sync_phase(&mut self, elapsed: f32) {
        self.elapsed = elapsed;
    }

    fn current_asset(&self) -> &SpriteAsset {
        let key = if self.sitting && self.state == "idle" && self.assets.contains_key("sit") {
            "sit"
        } else if self.assets.contains_key(&self.state) {
            self.state.as_str()
        } else if matches!(
            self.state.as_str(),
            "butt_dance" | "boogie" | "spin" | "spin_dance" | "excited"
        ) && self.assets.contains_key("dance")
        {
            "dance"
        } else if matches!(
            self.state.as_str(),
            "drag"
                | "fall"
                | "reach"
                | "greet"
                | "chat"
                | "dance"
                | "butt_dance"
                | "boogie"
                | "spin"
                | "spin_dance"
                | "excited"
        ) && self.assets.contains_key("talk")
        {
            "talk"
        } else if self.state == "chase" && self.assets.contains_key("walk") {
            "walk"
        } else if self.state == "special" && self.assets.contains_key("annoyed") {
            "annoyed"
        } else if self.state == "sleep" && self.assets.contains_key("sit") {
            "sit"
        } else {
            "idle"
        };
        self.assets
            .get(key)
            .unwrap_or_else(|| self.assets.get("idle").expect("idle exists"))
    }

    pub fn render(
        &mut self,
        dst_bgra: &mut [u8],
        dst_w: usize,
        dst_h: usize,
        dst_x: i32,
        dst_y: i32,
        box_px: u32,
    ) {
        let box_sz = box_px.max(1);
        let needs_alloc = self
            .scratch
            .as_ref()
            .is_none_or(|p| p.width() != box_sz || p.height() != box_sz);
        if needs_alloc {
            self.scratch = Pixmap::new(box_sz, box_sz);
        }
        let Some(mut scratch) = self.scratch.take() else {
            return;
        };
        scratch.fill(tiny_skia::Color::TRANSPARENT);

        {
            let asset = self.current_asset();
            let frame = if asset.frames.len() <= 1 || asset.total_sec <= 0.0 {
                &asset.frames[0]
            } else {
                let mut t = self.elapsed.rem_euclid(asset.total_sec);
                let mut chosen = &asset.frames[0];
                for f in &asset.frames {
                    if t <= f.delay_sec {
                        chosen = f;
                        break;
                    }
                    t -= f.delay_sec;
                }
                chosen
            };

            let fw = frame.pixmap.width() as f32;
            let fh = frame.pixmap.height() as f32;
            let scale = (box_sz as f32 / fw).min(box_sz as f32 / fh);
            let draw_w = fw * scale;
            let draw_h = fh * scale;
            let ox = (box_sz as f32 - draw_w) * 0.5;
            let oy = box_sz as f32 - draw_h; // object-position: bottom

            let tx = if self.facing >= 0 {
                Transform::from_row(scale, 0.0, 0.0, scale, ox, oy)
            } else {
                Transform::from_row(-scale, 0.0, 0.0, scale, ox + draw_w, oy)
            };
            let paint = PixmapPaint {
                opacity: 1.0,
                blend_mode: BlendMode::SourceOver,
                quality: FilterQuality::Bilinear,
            };
            scratch.draw_pixmap(0, 0, frame.pixmap.as_ref(), &paint, tx, None);
        }

        composite_rgba_onto_bgra(
            scratch.data(),
            box_sz as usize,
            box_sz as usize,
            dst_bgra,
            dst_w,
            dst_h,
            dst_x,
            dst_y,
        );
        self.scratch = Some(scratch);
    }
}

pub fn composite_rgba_onto_bgra(
    src_rgba: &[u8],
    src_w: usize,
    src_h: usize,
    dst_bgra: &mut [u8],
    dst_w: usize,
    dst_h: usize,
    dst_x: i32,
    dst_y: i32,
) {
    for sy in 0..src_h {
        let py = dst_y + sy as i32;
        if py < 0 || py >= dst_h as i32 {
            continue;
        }
        let s_row = sy * src_w * 4;
        let d_row = py as usize * dst_w * 4;
        for sx in 0..src_w {
            let px = dst_x + sx as i32;
            if px < 0 || px >= dst_w as i32 {
                continue;
            }
            let s_off = s_row + sx * 4;
            let sa = src_rgba[s_off + 3];
            if sa == 0 {
                continue;
            }
            let sr = src_rgba[s_off];
            let sg = src_rgba[s_off + 1];
            let sb = src_rgba[s_off + 2];

            let d_off = d_row + px as usize * 4;
            if sa == 255 || dst_bgra[d_off + 3] == 0 {
                dst_bgra[d_off] = sb;
                dst_bgra[d_off + 1] = sg;
                dst_bgra[d_off + 2] = sr;
                dst_bgra[d_off + 3] = sa;
            } else {
                let inv = 255 - sa as u16;
                dst_bgra[d_off] = sb.saturating_add(((dst_bgra[d_off] as u16 * inv + 127) / 255) as u8);
                dst_bgra[d_off + 1] =
                    sg.saturating_add(((dst_bgra[d_off + 1] as u16 * inv + 127) / 255) as u8);
                dst_bgra[d_off + 2] =
                    sr.saturating_add(((dst_bgra[d_off + 2] as u16 * inv + 127) / 255) as u8);
                dst_bgra[d_off + 3] =
                    sa.saturating_add(((dst_bgra[d_off + 3] as u16 * inv + 127) / 255) as u8);
            }
        }
    }
}

fn rgba_to_premul_pixmap(rgba: &[u8], w: u32, h: u32) -> Result<Pixmap, String> {
    let mut pix = Pixmap::new(w.max(1), h.max(1)).ok_or("Invalid sprite size")?;
    for (s, d) in rgba.chunks_exact(4).zip(pix.data_mut().chunks_exact_mut(4)) {
        let a = s[3] as u16;
        if a == 0 {
            d.copy_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            d.copy_from_slice(s);
        } else {
            d[0] = ((s[0] as u16 * a + 127) / 255) as u8;
            d[1] = ((s[1] as u16 * a + 127) / 255) as u8;
            d[2] = ((s[2] as u16 * a + 127) / 255) as u8;
            d[3] = a as u8;
        }
    }
    Ok(pix)
}

fn load_sprite_file(path: &Path) -> Result<SpriteAsset, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if ext == "svg" {
        let data = fs::read(path).map_err(|e| format!("Failed reading {:?}: {}", path, e))?;
        let opt = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_data(&data, &opt)
            .map_err(|e| format!("Invalid SVG {:?}: {}", path, e))?;
        let sz = tree.size();
        let scale = (512.0 / sz.width().max(sz.height()).max(1.0)).max(1.0);
        let w = (sz.width() * scale).ceil().max(1.0) as u32;
        let h = (sz.height() * scale).ceil().max(1.0) as u32;
        let mut pix = Pixmap::new(w, h).ok_or("SVG too large")?;
        resvg::render(&tree, Transform::from_scale(scale, scale), &mut pix.as_mut());
        return Ok(SpriteAsset {
            frames: vec![SpriteFrame {
                delay_sec: 1.0,
                pixmap: pix,
            }],
            total_sec: 1.0,
        });
    }

    if ext == "gif" {
        let file = File::open(path).map_err(|e| format!("Failed opening {:?}: {}", path, e))?;
        let decoder = GifDecoder::new(BufReader::new(file))
            .map_err(|e| format!("Invalid GIF {:?}: {}", path, e))?;
        let decoded = decoder
            .into_frames()
            .collect_frames()
            .map_err(|e| format!("Failed decoding GIF {:?}: {}", path, e))?;
        if !decoded.is_empty() {
            let mut frames = Vec::with_capacity(decoded.len());
            let mut total_sec = 0.0;
            for f in decoded {
                let (numer, denom) = f.delay().numer_denom_ms();
                let ms = numer as f32 / denom.max(1) as f32;
                let delay_sec = if ms < 20.0 { 0.1 } else { ms / 1000.0 };
                total_sec += delay_sec;
                let buf = f.into_buffer();
                let pix = rgba_to_premul_pixmap(buf.as_raw(), buf.width(), buf.height())?;
                frames.push(SpriteFrame { delay_sec, pixmap: pix });
            }
            return Ok(SpriteAsset { frames, total_sec });
        }
    }

    let img = image::open(path)
        .map_err(|e| format!("Failed loading sprite {:?}: {}", path, e))?
        .to_rgba8();
    let pix = rgba_to_premul_pixmap(img.as_raw(), img.width(), img.height())?;
    Ok(SpriteAsset {
        frames: vec![SpriteFrame {
            delay_sec: 1.0,
            pixmap: pix,
        }],
        total_sec: 1.0,
    })
}
