use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use glam::{Mat4, Quat, Vec3};
use tiny_skia::{
    BlendMode, FilterQuality, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint,
    Stroke, Transform,
};

use crate::config::{parse_hex_color, ExpressionOp, ModelConfig, TearsConfig};
use crate::raster3d::{AlphaMode3D, MipTexture, RenderTarget3D};

const FACE_TEXTURE_SIZE: u32 = 1024;
pub const BASE_FRAME: f32 = 1.3;
pub const FRAME: f32 = 1.65;
const TEAR_COUNT: usize = 4;
const FIDGETS: [&str; 9] = [
    "wave",
    "cheer",
    "look",
    "wiggle",
    "butt_peek",
    "spin_hop",
    "bow",
    "stretch",
    "tiptoe",
];
const FIDGET_SECONDS: f32 = 2.0;
const ARM_UP: f32 = 0.82;

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone)]
struct RawImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

struct CachedGltf {
    doc: gltf::Document,
    buffers: Vec<gltf::buffer::Data>,
    images: Vec<RawImage>,
}

static GLTF_CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<CachedGltf>>>> = OnceLock::new();

fn load_cached_gltf(path: &Path) -> Result<Arc<CachedGltf>, String> {
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let cache = GLTF_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(found) = guard.get(&key) {
            return Ok(found.clone());
        }
    }

    let (doc, buffers, raw_images) =
        gltf::import(&key).map_err(|e| format!("Failed to import glTF {:?}: {}", key, e))?;
    let images = raw_images.into_iter().map(convert_gltf_image).collect();
    let entry = Arc::new(CachedGltf {
        doc,
        buffers,
        images,
    });

    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, entry.clone());
    }
    Ok(entry)
}

fn convert_gltf_image(img: gltf::image::Data) -> RawImage {
    use gltf::image::Format;
    let rgba = match img.format {
        Format::R8G8B8A8 => img.pixels,
        Format::R8G8B8 => img
            .pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        Format::R8 => img
            .pixels
            .into_iter()
            .flat_map(|r| [r, r, r, 255])
            .collect(),
        Format::R8G8 => img
            .pixels
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        Format::R16G16B16A16 => img
            .pixels
            .chunks_exact(8)
            .flat_map(|p| [p[1], p[3], p[5], p[7]])
            .collect(),
        Format::R16G16B16 => img
            .pixels
            .chunks_exact(6)
            .flat_map(|p| [p[1], p[3], p[5], 255])
            .collect(),
        Format::R16 => img
            .pixels
            .chunks_exact(2)
            .flat_map(|p| [p[1], p[1], p[1], 255])
            .collect(),
        Format::R16G16 => img
            .pixels
            .chunks_exact(4)
            .flat_map(|p| [p[1], p[1], p[1], p[3]])
            .collect(),
        Format::R32G32B32FLOAT => img
            .pixels
            .chunks_exact(12)
            .flat_map(|p| {
                let r = f32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                let g = f32::from_le_bytes([p[4], p[5], p[6], p[7]]);
                let b = f32::from_le_bytes([p[8], p[9], p[10], p[11]]);
                [
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    (b.clamp(0.0, 1.0) * 255.0) as u8,
                    255,
                ]
            })
            .collect(),
        Format::R32G32B32A32FLOAT => img
            .pixels
            .chunks_exact(16)
            .flat_map(|p| {
                let r = f32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                let g = f32::from_le_bytes([p[4], p[5], p[6], p[7]]);
                let b = f32::from_le_bytes([p[8], p[9], p[10], p[11]]);
                let a = f32::from_le_bytes([p[12], p[13], p[14], p[15]]);
                [
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    (b.clamp(0.0, 1.0) * 255.0) as u8,
                    (a.clamp(0.0, 1.0) * 255.0) as u8,
                ]
            })
            .collect(),
    };
    RawImage {
        width: img.width,
        height: img.height,
        rgba,
    }
}

#[inline]
fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[inline]
fn sign_or_zero(v: f32) -> f32 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

struct CollectedPrimitive {
    local_positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    world_mat: Mat4,
    base_color_srgb: [f32; 4],
    alpha_mode: AlphaMode3D,
    double_sided: bool,
    image_index: Option<usize>,
}

struct RuntimePrimitive {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    base_color_srgb: [f32; 4],
    alpha_mode: AlphaMode3D,
    double_sided: bool,
    is_face: bool,
}

#[derive(Debug, Clone)]
struct Limb {
    prim_idx: usize,
    indices: Vec<usize>,
    rest: Vec<[f32; 3]>,
    tall: f32,
    side: f32,
    flex: f32,
    joint: Vec3,
}

struct Piece {
    indices: Vec<usize>,
    solid: bool,
}

fn find_root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        let p = parent[i];
        parent[i] = parent[p];
        i = p;
    }
    i
}

fn union_sets(parent: &mut [usize], a: usize, b: usize) {
    let ra = find_root(parent, a);
    let rb = find_root(parent, b);
    if ra != rb {
        parent[ra] = rb;
    }
}

fn extract_pieces(local_positions: &[[f32; 3]], indices: &[u32]) -> Vec<Piece> {
    let count = local_positions.len();
    if count == 0 || indices.is_empty() {
        return Vec::new();
    }
    let mut parent: Vec<usize> = (0..count).collect();
    let mut at: HashMap<[i64; 3], usize> = HashMap::with_capacity(count);
    let mut welded = vec![0usize; count];

    for (i, p) in local_positions.iter().enumerate() {
        let key = [
            (p[0] * 1e4).round() as i64,
            (p[1] * 1e4).round() as i64,
            (p[2] * 1e4).round() as i64,
        ];
        let first = *at.entry(key).or_insert(i);
        welded[i] = first;
        union_sets(&mut parent, i, first);
    }

    let mut uses: HashMap<(usize, usize), u32> = HashMap::with_capacity(indices.len());
    for tri in indices.chunks_exact(3) {
        let corners = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        for n in 0..3 {
            let c0 = corners[n];
            let c1 = corners[(n + 1) % 3];
            union_sets(&mut parent, c0, c1);
            let a = welded[c0];
            let b = welded[c1];
            let edge = if a < b { (a, b) } else { (b, a) };
            *uses.entry(edge).or_insert(0) += 1;
        }
    }

    let mut open = HashSet::new();
    for ((a, _), uses_count) in uses {
        if uses_count == 1 {
            open.insert(find_root(&mut parent, a));
        }
    }

    let mut group_map: HashMap<usize, usize> = HashMap::new();
    let mut groups: Vec<Piece> = Vec::new();
    for i in 0..count {
        let root = find_root(&mut parent, i);
        let idx = *group_map.entry(root).or_insert_with(|| {
            let new_idx = groups.len();
            groups.push(Piece {
                indices: Vec::new(),
                solid: !open.contains(&root),
            });
            new_idx
        });
        groups[idx].indices.push(i);
    }
    groups
}

fn build_sphere(radius: f32, width_segs: usize, height_segs: usize) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for iy in 0..=height_segs {
        let v = iy as f32 / height_segs as f32;
        let theta = v * PI;
        for ix in 0..=width_segs {
            let u = ix as f32 / width_segs as f32;
            let phi = u * PI * 2.0;
            let x = -radius * phi.cos() * theta.sin();
            let y = radius * theta.cos();
            let z = radius * phi.sin() * theta.sin();
            positions.push([x, y, z]);
        }
    }
    let row = width_segs + 1;
    for iy in 0..height_segs {
        for ix in 0..width_segs {
            let a = (iy * row + ix + 1) as u32;
            let b = (iy * row + ix) as u32;
            let c = ((iy + 1) * row + ix) as u32;
            let d = ((iy + 1) * row + ix + 1) as u32;
            if iy != 0 {
                indices.extend_from_slice(&[a, b, d]);
            }
            if iy != height_segs - 1 {
                indices.extend_from_slice(&[b, c, d]);
            }
        }
    }
    (positions, indices)
}

fn expand_expression_ops(
    name: &str,
    exprs: &HashMap<String, Vec<ExpressionOp>>,
    visiting: &mut HashSet<String>,
    out: &mut Vec<ExpressionOp>,
) {
    if !visiting.insert(name.to_string()) {
        return;
    }
    if let Some(ops) = exprs.get(name) {
        for op in ops {
            match op {
                ExpressionOp::Include(ref_name) => {
                    expand_expression_ops(ref_name, exprs, visiting, out);
                }
                other => out.push(other.clone()),
            }
        }
    }
    visiting.remove(name);
}

fn rgba_to_premul_pixmap(rgba: &[u8], width: u32, height: u32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    let dst = pixmap.data_mut();
    for (s, d) in rgba.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
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
    Some(pixmap)
}

pub fn paint_expression_texture(
    orig_rgba: &[u8],
    orig_w: u32,
    orig_h: u32,
    ops: &[ExpressionOp],
    ink: [u8; 4],
) -> MipTexture {
    let mut pixmap = rgba_to_premul_pixmap(orig_rgba, orig_w, orig_h)
        .unwrap_or_else(|| Pixmap::new(1, 1).expect("1x1 pixmap"));
    let orig_pixmap = pixmap.clone();

    for op in ops {
        match op {
            ExpressionOp::Include(_) => {}
            ExpressionOp::Clear([x, y, w, h]) => {
                let x0 = (*x).floor().max(0.0) as i32;
                let y0 = (*y).floor().max(0.0) as i32;
                let x1 = (*x + *w).ceil().min(orig_w as f32) as i32;
                let y1 = (*y + *h).ceil().min(orig_h as f32) as i32;
                let stride = orig_w as usize * 4;
                let data = pixmap.data_mut();
                for py in y0..y1 {
                    let row = py as usize * stride;
                    for px in x0..x1 {
                        let off = row + px as usize * 4;
                        data[off..off + 4].fill(0);
                    }
                }
            }
            ExpressionOp::Copy {
                sx,
                sy,
                w,
                h,
                dx,
                dy,
                dw,
                dh,
            } => {
                let sx0 = sx.floor().max(0.0) as u32;
                let sy0 = sy.floor().max(0.0) as u32;
                let sw = w.ceil().max(1.0) as u32;
                let sh = h.ceil().max(1.0) as u32;
                let cw = sw.min(orig_w.saturating_sub(sx0));
                let ch = sh.min(orig_h.saturating_sub(sy0));
                if cw > 0 && ch > 0 {
                    if let Some(mut crop) = Pixmap::new(cw, ch) {
                        let src_data = orig_pixmap.data();
                        let dst_data = crop.data_mut();
                        let src_stride = orig_w as usize * 4;
                        let dst_stride = cw as usize * 4;
                        for row in 0..ch as usize {
                            let s_off = (sy0 as usize + row) * src_stride + sx0 as usize * 4;
                            let d_off = row * dst_stride;
                            dst_data[d_off..d_off + dst_stride]
                                .copy_from_slice(&src_data[s_off..s_off + dst_stride]);
                        }
                        let paint = PixmapPaint {
                            opacity: 1.0,
                            blend_mode: BlendMode::SourceOver,
                            quality: FilterQuality::Bilinear,
                        };
                        let sx_scale = *dw / cw as f32;
                        let sy_scale = *dh / ch as f32;
                        let tx = Transform::from_row(sx_scale, 0.0, 0.0, sy_scale, *dx, *dy);
                        pixmap.draw_pixmap(0, 0, crop.as_ref(), &paint, tx, None);
                    }
                }
            }
            ExpressionOp::Stroke { width, points } => {
                if points.len() >= 2 {
                    let mut pb = PathBuilder::new();
                    pb.move_to(points[0][0], points[0][1]);
                    for p in &points[1..] {
                        pb.line_to(p[0], p[1]);
                    }
                    if let Some(path) = pb.finish() {
                        let mut paint = Paint::default();
                        paint.set_color_rgba8(ink[0], ink[1], ink[2], ink[3]);
                        paint.anti_alias = true;
                        let stroke = Stroke {
                            width: *width,
                            line_cap: LineCap::Round,
                            line_join: LineJoin::Round,
                            ..Default::default()
                        };
                        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
                    }
                }
            }
        }
    }

    let mut straight_rgba = vec![0u8; (orig_w * orig_h * 4) as usize];
    for (src, dst) in pixmap
        .data()
        .chunks_exact(4)
        .zip(straight_rgba.chunks_exact_mut(4))
    {
        let a = src[3];
        if a == 0 {
            dst.copy_from_slice(&[255, 255, 255, 0]);
        } else if a == 255 {
            dst.copy_from_slice(src);
        } else {
            let af = a as f32;
            dst[0] = ((src[0] as f32 * 255.0 / af).round()).clamp(0.0, 255.0) as u8;
            dst[1] = ((src[1] as f32 * 255.0 / af).round()).clamp(0.0, 255.0) as u8;
            dst[2] = ((src[2] as f32 * 255.0 / af).round()).clamp(0.0, 255.0) as u8;
            dst[3] = a;
        }
    }

    MipTexture::from_rgba8(&straight_rgba, orig_w, orig_h, FACE_TEXTURE_SIZE)
}

pub struct ModelView {
    primitives: Vec<RuntimePrimitive>,
    legs: Vec<Limb>,
    arms: Vec<Limb>,
    ears: Vec<Limb>,
    tails: Vec<Limb>,
    belly: f32,
    size: Vec3,
    span: f32,
    leg_height: f32,
    base_yaw_rad: f32,
    ink_color: [u8; 4],

    orig_face_image: Option<RawImage>,
    textures: HashMap<String, Arc<MipTexture>>,
    expression_ops: HashMap<String, Vec<ExpressionOp>>,
    current_expression: String,

    tears_cfg: Option<TearsConfig>,
    tear_color_srgb: [f32; 4],
    tear_positions: Vec<[f32; 3]>,
    tear_indices: Vec<u32>,

    state: String,
    facing: i32,
    sitting: bool,
    walk_speed_scale: f32,
    sit: f32,
    yaw: f32,
    cur_pitch: f32,
    cur_roll: f32,
    cur_hop: f32,
    cur_shift: f32,
    cur_stretch: f32,
    cur_step: f32,
    cur_raise: [f32; 2],
    cur_lean: [[f32; 2]; 2],
    cur_tail_wag: f32,
    cur_tail_lift: f32,
    clock: f32,
    walk_phase: f32,
    since: f32,
    blink_in: f32,
    fidget: Option<(&'static str, f32, f32)>,
    fidget_in: f32,
    flick: Option<(f32, f32)>,
    flick_in: f32,

    pivot_mat: Mat4,
    rt: RenderTarget3D,
}

impl ModelView {
    pub fn load(char_dir: &Path, opts: &ModelConfig) -> Result<Self, String> {
        let model_path = char_dir.join(&opts.file);
        let cached = load_cached_gltf(&model_path)?;
        let scene = cached
            .doc
            .default_scene()
            .or_else(|| cached.doc.scenes().next())
            .ok_or_else(|| "glTF has no scene".to_string())?;

        let mut collected = Vec::new();
        let target_node = opts.node.as_deref();
        for node in scene.nodes() {
            collect_node(
                &node,
                Mat4::IDENTITY,
                target_node.is_none(),
                target_node,
                &cached,
                &mut collected,
            );
        }

        if collected.is_empty() {
            return Err(match target_node {
                Some(n) => format!("Model has no node named \"{}\"", n),
                None => "Model has no triangle meshes".to_string(),
            });
        }

        let mut box_min = Vec3::splat(f32::INFINITY);
        let mut box_max = Vec3::splat(f32::NEG_INFINITY);
        for prim in &collected {
            for p in &prim.local_positions {
                let wp = prim.world_mat.transform_point3(Vec3::from_array(*p));
                box_min = box_min.min(wp);
                box_max = box_max.max(wp);
            }
        }
        let size = box_max - box_min;
        let center = (box_min + box_max) * 0.5;
        let stand_offset = Vec3::new(-center.x, -box_min.y, -center.z);
        let stand_mat = Mat4::from_translation(stand_offset);

        let mut primitives = Vec::with_capacity(collected.len());
        let mut legs = Vec::new();
        let mut arms = Vec::new();
        let mut ears = Vec::new();
        let mut tails = Vec::new();
        let mut belly = 0.0_f32;
        let mut face_image_index: Option<usize> = None;

        for (prim_idx, cp) in collected.into_iter().enumerate() {
            let pivot_from_local = stand_mat * cp.world_mat;
            let mut indices = cp.indices;
            if pivot_from_local.determinant() < 0.0 {
                for tri in indices.chunks_exact_mut(3) {
                    tri.swap(1, 2);
                }
            }

            let pivot_positions: Vec<[f32; 3]> = cp
                .local_positions
                .iter()
                .map(|p| pivot_from_local.transform_point3(Vec3::from_array(*p)).to_array())
                .collect();

            if cp.image_index.is_some() && face_image_index.is_none() {
                face_image_index = cp.image_index;
            }

            for piece in extract_pieces(&cp.local_positions, &indices) {
                let mut rest = Vec::with_capacity(piece.indices.len());
                let mut pmin = Vec3::splat(f32::INFINITY);
                let mut pmax = Vec3::splat(f32::NEG_INFINITY);
                for &vi in &piece.indices {
                    let pt = Vec3::from_array(pivot_positions[vi]);
                    rest.push(pt.to_array());
                    pmin = pmin.min(pt);
                    pmax = pmax.max(pt);
                }
                let tall = pmax.y - pmin.y;
                let wide = pmax.x - pmin.x;
                let depth = pmax.z - pmin.z;
                let middle = (pmin + pmax) * 0.5;
                let side = sign_or_zero(middle.x);

                if pmin.y > size.y * 0.6 && pmax.y > size.y * 0.9 && middle.x.abs() > size.x * 0.08 {
                    // Wide flush ears (like Hachiware) stay anchored to the hair cap; tall ears move softly.
                    let aspect = wide / tall.max(1e-4);
                    let flex = if aspect > 0.78 { 0.35 } else if tall > size.y * 0.22 { 0.8 } else { 0.9 };
                    ears.push(Limb {
                        prim_idx,
                        indices: piece.indices,
                        rest,
                        tall,
                        side,
                        flex,
                        joint: Vec3::new(middle.x, pmin.y + tall * 0.05, middle.z),
                    });
                } else if !piece.solid {
                    continue;
                } else if pmin.y <= size.y * 0.03 && pmax.y <= size.y * 0.25 {
                    legs.push(Limb {
                        prim_idx,
                        indices: piece.indices,
                        rest,
                        tall,
                        side,
                        flex: 1.0,
                        joint: Vec3::new(middle.x, pmin.y + tall * 0.65, middle.z),
                    });
                } else if middle.x.abs() > size.x * 0.2
                    && middle.y > size.y * 0.15
                    && middle.y < size.y * 0.6
                    && tall < size.y * 0.4
                {
                    arms.push(Limb {
                        prim_idx,
                        indices: piece.indices,
                        rest,
                        tall,
                        side,
                        flex: 1.0,
                        joint: Vec3::new(
                            middle.x - side * wide * 0.35,
                            pmax.y - tall * 0.2,
                            middle.z,
                        ),
                    });
                } else if middle.x.abs() <= size.x * 0.22
                    && middle.z < -size.z * 0.15
                    && pmin.y > size.y * 0.02
                    && pmax.y < size.y * 0.55
                    && tall < size.y * 0.42
                {
                    // Short puffball tails (Chiikawa/Usagi) wiggle subtly so outline shells never detach.
                    let flex = if depth / tall.max(1e-4) < 1.35 { 0.45 } else { 0.95 };
                    tails.push(Limb {
                        prim_idx,
                        indices: piece.indices,
                        rest,
                        tall,
                        side: 0.0,
                        flex,
                        joint: Vec3::new(middle.x, (pmin.y + pmax.y) * 0.5, pmax.z + 0.008),
                    });
                } else {
                    for pt in &rest {
                        if pt[1] < size.y * 0.2 {
                            belly = belly.max(pt[2]);
                        }
                    }
                }
            }

            primitives.push(RuntimePrimitive {
                positions: pivot_positions,
                uvs: cp.uvs,
                indices,
                base_color_srgb: cp.base_color_srgb,
                alpha_mode: cp.alpha_mode,
                double_sided: cp.double_sided,
                is_face: cp.image_index.is_some(),
            });
        }

        for limbs in [&mut legs, &mut arms, &mut ears] {
            for &side in &[-1.0_f32, 1.0_f32] {
                let mut sum = Vec3::ZERO;
                let mut min_flex = 1.0_f32;
                let mut count = 0usize;
                for limb in limbs.iter() {
                    if (limb.side - side).abs() < 0.01 {
                        sum += limb.joint;
                        min_flex = min_flex.min(limb.flex);
                        count += 1;
                    }
                }
                if count > 0 {
                    let avg = sum / count as f32;
                    for limb in limbs.iter_mut() {
                        if (limb.side - side).abs() < 0.01 {
                            limb.joint = avg;
                            limb.flex = min_flex;
                        }
                    }
                }
            }
        }
        if !tails.is_empty() {
            let sum: Vec3 = tails.iter().map(|t| t.joint).sum();
            let avg = sum / tails.len() as f32;
            let min_flex = tails.iter().map(|t| t.flex).fold(1.0_f32, f32::min);
            for tail in tails.iter_mut() {
                tail.joint = avg;
                tail.flex = min_flex;
            }
        }

        let leg_height = legs.first().map(|l| l.tall).unwrap_or(0.0);
        let span = size.y * FRAME;

        let orig_face_image = face_image_index.and_then(|idx| cached.images.get(idx).cloned());
        let mut textures = HashMap::new();
        if let Some(orig) = &orig_face_image {
            if !opts.expressions.contains_key("neutral") {
                let tex = Arc::new(MipTexture::from_rgba8(
                    &orig.rgba,
                    orig.width,
                    orig.height,
                    FACE_TEXTURE_SIZE,
                ));
                textures.insert("neutral".to_string(), tex);
            }
        }

        let ink_color = parse_hex_color(opts.ink.as_deref().unwrap_or("#391a1c"));
        let tear_color_srgb = if let Some(t) = &opts.tears {
            let c = parse_hex_color(t.color.as_deref().unwrap_or("#8ed6ff"));
            [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
        } else {
            [0.557, 0.839, 1.0, 1.0]
        };
        let (tear_positions, tear_indices) = if opts.tears.is_some() {
            build_sphere(size.y * 0.035, 12, 8)
        } else {
            (Vec::new(), Vec::new())
        };

        let mut view = Self {
            primitives,
            legs,
            arms,
            ears,
            tails,
            belly,
            size,
            span,
            leg_height,
            base_yaw_rad: opts.yaw.unwrap_or(0.0).to_radians(),
            ink_color,
            orig_face_image,
            textures,
            expression_ops: opts.expressions.clone(),
            current_expression: "neutral".to_string(),
            tears_cfg: opts.tears.clone(),
            tear_color_srgb,
            tear_positions,
            tear_indices,
            state: "idle".to_string(),
            facing: 1,
            sitting: false,
            walk_speed_scale: 1.0,
            sit: 0.0,
            yaw: 0.0,
            cur_pitch: 0.0,
            cur_roll: 0.0,
            cur_hop: 0.0,
            cur_shift: 0.0,
            cur_stretch: 1.0,
            cur_step: 0.0,
            cur_raise: [0.0, 0.0],
            cur_lean: [[0.0, 0.0], [0.0, 0.0]],
            cur_tail_wag: 0.0,
            cur_tail_lift: 0.0,
            clock: 0.0,
            walk_phase: 0.0,
            since: 0.0,
            blink_in: 3.0,
            fidget: None,
            fidget_in: 2.0,
            flick: None,
            flick_in: 2.5,
            pivot_mat: Mat4::IDENTITY,
            rt: RenderTarget3D::new(90, 90),
        };
        view.show_expression("neutral");
        view.update(0.0);
        Ok(view)
    }

    #[cfg(test)]
    pub fn limb_counts(&self) -> (usize, usize, usize) {
        (self.legs.len(), self.arms.len(), self.ears.len())
    }

    #[cfg(test)]
    pub fn set_fidget_for_test(&mut self, kind: &'static str) {
        self.set_state("idle");
        self.fidget = Some((kind, 0.35, 1.0));
    }

    #[cfg(test)]
    pub fn check_positions_finite(&self) -> bool {
        self.primitives
            .iter()
            .flat_map(|p| p.positions.iter())
            .all(|v| v[0].is_finite() && v[1].is_finite() && v[2].is_finite())
    }

    pub fn sync_phase(&mut self, since: f32) {
        self.since = since;
    }

    #[inline]
    pub fn width_ratio(&self) -> f32 {
        self.size.x / (self.size.y * BASE_FRAME)
    }

    #[inline]
    pub fn height_ratio(&self) -> f32 {
        1.0 / BASE_FRAME + 0.03
    }

    pub fn set_facing(&mut self, dir: i32) {
        self.facing = if dir >= 0 { 1 } else { -1 };
    }

    pub fn set_sitting(&mut self, sitting: bool) {
        self.sitting = sitting;
    }

    pub fn set_speed_scale(&mut self, scale: f32) {
        self.walk_speed_scale = scale.clamp(0.45, 2.0);
    }

    pub fn set_state(&mut self, next: &str) {
        if self.state == next {
            return;
        }
        self.state = next.to_string();
        self.since = 0.0;
        self.fidget = None;
        self.fidget_in = 1.6 + fastrand::f32() * 2.2;

        let has = |name: &str| self.expression_ops.contains_key(name);
        let stand = match next {
            "drag" | "fall" | "chase" | "reach" | "greet" | "chat" | "dance"
            | "butt_dance" | "boogie" | "spin" | "spin_dance" | "excited" => Some("talk"),
            "special" => Some("annoyed"),
            "sleep" => Some("blink"),
            _ => None,
        };
        let expr = if has(next) {
            next
        } else if let Some(s) = stand {
            if has(s) { s } else { "neutral" }
        } else {
            "neutral"
        };
        self.current_expression = expr.to_string();
        self.show_expression(expr);
    }

    fn show_expression(&mut self, name: &str) {
        if self.orig_face_image.is_none() {
            return;
        }
        if !self.textures.contains_key(name) {
            let orig = self.orig_face_image.as_ref().unwrap();
            let mut ops = Vec::new();
            let mut visiting = HashSet::new();
            expand_expression_ops(name, &self.expression_ops, &mut visiting, &mut ops);
            let tex = Arc::new(paint_expression_texture(
                &orig.rgba,
                orig.width,
                orig.height,
                &ops,
                self.ink_color,
            ));
            self.textures.insert(name.to_string(), tex);
        }
        self.current_expression = name.to_string();
    }

    fn place_limb(&mut self, limb: &Limb, rot: Quat, nudge: Vec3) {
        let positions = &mut self.primitives[limb.prim_idx].positions;
        for (&vi, rest_arr) in limb.indices.iter().zip(limb.rest.iter()) {
            let p = Vec3::from_array(*rest_arr);
            let posed = rot.mul_vec3(p - limb.joint) + limb.joint + nudge;
            positions[vi] = posed.to_array();
        }
    }

    fn pose_limbs(
        &mut self,
        step: f32,
        sit: f32,
        raise: [f32; 2],
        lean: [[f32; 2]; 2],
        tail_wag: f32,
        tail_lift: f32,
    ) {
        let step = step.clamp(-0.85, 0.85);
        for i in 0..self.legs.len() {
            let leg = self.legs[i].clone();
            let stride = step * leg.side;
            let rx = -stride * 0.62 - sit * 1.32;
            let ry = sit * 0.28 * leg.side;
            let rot = Quat::from_rotation_y(ry) * Quat::from_rotation_x(rx);
            let nudge = Vec3::new(
                0.0,
                stride.max(0.0) * leg.tall * 0.24,
                sit * (self.belly - leg.joint.z + leg.tall * 0.35),
            );
            self.place_limb(&leg, rot, nudge);
        }
        for i in 0..self.arms.len() {
            let arm = self.arms[i].clone();
            let side_idx = if arm.side < 0.0 { 0 } else { 1 };
            let lift = raise[side_idx].clamp(-0.15, ARM_UP);
            let rx = step * arm.side * 0.42;
            let rz = lift * arm.side;
            let rot = Quat::from_rotation_x(rx) * Quat::from_rotation_z(rz);
            self.place_limb(&arm, rot, Vec3::ZERO);
        }
        for i in 0..self.ears.len() {
            let ear = self.ears[i].clone();
            let side_idx = if ear.side < 0.0 { 0 } else { 1 };
            let [raw_out, raw_fwd] = lean[side_idx];
            let out = (raw_out * ear.flex).clamp(-0.14, 0.24);
            let forward = (raw_fwd * ear.flex).clamp(-0.22, 0.18);
            let rot = Quat::from_rotation_x(forward) * Quat::from_rotation_z(-out * ear.side);
            self.place_limb(&ear, rot, Vec3::ZERO);
        }
        for i in 0..self.tails.len() {
            let tail = self.tails[i].clone();
            let wag = (tail_wag * tail.flex).clamp(-0.28, 0.28);
            let lift = (tail_lift * tail.flex).clamp(-0.10, 0.12);
            let rot = Quat::from_rotation_y(wag) * Quat::from_rotation_x(lift);
            self.place_limb(&tail, rot, Vec3::ZERO);
        }
    }

    pub fn update(&mut self, dt: f32) {
        self.clock += dt;
        self.walk_phase += dt * self.walk_speed_scale;
        self.since += dt;
        let clock = self.clock;
        let wphase = self.walk_phase;
        let since = self.since;
        let facing = self.facing as f32;
        let facing_idx = if self.facing < 0 { 0 } else { 1 };
        let other_idx = 1 - facing_idx;

        let mut turn = 0.0_f32;
        let mut hop = 0.0_f32;
        let mut roll = 0.0_f32;
        let mut pitch = 0.0_f32;
        let mut shift = 0.0_f32;
        let mut stretch = 1.0_f32;
        let mut step = 0.0_f32;
        let mut tail_wag = 0.0_f32;
        let mut tail_lift = 0.0_f32;
        let mut raise = [0.0_f32; 2];
        let mut lean = [
            [(clock * 1.4).sin() * 0.04, 0.0_f32],
            [(clock * 1.4 + 2.0).sin() * 0.04, 0.0_f32],
        ];
        let both_ears = |lean: &mut [[f32; 2]; 2], out: f32, forward: f32| {
            lean[0][0] += out;
            lean[0][1] += forward;
            lean[1][0] += out;
            lean[1][1] += forward;
        };

        match self.state.as_str() {
            "walk" => {
                turn = facing * 0.55;
                step = (wphase * 6.4).sin();
                hop = step.abs() * 0.026;
                roll = step * 0.048;
                stretch = 1.0 + (wphase * 12.8).cos() * 0.015;
                raise = [0.14, 0.14];
                tail_wag = step * 0.18;
                both_ears(
                    &mut lean,
                    (wphase * 6.4 - 1.1).sin() * 0.05,
                    -0.08 + (wphase * 12.8 - 1.1).sin() * 0.11,
                );
            }
            "talk" => {
                hop = (since * 7.5).sin().abs() * 0.05 * (-since * 2.2).exp();
                stretch = 1.0 + (since * 10.0).sin() * 0.025;
                let r = 0.42 + (since * 9.0).sin() * 0.2;
                raise = [r, r];
                both_ears(&mut lean, 0.06 + (since * 9.0).sin() * 0.05, 0.0);
            }
            "annoyed" => {
                turn = (since * 13.0).sin() * (0.42 - self.sit * 0.25);
                stretch = 1.0 + (since * 18.0).sin() * 0.02;
                raise[0] = 0.42 + (since * 14.0).sin() * 0.32;
                raise[1] = 0.42 - (since * 14.0).sin() * 0.32;
                both_ears(&mut lean, 0.22 + (since * 18.0).sin() * 0.05, 0.0);
            }
            "special" => {
                turn = facing * 0.3;
                hop = 0.05 + (since * 4.0).sin() * 0.02;
                stretch = 1.0 + (since * 16.0).sin() * 0.015;
                let r = 0.72 + (since * 12.0).sin() * 0.08;
                raise = [r, r];
                both_ears(
                    &mut lean,
                    -0.04 + (since * 22.0).sin() * 0.05,
                    (since * 18.0).sin() * 0.06,
                );
            }
            "cry" => {
                pitch = 0.09;
                shift = (since * 26.0).sin() * 0.006;
                stretch = 1.0 + (since * 7.5).sin() * 0.022;
                let r = 0.45 + (since * 15.0).sin() * 0.06;
                raise = [r, r];
                both_ears(&mut lean, 0.24, 0.14);
            }
            "chase" => {
                turn = facing * 0.58;
                step = (wphase * 8.2).sin();
                hop = step.abs() * 0.034;
                roll = step * 0.058;
                let r = 0.68 + (wphase * 8.2).sin() * 0.1;
                raise = [r, r];
                tail_wag = step * 0.24;
                both_ears(&mut lean, 0.04, -0.16);
            }
            "reach" => {
                turn = facing * 0.25;
                hop = (since * 5.2).sin().abs() * 0.075;
                stretch = 1.03 + (since * 10.4).sin() * 0.02;
                raise[0] = 0.62 + (since * 10.0).sin() * 0.18;
                raise[1] = 0.62 - (since * 10.0).sin() * 0.18;
                both_ears(&mut lean, -0.04 + (since * 10.0).sin() * 0.04, 0.0);
            }
            "greet" => {
                turn = facing * 0.45;
                hop = (since * 6.5).sin().abs() * 0.065 * (-since * 1.6).exp();
                raise[facing_idx] = 0.62 + (since * 10.0).sin() * 0.2;
                raise[other_idx] = 0.15;
                tail_wag = (since * 8.0).sin() * 0.2;
                both_ears(&mut lean, 0.08 + (since * 10.0).sin() * 0.05, 0.0);
            }
            "chat" => {
                turn = facing * 0.48;
                pitch = (since * 2.8).sin().max(0.0) * 0.06;
                stretch = 1.0 + (since * 8.0).sin() * 0.018;
                raise[0] = 0.35 + (since * 4.2).sin() * 0.22;
                raise[1] = 0.35 - (since * 4.2).sin() * 0.22;
            }
            "dance" => {
                let beat = since * 6.0;
                turn = facing * 0.28 + (beat * 0.5).sin() * 0.24;
                hop = beat.sin().abs() * 0.058;
                roll = beat.sin() * 0.085;
                stretch = 1.0 + (beat * 2.0).cos() * 0.028;
                raise[0] = 0.46 + beat.sin() * 0.3;
                raise[1] = 0.46 - beat.sin() * 0.3;
                tail_wag = beat.sin() * 0.22;
                both_ears(&mut lean, beat.sin() * 0.1, (beat * 2.0).sin() * 0.08);
            }
            "butt_dance" => {
                // Smooth 0.5s turn-around, then relaxed groovy hip-shake & tail wag,
                // with a gentle over-the-shoulder peek around t = 2.6..3.4s.
                let intro = smoothstep(0.0, 0.5, since);
                let beat = (since - 0.25).max(0.0) * 7.2;
                let wiggle = beat.sin();
                let peek = if since > 2.6 && since < 3.4 {
                    let u = (since - 2.6) / 0.8;
                    0.5 - 0.5 * (2.0 * PI * u).cos()
                } else {
                    0.0
                };
                turn = facing * (PI - peek * 0.9) + wiggle * 0.32 * intro * (1.0 - peek * 0.65);
                pitch = (0.22 + (beat * 0.5).sin().abs() * 0.04) * intro - peek * 0.1;
                roll = wiggle * 0.12 * intro;
                shift = -wiggle * 0.045 * intro;
                hop = beat.cos().abs() * 0.034 * intro + peek * 0.025;
                stretch = 0.98 + (beat * 2.0).sin() * 0.026 * intro;
                step = wiggle * 0.28 * intro;
                raise[0] = (0.48 + wiggle * 0.18) * intro;
                raise[1] = (0.48 - wiggle * 0.18) * intro;
                tail_wag = wiggle * 0.26 * intro;
                tail_lift = 0.1 * intro;
                both_ears(
                    &mut lean,
                    (0.08 + wiggle.abs() * 0.06) * intro,
                    (-0.1 + (beat * 2.0).sin() * 0.08) * intro,
                );
            }
            "boogie" => {
                let intro = smoothstep(0.0, 0.3, since);
                let beat = since * 6.2;
                let sway = (beat * 0.5).sin();
                turn = sway * 0.42 * intro;
                shift = sway * 0.065 * intro;
                hop = beat.sin().abs() * 0.062 * intro;
                roll = sway * 0.11 * intro;
                stretch = 1.0 + (beat * 2.0).cos() * 0.032 * intro;
                step = beat.sin() * 0.45 * intro;
                let pump = (0.32 + beat.sin().max(0.0) * 0.42) * intro;
                raise = [pump, pump];
                tail_wag = sway * 0.24 * intro;
                both_ears(&mut lean, sway * 0.1 * intro, (beat * 2.0).sin() * 0.07 * intro);
            }
            "spin" | "spin_dance" => {
                let beat = since * 6.0;
                hop = beat.sin().abs() * 0.06;
                stretch = 1.02 + (beat * 2.0).cos() * 0.02;
                raise[facing_idx] = 0.72;
                raise[other_idx] = 0.42;
                tail_wag = beat.sin() * 0.2;
                both_ears(&mut lean, 0.16, -0.06);
            }
            "excited" => {
                let beat = since * 7.5;
                hop = beat.sin().abs() * 0.085;
                stretch = 0.97 + hop * 0.8;
                let arm = 0.24 + (hop / 0.085) * 0.54;
                raise = [arm, arm];
                tail_wag = beat.sin() * 0.24;
                both_ears(&mut lean, -0.04 + hop * 0.8, 0.0);
            }
            "comfort" => {
                turn = facing * 0.5;
                roll = -facing * 0.08;
                raise[facing_idx] = 0.48 + (since * 4.8).sin().abs() * 0.28;
                both_ears(&mut lean, 0.16, 0.08);
            }
            "sleep" => {
                pitch = 0.11 + (clock * 0.85).sin() * 0.045;
                roll = (clock * 0.42).sin() * 0.035;
                stretch = 1.0 + (clock * 1.35).sin() * 0.025;
                both_ears(&mut lean, 0.18, 0.12);
            }
            "drag" => {
                roll = (since * 5.2).sin() * 0.15;
                stretch = 1.05;
                raise[0] = 0.58 + (since * 7.5).sin() * 0.2;
                raise[1] = 0.58 - (since * 7.5).sin() * 0.2;
                lean[0][0] -= (since * 5.2 - 1.0).sin() * 0.16;
                lean[1][0] += (since * 5.2 - 1.0).sin() * 0.16;
            }
            "fall" => {
                stretch = 1.07;
                raise = [0.76, 0.76];
                both_ears(&mut lean, 0.1, -0.2);
            }
            _ => {
                turn = (clock * 0.55).sin() * 0.11;
                stretch = 1.0 + (clock * 2.1).sin() * 0.014;
                let r = (clock * 2.1).sin() * 0.05;
                raise = [r, r];

                self.fidget_in -= dt;
                if self.fidget.is_none() && self.fidget_in <= 0.0 {
                    let kind = FIDGETS[fastrand::usize(..FIDGETS.len())];
                    let side = if fastrand::bool() { -1.0 } else { 1.0 };
                    self.fidget = Some((kind, 0.0, side));
                }
                if let Some((kind, mut t, side)) = self.fidget {
                    t += dt;
                    let dur = match kind {
                        "butt_peek" => 2.6,
                        "stretch" => 2.2,
                        "spin_hop" => 1.7,
                        _ => FIDGET_SECONDS,
                    };
                    let part = (t / dur).min(1.0);
                    // Smooth bell envelope: zero velocity at start & end, no hard clamping
                    let ease = smoothstep(0.0, 0.22, part) * smoothstep(1.0, 0.78, part);
                    match kind {
                        "wave" => {
                            let idx = if side < 0.0 { 0 } else { 1 };
                            raise[idx] = ease * (0.58 + (t * 9.5).sin() * 0.2);
                        }
                        "cheer" => {
                            raise = [ease * 0.76, ease * 0.76];
                            hop = (t * 6.8).sin().abs() * 0.05 * ease;
                        }
                        "look" => {
                            turn = (PI * 2.0 * part).sin() * 0.45 * ease;
                        }
                        "butt_peek" => {
                            let wiggle = (t * 7.5).sin();
                            let body_in = smoothstep(0.15, 0.35, part) * smoothstep(0.88, 0.68, part);
                            pitch = 0.2 * ease;
                            turn = side * PI * ease + wiggle * 0.28 * body_in;
                            shift = -wiggle * 0.036 * body_in;
                            roll = wiggle * 0.1 * body_in;
                            raise[0] = ease * 0.44;
                            raise[1] = ease * 0.44;
                            tail_wag = wiggle * 0.24 * body_in;
                            tail_lift = 0.09 * ease;
                        }
                        "spin_hop" => {
                            let smooth_p = part * part * (3.0 - 2.0 * part);
                            let angle = side * 2.0 * PI * smooth_p;
                            turn = (angle + PI).rem_euclid(2.0 * PI) - PI;
                            hop = (PI * part).sin() * 0.075;
                            raise = [ease * 0.68, ease * 0.68];
                        }
                        "bow" => {
                            pitch = 0.28 * ease;
                            raise = [ease * 0.28, ease * 0.28];
                            both_ears(&mut lean, 0.1 * ease, 0.12 * ease);
                        }
                        "stretch" => {
                            if part < 0.32 {
                                let sq = (part / 0.32 * PI).sin();
                                stretch = 1.0 - 0.06 * sq;
                            } else {
                                let up = smoothstep(0.32, 0.55, part) * smoothstep(1.0, 0.78, part);
                                stretch = 1.0 + 0.09 * up;
                                raise = [up * 0.78, up * 0.78];
                            }
                        }
                        "tiptoe" => {
                            step = (t * 8.5).sin() * 0.38 * ease;
                            hop = 0.025 * ease;
                            roll = (t * 4.25).sin() * 0.065 * ease;
                            raise = [0.22 * ease, 0.22 * ease];
                            tail_wag = (t * 8.5).sin() * 0.18 * ease;
                        }
                        _ => {
                            roll = (t * 6.0).sin() * 0.075 * ease;
                            raise[0] = ease * (0.38 + (t * 6.0).sin() * 0.26);
                            raise[1] = ease * (0.38 - (t * 6.0).sin() * 0.26);
                        }
                    }
                    if part >= 1.0 {
                        self.fidget = None;
                        self.fidget_in = 1.8 + fastrand::f32() * 2.4;
                    } else {
                        self.fidget = Some((kind, t, side));
                    }
                }
            }
        }

        self.flick_in -= dt;
        if self.flick.is_none() && self.flick_in <= 0.0 {
            let side = if fastrand::bool() { 0.0 } else { 1.0 };
            self.flick = Some((0.0, side));
        }
        if let Some((mut ft, side)) = self.flick {
            ft += dt;
            let idx = if side < 0.5 { 0 } else { 1 };
            lean[idx][0] += (ft * 16.0).sin().abs() * 0.2 * (PI * ft / 0.5).sin();
            if ft >= 0.5 {
                self.flick = None;
                self.flick_in = 2.0 + fastrand::f32() * 3.5;
            } else {
                self.flick = Some((ft, side));
            }
        }

        let upright = matches!(
            self.state.as_str(),
            "walk"
                | "chase"
                | "drag"
                | "fall"
                | "dance"
                | "butt_dance"
                | "boogie"
                | "spin"
                | "spin_dance"
                | "excited"
        );
        let seated = self.state == "sleep" || (self.sitting && !upright);
        let sit_target = if seated { 1.0 } else { 0.0 };
        let sit_alpha = 1.0 - (-7.5 * dt).exp();
        self.sit += (sit_target - self.sit) * sit_alpha;
        if (self.sit - sit_target).abs() < 0.002 {
            self.sit = sit_target;
        }

        // Frame-rate-independent exponential smoothing across all body & limb channels
        let a_rot = if dt <= 0.0 { 1.0 } else { 1.0 - (-8.5 * dt).exp() };
        let a_pos = if dt <= 0.0 { 1.0 } else { 1.0 - (-10.5 * dt).exp() };
        let a_limb = if dt <= 0.0 { 1.0 } else { 1.0 - (-11.5 * dt).exp() };

        self.cur_pitch += (pitch - self.cur_pitch) * a_rot;
        self.cur_roll += (roll - self.cur_roll) * a_rot;
        self.cur_hop += (hop - self.cur_hop) * a_pos;
        self.cur_shift += (shift - self.cur_shift) * a_pos;
        self.cur_stretch += (stretch - self.cur_stretch) * a_pos;
        self.cur_step += (step - self.cur_step) * a_limb;
        for s in 0..2 {
            self.cur_raise[s] += (raise[s] - self.cur_raise[s]) * a_limb;
            self.cur_lean[s][0] += (lean[s][0] - self.cur_lean[s][0]) * a_limb;
            self.cur_lean[s][1] += (lean[s][1] - self.cur_lean[s][1]) * a_limb;
        }
        self.cur_tail_wag += (tail_wag - self.cur_tail_wag) * a_limb;
        self.cur_tail_lift += (tail_lift - self.cur_tail_lift) * a_limb;

        self.pose_limbs(
            self.cur_step,
            self.sit,
            self.cur_raise,
            self.cur_lean,
            self.cur_tail_wag,
            self.cur_tail_lift,
        );

        if matches!(self.state.as_str(), "spin" | "spin_dance") {
            self.yaw = (self.yaw + facing * 4.6 * dt + PI).rem_euclid(2.0 * PI) - PI;
        } else {
            let yaw_alpha = if dt <= 0.0 { 1.0 } else { 1.0 - (-7.5 * dt).exp() };
            let diff = (turn - self.yaw + PI).rem_euclid(2.0 * PI) - PI;
            self.yaw += diff * yaw_alpha;
        }
        let rot = Quat::from_rotation_y(self.yaw + self.base_yaw_rad)
            * Quat::from_rotation_x(self.cur_pitch)
            * Quat::from_rotation_z(self.cur_roll);
        let pos = Vec3::new(
            self.cur_shift * self.size.y,
            self.cur_hop * self.size.y * (1.0 - self.sit) - self.sit * self.leg_height * 0.45,
            0.0,
        );
        let inv_sqrt = 1.0 / self.cur_stretch.sqrt();
        let scale = Vec3::new(inv_sqrt, self.cur_stretch, inv_sqrt);
        self.pivot_mat = Mat4::from_scale_rotation_translation(scale, rot, pos);

        if self.current_expression == "neutral" && self.expression_ops.contains_key("blink") {
            self.blink_in -= dt;
            if self.blink_in < -0.14 {
                self.blink_in = 2.6 + fastrand::f32() * 3.2;
            }
            let target_expr = if self.blink_in < 0.0 { "blink" } else { "neutral" };
            let prev = self.current_expression.clone();
            self.show_expression(target_expr);
            self.current_expression = prev;
        } else if self.current_expression == "talk"
            && matches!(self.state.as_str(), "talk" | "greet" | "chat")
        {
            let target_expr = if (since * 13.0).sin() > -0.3 {
                "talk"
            } else {
                "neutral"
            };
            let prev = self.current_expression.clone();
            self.show_expression(target_expr);
            self.current_expression = prev;
        }
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
        self.rt.ensure_size(box_px, box_px);
        self.rt.clear();

        let active_tex = self
            .textures
            .iter()
            .find(|_| false)
            .map(|(_, t)| t.as_ref());
        let _ = active_tex;

        for prim in &self.primitives {
            let tex = if prim.is_face {
                self.textures.get(&self.shown_texture_key()).map(|t| t.as_ref())
            } else {
                None
            };
            self.rt.draw_mesh(
                &prim.positions,
                &prim.uvs,
                &prim.indices,
                &self.pivot_mat,
                self.span,
                prim.base_color_srgb,
                tex,
                prim.alpha_mode,
                prim.double_sided,
            );
        }

        if self.state == "cry" {
            if let Some(tcfg) = &self.tears_cfg {
                for i in 0..TEAR_COUNT {
                    let side = if i % 2 == 1 { 1.0 } else { -1.0 };
                    let fall = (self.since * 1.1 + i as f32 / TEAR_COUNT as f32).rem_euclid(1.0);
                    let drop_pos = Vec3::new(
                        side * self.size.x * (tcfg.x + fall * 0.05),
                        self.size.y * (tcfg.y - fall * 0.45),
                        self.size.z * 0.5,
                    );
                    let s = 1.0 - fall * 0.5;
                    let drop_scale = Vec3::new(s, 1.5 * s, s);
                    let drop_mat = self.pivot_mat
                        * Mat4::from_scale_rotation_translation(drop_scale, Quat::IDENTITY, drop_pos);
                    self.rt.draw_mesh(
                        &self.tear_positions,
                        &[],
                        &self.tear_indices,
                        &drop_mat,
                        self.span,
                        self.tear_color_srgb,
                        None,
                        AlphaMode3D::Opaque,
                        false,
                    );
                }
            }
        }

        self.rt.downsample_2x_into(dst_bgra, dst_w, dst_h, dst_x, dst_y);
    }

    fn shown_texture_key(&self) -> String {
        if self.current_expression == "neutral" && self.expression_ops.contains_key("blink") {
            if self.blink_in < 0.0 {
                return "blink".to_string();
            }
            return "neutral".to_string();
        }
        if self.current_expression == "talk"
            && matches!(self.state.as_str(), "talk" | "greet" | "chat")
        {
            if (self.since * 20.0).sin() > -0.3 {
                return "talk".to_string();
            }
            return "neutral".to_string();
        }
        self.current_expression.clone()
    }
}

fn collect_node(
    node: &gltf::Node,
    parent_mat: Mat4,
    inside_figure: bool,
    target_node: Option<&str>,
    cached: &CachedGltf,
    out: &mut Vec<CollectedPrimitive>,
) {
    let local_mat = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world_mat = parent_mat * local_mat;
    let now_inside = inside_figure || (target_node.is_some() && node.name() == target_node);

    if now_inside {
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = prim.reader(|buf| cached.buffers.get(buf.index()).map(|b| &b.0[..]));
                let Some(pos_iter) = reader.read_positions() else {
                    continue;
                };
                let local_positions: Vec<[f32; 3]> = pos_iter.collect();
                let uvs: Vec<[f32; 2]> = reader
                    .read_tex_coords(0)
                    .map(|tc| tc.into_f32().collect())
                    .unwrap_or_else(|| vec![[0.0, 0.0]; local_positions.len()]);
                let indices: Vec<u32> = reader
                    .read_indices()
                    .map(|idx| idx.into_u32().collect())
                    .unwrap_or_else(|| (0..local_positions.len() as u32).collect());

                let mat = prim.material();
                let pbr = mat.pbr_metallic_roughness();
                let f = pbr.base_color_factor();
                let base_color_srgb = [
                    linear_to_srgb(f[0]),
                    linear_to_srgb(f[1]),
                    linear_to_srgb(f[2]),
                    f[3],
                ];
                let alpha_mode = match mat.alpha_mode() {
                    gltf::material::AlphaMode::Opaque => AlphaMode3D::Opaque,
                    gltf::material::AlphaMode::Mask => {
                        AlphaMode3D::Mask(mat.alpha_cutoff().unwrap_or(0.5))
                    }
                    gltf::material::AlphaMode::Blend => AlphaMode3D::Blend,
                };
                let image_index = pbr
                    .base_color_texture()
                    .map(|info| info.texture().source().index());

                out.push(CollectedPrimitive {
                    local_positions,
                    uvs,
                    indices,
                    world_mat,
                    base_color_srgb,
                    alpha_mode,
                    double_sided: mat.double_sided(),
                    image_index,
                });
            }
        }
    }

    for child in node.children() {
        collect_node(&child, world_mat, now_inside, target_node, cached, out);
    }
}
