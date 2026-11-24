#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod control_panel;
mod effects;
mod model_view;
mod pet;
mod raster3d;
mod sprite_view;
mod ui_overlay;
mod win32_app;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mutex_name: Vec<u16> = "Local\\com.frceapp.desktoppet.rust\0"
            .encode_utf16()
            .collect();
        let mutex = CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
        if !mutex.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(mutex);
            return;
        }
    }

    if let Ok(mut app) = win32_app::Win32App::new() {
        app.run();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::config::{load_characters, parse_hex_color};
    use crate::model_view::{paint_expression_texture, ModelView};

    #[test]
    fn loads_all_three_characters_and_detects_limbs() {
        let char_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("characters");
        let chars = load_characters(&char_dir);
        assert_eq!(chars.len(), 3, "expected chiikawa, hachiware, usagi");

        for c in &chars {
            let model_cfg = c.config.model.as_ref().expect("model config");
            let mut view = ModelView::load(&c.dir, model_cfg)
                .unwrap_or_else(|e| panic!("failed loading {}: {}", c.id, e));
            let (legs, arms, ears) = view.limb_counts();
            assert!(legs >= 2, "{} expected >=2 legs, got {}", c.id, legs);
            assert!(arms >= 2, "{} expected >=2 arms, got {}", c.id, arms);
            assert!(ears >= 2, "{} expected >=2 ears, got {}", c.id, ears);

            let mut buf = vec![0u8; 260 * 300 * 4];
            view.update(0.016);
            view.render(&mut buf, 260, 300, 85, 210, 90);
            let opaque_pixels = buf.chunks_exact(4).filter(|p| p[3] > 0).count();
            assert!(opaque_pixels > 500, "{} rendered too few pixels: {}", c.id, opaque_pixels);

            // Verify inverted-hull outline is culled in front so bright body pixels are visible
            let bright_pixels = buf
                .chunks_exact(4)
                .filter(|p| p[3] == 255 && p[2] > 200 && p[1] > 180)
                .count();
            assert!(bright_pixels > 200, "{} expected bright body pixels, got {}", c.id, bright_pixels);
        }
    }

    #[test]
    fn effects_render_cleanly() {
        use crate::effects::{BlackHoleEffect, CrackEffect, CrackRenderResult};

        let bh = BlackHoleEffect::new(128, 128, 6.0, None);
        let mut bh_buf = vec![0u8; 128 * 128 * 4];
        assert!(bh.render(1.0, &mut bh_buf));
        assert!(bh_buf.chunks_exact(4).any(|p| p[3] > 0));
        assert!(!bh.render(6.5, &mut bh_buf));

        let mut cr = CrackEffect::new(200, 140, 5.0, 1.0);
        let mut cr_buf = vec![0u8; 200 * 140 * 4];
        assert!(matches!(cr.render(0.1, &mut cr_buf), CrackRenderResult::Updated));
        assert!(cr_buf.chunks_exact(4).any(|p| p[3] > 0));
        assert!(matches!(cr.render(1.0, &mut cr_buf), CrackRenderResult::Updated));
        assert!(matches!(cr.render(1.5, &mut cr_buf), CrackRenderResult::Unchanged));
        assert!(matches!(cr.render(5.1, &mut cr_buf), CrackRenderResult::Finished));
    }

    #[test]
    fn expression_paint_preserves_white_rgb_on_transparent_pixels_when_downscaled() {
        let w = 2048u32;
        let h = 2048u32;
        let mut orig = vec![0u8; (w * h * 4) as usize];
        for px in orig.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 255, 255, 0]);
        }
        let tex = paint_expression_texture(&orig, w, h, &[], parse_hex_color("#391a1c"));
        assert_eq!(tex.base_size(), (1024, 1024));
        assert_eq!(tex.levels[0].pixels[0], [255, 255, 255, 0]);
    }

    #[test]
    fn renders_control_panel_ui_and_hitboxes() {
        use crate::config::Settings;
        use crate::control_panel::{ControlPanel, PANEL_H, PANEL_W};
        use crate::ui_overlay::TextRenderer;

        let char_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("characters");
        let settings = Settings::default();
        let mut tr = TextRenderer::new();
        let mut panel = ControlPanel::new();
        let (pw, ph) = panel.render_to_buffer(&char_dir, &settings, &mut tr, 1.0);
        assert_eq!(pw, PANEL_W as u32);
        assert_eq!(ph, PANEL_H as u32);
        // 3 chars + toggles + 5 size presets + 2 sliders + 4 behavior toggles + 4 buttons
        assert!(
            panel.hitbox_count() >= 18,
            "expected interactive hitboxes, got {}",
            panel.hitbox_count()
        );
    }

    #[test]
    fn animations_and_butt_dance_pose_cleanly() {
        let char_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("characters");
        let chars = load_characters(&char_dir);
        let dances = ["butt_dance", "boogie", "spin", "spin_dance", "excited", "dance"];
        let fidgets = [
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

        for c in &chars {
            let model_cfg = c.config.model.as_ref().expect("model config");
            let mut view = ModelView::load(&c.dir, model_cfg).expect("load model");
            let mut buf = vec![0u8; 260 * 300 * 4];

            for state in dances {
                view.set_state(state);
                view.sync_phase(0.6);
                view.update(0.05);
                assert!(view.check_positions_finite(), "{} {} non-finite", c.id, state);
                buf.fill(0);
                view.render(&mut buf, 260, 300, 85, 210, 90);
                let opaque = buf.chunks_exact(4).filter(|p| p[3] > 0).count();
                assert!(opaque > 500, "{} {} rendered {} pixels", c.id, state, opaque);
            }

            for fidget in fidgets {
                view.set_fidget_for_test(fidget);
                view.update(0.05);
                assert!(view.check_positions_finite(), "{} {} non-finite", c.id, fidget);
            }
        }

        if let Ok(out_path) = std::env::var("DUMP_VERIFY_GRID") {
            let cell = 160usize;
            let states = [
                ("idle", 0.0_f32, false),
                ("walk", 0.45, false),
                ("butt_dance", 0.9, false),
                ("cry", 0.5, false),
                ("excited", 0.2, false),
                ("idle", 0.5, true),
            ];
            let gw = cell * states.len();
            let gh = cell * chars.len();
            let mut grid = vec![240u8; gw * gh * 4];
            for px in grid.chunks_exact_mut(4) {
                px[3] = 255;
            }
            for (ci, c) in chars.iter().enumerate() {
                let model_cfg = c.config.model.as_ref().expect("model config");
                for (si, &(st, phase, sit)) in states.iter().enumerate() {
                    let mut view = ModelView::load(&c.dir, model_cfg).expect("load");
                    view.set_sitting(sit);
                    view.set_state(st);
                    view.sync_phase(phase);
                    for _ in 0..40 {
                        view.update(0.016);
                    }
                    let mut cell_buf = vec![0u8; cell * cell * 4];
                    view.render(&mut cell_buf, cell, cell, 0, 0, cell as u32);
                    for y in 0..cell {
                        for x in 0..cell {
                            let s = (y * cell + x) * 4;
                            let a = cell_buf[s + 3] as u16;
                            if a == 0 {
                                continue;
                            }
                            let inv = 255 - a;
                            let d = ((ci * cell + y) * gw + (si * cell + x)) * 4;
                            grid[d] = cell_buf[s + 2].saturating_add(((grid[d] as u16 * inv + 127) / 255) as u8);
                            grid[d + 1] = cell_buf[s + 1].saturating_add(((grid[d + 1] as u16 * inv + 127) / 255) as u8);
                            grid[d + 2] = cell_buf[s].saturating_add(((grid[d + 2] as u16 * inv + 127) / 255) as u8);
                            grid[d + 3] = 255;
                        }
                    }
                }
            }
            let img = image::RgbaImage::from_raw(gw as u32, gh as u32, grid).expect("grid");
            img.save(&out_path).expect("save verify grid");
        }
    }
}
