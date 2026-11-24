use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::path::Path;

use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Rect,
    Stroke, Transform,
};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, StretchDIBits,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, LoadCursorW, RegisterClassExW,
    SetCursor, SetForegroundWindow, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, IDC_ARROW,
    IDC_HAND, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SW_RESTORE, SW_SHOW, WM_CAPTURECHANGED,
    WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_SETCURSOR, WNDCLASSEXW, WS_CAPTION, WS_MINIMIZEBOX,
    WS_OVERLAPPED, WS_SYSMENU,
};

use crate::config::{chosen_characters, load_characters, PetSettings, Settings, SIZES};
use crate::pet::render_character_avatar;
use crate::ui_overlay::{draw_glyph_on_pixmap, TextRenderer};

pub const PANEL_W: f32 = 640.0;
pub const PANEL_H: f32 = 520.0;

const SIDEBAR_W: f32 = 210.0;
const SIZE_MIN: f32 = 0.50;
const SIZE_MAX: f32 = 2.20;
const SPEED_MIN: f32 = 0.25;
const SPEED_MAX: f32 = 2.50;

// Palette (Chiikawa warm cream + soft coral + sage)
const COL_BG: [u8; 4] = [0xfd, 0xfa, 0xf6, 255];
const COL_SIDEBAR: [u8; 4] = [0xf6, 0xef, 0xe8, 255];
const COL_CARD: [u8; 4] = [0xff, 0xff, 0xff, 255];
const COL_CARD_SEL: [u8; 4] = [0xff, 0xf3, 0xed, 255];
const COL_BORDER: [u8; 4] = [0xea, 0xdf, 0xd5, 255];
const COL_BORDER_SEL: [u8; 4] = [0xe8, 0x8c, 0x72, 255];
const COL_HAIRLINE: [u8; 4] = [0xf2, 0xe9, 0xe1, 255];
const COL_INK: [u8; 4] = [0x4a, 0x3b, 0x35, 255];
const COL_MUTED: [u8; 4] = [0x8b, 0x76, 0x6c, 255];
const COL_ACCENT: [u8; 4] = [0xe8, 0x8c, 0x72, 255];
const COL_GREEN: [u8; 4] = [0x6d, 0xbf, 0x78, 255];
const COL_GREEN_DIM: [u8; 4] = [0x9e, 0xd4, 0xa5, 255];
const COL_SWITCH_OFF: [u8; 4] = [0xd8, 0xcd, 0xc4, 255];
const COL_BTN_HOVER: [u8; 4] = [0xf2, 0xe6, 0xdb, 255];
const COL_DANGER: [u8; 4] = [0xc9, 0x52, 0x4b, 255];

#[derive(Debug, Clone)]
pub enum PanelAction {
    ToggleCharacter(String),
    ChangePetSettings {
        id: String,
        settings: PetSettings,
        save_to_disk: bool,
    },
    TriggerDance(String),
    BringToFloor(String),
    OpenCharactersFolder,
    QuitApp,
}

#[derive(Debug, Clone)]
enum PanelWinEvent {
    MouseMove { x: i32, y: i32 },
    LeftDown { x: i32, y: i32 },
    LeftUp { x: i32 },
    CaptureLost,
    Wheel { delta: i32 },
    DpiChanged { dpi: f32 },
    Closed,
}

#[derive(Default)]
struct PaintFramebuffer {
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

thread_local! {
    static PANEL_EVENTS: RefCell<VecDeque<PanelWinEvent>> = const { RefCell::new(VecDeque::new()) };
    static PANEL_FB: RefCell<PaintFramebuffer> = RefCell::new(PaintFramebuffer::default());
    static PANEL_HOVER_HAND: RefCell<bool> = const { RefCell::new(false) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SliderKind {
    Size,
    Speed,
}

#[derive(Debug, Clone, PartialEq)]
enum HitTarget {
    SelectChar(String),
    ToggleChar(String),
    SetSizePreset(String, f32),
    Slider(SliderKind, String, f32, f32), // kind, id, track_x, track_w
    ToggleStay(String),
    ToggleSleep(String),
    ToggleMute(String),
    ToggleChase(String),
    TriggerDance(String),
    BringToFloor(String),
    ResetDefaults(String),
    OpenFolder,
    QuitApp,
}

#[derive(Debug, Clone)]
struct HitBox {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    target: HitTarget,
}

impl HitBox {
    fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn panel_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_SETCURSOR => {
            if (lparam as u32 & 0xFFFF) == 1 {
                let hand = PANEL_HOVER_HAND.with(|h| *h.borrow());
                let cur = LoadCursorW(std::ptr::null_mut(), if hand { IDC_HAND } else { IDC_ARROW });
                SetCursor(cur);
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut client = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            GetClientRect(hwnd, &mut client);
            let cw = (client.right - client.left).max(1);
            let ch = (client.bottom - client.top).max(1);

            PANEL_FB.with(|fb_cell| {
                let fb = fb_cell.borrow();
                if fb.width > 0 && fb.height > 0 && !fb.bgra.is_empty() {
                    let mut bmi: BITMAPINFO = std::mem::zeroed();
                    bmi.bmiHeader = BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: fb.width as i32,
                        biHeight: -(fb.height as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB,
                        ..std::mem::zeroed()
                    };
                    StretchDIBits(
                        hdc,
                        0,
                        0,
                        cw,
                        ch,
                        0,
                        0,
                        fb.width as i32,
                        fb.height as i32,
                        fb.bgra.as_ptr() as *const c_void,
                        &bmi,
                        DIB_RGB_COLORS,
                        SRCCOPY,
                    );
                }
            });
            EndPaint(hwnd, &ps);
            0
        }
        WM_MOUSEMOVE => {
            let x = (lparam as i16) as i32;
            let y = ((lparam >> 16) as i16) as i32;
            PANEL_EVENTS.with(|q| q.borrow_mut().push_back(PanelWinEvent::MouseMove { x, y }));
            0
        }
        WM_LBUTTONDOWN => {
            SetCapture(hwnd);
            let x = (lparam as i16) as i32;
            let y = ((lparam >> 16) as i16) as i32;
            PANEL_EVENTS.with(|q| q.borrow_mut().push_back(PanelWinEvent::LeftDown { x, y }));
            0
        }
        WM_LBUTTONUP => {
            let x = (lparam as i16) as i32;
            PANEL_EVENTS.with(|q| q.borrow_mut().push_back(PanelWinEvent::LeftUp { x }));
            if GetCapture() == hwnd {
                ReleaseCapture();
            }
            0
        }
        WM_CAPTURECHANGED => {
            PANEL_EVENTS.with(|q| q.borrow_mut().push_back(PanelWinEvent::CaptureLost));
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) as i16) as i32;
            PANEL_EVENTS.with(|q| q.borrow_mut().push_back(PanelWinEvent::Wheel { delta }));
            0
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam & 0xFFFF) as f32 / 96.0;
            let suggested = &*(lparam as *const RECT);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            PANEL_EVENTS.with(|q| {
                q.borrow_mut()
                    .push_back(PanelWinEvent::DpiChanged { dpi: new_dpi })
            });
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PANEL_EVENTS.with(|q| q.borrow_mut().push_back(PanelWinEvent::Closed));
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub struct ControlPanel {
    class_registered: bool,
    class_name: Vec<u16>,
    hwnd: HWND,
    dpi: f32,
    selected_id: Option<String>,
    scroll_y: f32,
    max_scroll_y: f32,
    hover: Option<HitTarget>,
    dragging_slider: Option<(SliderKind, String, f32, f32)>,
    avatars: HashMap<String, Pixmap>,
    hitboxes: Vec<HitBox>,
    dirty: bool,
    pixmap: Option<Pixmap>,
}

impl ControlPanel {
    pub fn new() -> Self {
        Self {
            class_registered: false,
            class_name: wide_null("DesktopPetControlPanelClass"),
            hwnd: std::ptr::null_mut(),
            dpi: 1.0,
            selected_id: None,
            scroll_y: 0.0,
            max_scroll_y: 0.0,
            hover: None,
            dragging_slider: None,
            avatars: HashMap::new(),
            hitboxes: Vec::new(),
            dirty: true,
            pixmap: None,
        }
    }

    pub fn open(
        &mut self,
        initial_pet: Option<&str>,
        char_dir: &Path,
        settings: &Settings,
        text_renderer: &mut TextRenderer,
    ) {
        if let Some(id) = initial_pet {
            self.selected_id = Some(id.to_string());
        } else if self.selected_id.is_none() {
            self.selected_id = chosen_characters(char_dir, settings)
                .first()
                .map(|c| c.id.clone());
        }

        if !self.hwnd.is_null() {
            unsafe {
                ShowWindow(self.hwnd, SW_RESTORE);
                SetForegroundWindow(self.hwnd);
            }
            self.dirty = true;
            self.render_if_needed(char_dir, settings, text_renderer);
            return;
        }

        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            if !self.class_registered {
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(panel_wnd_proc),
                    hInstance: hinst,
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    lpszClassName: self.class_name.as_ptr(),
                    ..std::mem::zeroed()
                };
                RegisterClassExW(&wc);
                self.class_registered = true;
            }

            let sys_dpi = GetDpiForSystem().max(96);
            self.dpi = sys_dpi as f32 / 96.0;
            let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
            let mut wr = RECT {
                left: 0,
                top: 0,
                right: (PANEL_W * self.dpi).round() as i32,
                bottom: (PANEL_H * self.dpi).round() as i32,
            };
            AdjustWindowRectExForDpi(&mut wr, style, 0, 0, sys_dpi);
            let win_w = wr.right - wr.left;
            let win_h = wr.bottom - wr.top;

            let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            let (wx, wy) = if GetMonitorInfoW(hmon, &mut mi) != 0 {
                let mw = mi.rcWork.right - mi.rcWork.left;
                let mh = mi.rcWork.bottom - mi.rcWork.top;
                (
                    mi.rcWork.left + (mw - win_w) / 2,
                    mi.rcWork.top + (mh - win_h) / 2,
                )
            } else {
                (160, 120)
            };

            let title = wide_null("Desktop Pet — Character Control Panel");
            let hwnd = CreateWindowExW(
                0,
                self.class_name.as_ptr(),
                title.as_ptr(),
                style,
                wx,
                wy,
                win_w,
                win_h,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null_mut(),
            );
            if hwnd.is_null() {
                return;
            }
            self.hwnd = hwnd;

            let actual_dpi = GetDpiForWindow(hwnd);
            if actual_dpi >= 96 && actual_dpi != sys_dpi {
                self.dpi = actual_dpi as f32 / 96.0;
                let mut adj = RECT {
                    left: 0,
                    top: 0,
                    right: (PANEL_W * self.dpi).round() as i32,
                    bottom: (PANEL_H * self.dpi).round() as i32,
                };
                AdjustWindowRectExForDpi(&mut adj, style, 0, 0, actual_dpi);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    adj.right - adj.left,
                    adj.bottom - adj.top,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }

            self.dirty = true;
            self.render_if_needed(char_dir, settings, text_renderer);
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
        }
    }

    pub fn mark_dirty(&mut self) {
        if !self.hwnd.is_null() {
            self.dirty = true;
        }
    }

    pub fn poll_events(
        &mut self,
        char_dir: &Path,
        settings: &Settings,
        text_renderer: &mut TextRenderer,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        let events: Vec<PanelWinEvent> =
            PANEL_EVENTS.with(|q| q.borrow_mut().drain(..).collect());
        if self.hwnd.is_null() && events.is_empty() {
            return actions;
        }

        for ev in events {
            match ev {
                PanelWinEvent::MouseMove { x, y } => {
                    let lx = x as f32 / self.dpi;
                    let ly = y as f32 / self.dpi;
                    if let Some((kind, id, tx, tw)) = self.dragging_slider.clone() {
                        let next = self.compute_slider_settings(&id, kind, tx, tw, lx, settings);
                        actions.push(PanelAction::ChangePetSettings {
                            id,
                            settings: next,
                            save_to_disk: false,
                        });
                        self.dirty = true;
                    } else {
                        let next_hover = self.hit_test(lx, ly);
                        if next_hover != self.hover {
                            PANEL_HOVER_HAND.with(|h| *h.borrow_mut() = next_hover.is_some());
                            self.hover = next_hover;
                            self.dirty = true;
                        }
                    }
                }
                PanelWinEvent::LeftDown { x, y } => {
                    let lx = x as f32 / self.dpi;
                    let ly = y as f32 / self.dpi;
                    if let Some(target) = self.hit_test(lx, ly) {
                        match target {
                            HitTarget::SelectChar(id) => {
                                if self.selected_id.as_deref() != Some(id.as_str()) {
                                    self.selected_id = Some(id);
                                    self.dirty = true;
                                }
                            }
                            HitTarget::ToggleChar(id) => {
                                actions.push(PanelAction::ToggleCharacter(id));
                                self.dirty = true;
                            }
                            HitTarget::SetSizePreset(id, scale) => {
                                let mut next = settings.pet_settings(&id);
                                next.scale = scale;
                                actions.push(PanelAction::ChangePetSettings {
                                    id,
                                    settings: next,
                                    save_to_disk: true,
                                });
                                self.dirty = true;
                            }
                            HitTarget::Slider(kind, id, tx, tw) => {
                                self.dragging_slider = Some((kind, id.clone(), tx, tw));
                                let next =
                                    self.compute_slider_settings(&id, kind, tx, tw, lx, settings);
                                actions.push(PanelAction::ChangePetSettings {
                                    id,
                                    settings: next,
                                    save_to_disk: false,
                                });
                                self.dirty = true;
                            }
                            HitTarget::ToggleStay(id) => {
                                let mut next = settings.pet_settings(&id);
                                next.stay = !next.stay;
                                actions.push(PanelAction::ChangePetSettings {
                                    id,
                                    settings: next,
                                    save_to_disk: true,
                                });
                                self.dirty = true;
                            }
                            HitTarget::ToggleSleep(id) => {
                                let mut next = settings.pet_settings(&id);
                                next.sleep = !next.sleep;
                                actions.push(PanelAction::ChangePetSettings {
                                    id,
                                    settings: next,
                                    save_to_disk: true,
                                });
                                self.dirty = true;
                            }
                            HitTarget::ToggleMute(id) => {
                                let mut next = settings.pet_settings(&id);
                                next.mute = !next.mute;
                                actions.push(PanelAction::ChangePetSettings {
                                    id,
                                    settings: next,
                                    save_to_disk: true,
                                });
                                self.dirty = true;
                            }
                            HitTarget::ToggleChase(id) => {
                                let mut next = settings.pet_settings(&id);
                                next.chase = !next.chase;
                                actions.push(PanelAction::ChangePetSettings {
                                    id,
                                    settings: next,
                                    save_to_disk: true,
                                });
                                self.dirty = true;
                            }
                            HitTarget::TriggerDance(id) => {
                                actions.push(PanelAction::TriggerDance(id));
                            }
                            HitTarget::BringToFloor(id) => {
                                actions.push(PanelAction::BringToFloor(id));
                            }
                            HitTarget::ResetDefaults(id) => {
                                actions.push(PanelAction::ChangePetSettings {
                                    id,
                                    settings: PetSettings::default(),
                                    save_to_disk: true,
                                });
                                self.dirty = true;
                            }
                            HitTarget::OpenFolder => {
                                actions.push(PanelAction::OpenCharactersFolder);
                            }
                            HitTarget::QuitApp => {
                                actions.push(PanelAction::QuitApp);
                            }
                        }
                    }
                }
                PanelWinEvent::LeftUp { x, .. } => {
                    if let Some((kind, id, tx, tw)) = self.dragging_slider.take() {
                        let lx = x as f32 / self.dpi;
                        let next = self.compute_slider_settings(&id, kind, tx, tw, lx, settings);
                        actions.push(PanelAction::ChangePetSettings {
                            id,
                            settings: next,
                            save_to_disk: true,
                        });
                        self.dirty = true;
                    }
                }
                PanelWinEvent::CaptureLost => {
                    if let Some((_kind, id, _tx, _tw)) = self.dragging_slider.take() {
                        actions.push(PanelAction::ChangePetSettings {
                            id: id.clone(),
                            settings: settings.pet_settings(&id),
                            save_to_disk: true,
                        });
                    }
                }
                PanelWinEvent::Wheel { delta } => {
                    if self.max_scroll_y > 0.0 {
                        self.scroll_y =
                            (self.scroll_y - (delta as f32 / 120.0) * 36.0).clamp(0.0, self.max_scroll_y);
                        self.dirty = true;
                    }
                }
                PanelWinEvent::DpiChanged { dpi } => {
                    self.dpi = dpi.max(1.0);
                    self.avatars.clear();
                    self.dirty = true;
                }
                PanelWinEvent::Closed => {
                    self.hwnd = std::ptr::null_mut();
                    self.dragging_slider = None;
                    self.hover = None;
                    PANEL_HOVER_HAND.with(|h| *h.borrow_mut() = false);
                }
            }
        }

        if !self.hwnd.is_null() {
            self.render_if_needed(char_dir, settings, text_renderer);
        }
        actions
    }

    fn compute_slider_settings(
        &self,
        id: &str,
        kind: SliderKind,
        tx: f32,
        tw: f32,
        lx: f32,
        settings: &Settings,
    ) -> PetSettings {
        let t = ((lx - tx) / tw).clamp(0.0, 1.0);
        let mut next = settings.pet_settings(id);
        match kind {
            SliderKind::Size => {
                let raw = SIZE_MIN + t * (SIZE_MAX - SIZE_MIN);
                next.scale = (raw * 20.0).round() / 20.0;
            }
            SliderKind::Speed => {
                let raw = SPEED_MIN + t * (SPEED_MAX - SPEED_MIN);
                next.speed_scale = (raw * 20.0).round() / 20.0;
            }
        }
        next
    }

    fn hit_test(&self, lx: f32, ly: f32) -> Option<HitTarget> {
        self.hitboxes
            .iter()
            .rev()
            .find(|hb| hb.contains(lx, ly))
            .map(|hb| hb.target.clone())
    }

    fn render_if_needed(&mut self, char_dir: &Path, settings: &Settings, text_renderer: &mut TextRenderer) {
        if !self.dirty || self.hwnd.is_null() {
            return;
        }
        self.dirty = false;
        let (pw, ph) = self.render_to_buffer(char_dir, settings, text_renderer, self.dpi);
        if let Some(pix) = &self.pixmap {
            PANEL_FB.with(|fb_cell| {
                let mut fb = fb_cell.borrow_mut();
                fb.width = pw;
                fb.height = ph;
                fb.bgra.resize((pw * ph * 4) as usize, 0);
                for (src, dst) in pix.data().chunks_exact(4).zip(fb.bgra.chunks_exact_mut(4)) {
                    dst[0] = src[2];
                    dst[1] = src[1];
                    dst[2] = src[0];
                    dst[3] = 255;
                }
            });
            unsafe {
                InvalidateRect(self.hwnd, std::ptr::null(), 0);
            }
        }
    }

    pub fn render_to_buffer(
        &mut self,
        char_dir: &Path,
        settings: &Settings,
        text_renderer: &mut TextRenderer,
        dpi: f32,
    ) -> (u32, u32) {
        let pw = (PANEL_W * dpi).round().max(1.0) as u32;
        let ph = (PANEL_H * dpi).round().max(1.0) as u32;
        if self
            .pixmap
            .as_ref()
            .is_none_or(|p| p.width() != pw || p.height() != ph)
        {
            self.pixmap = Pixmap::new(pw, ph);
        }

        let chars = load_characters(char_dir);
        let chosen_ids: Vec<String> = chosen_characters(char_dir, settings)
            .into_iter()
            .map(|c| c.id)
            .collect();

        if self
            .selected_id
            .as_ref()
            .is_none_or(|sid| !chars.iter().any(|c| &c.id == sid))
        {
            self.selected_id = chosen_ids
                .first()
                .cloned()
                .or_else(|| chars.first().map(|c| c.id.clone()));
        }

        let avatar_px = (44.0 * dpi).round().max(24.0) as u32;
        for c in &chars {
            if !self.avatars.contains_key(&c.id) {
                if let Some(av) = render_character_avatar(c, avatar_px) {
                    self.avatars.insert(c.id.clone(), av);
                }
            }
        }

        let Some(mut pix) = self.pixmap.take() else {
            return (pw, ph);
        };
        pix.fill(Color::from_rgba8(COL_BG[0], COL_BG[1], COL_BG[2], 255));
        self.hitboxes.clear();

        // Sidebar background & divider
        fill_rect_scaled(&mut pix, 0.0, 0.0, SIDEBAR_W, PANEL_H, dpi, COL_SIDEBAR);
        fill_rect_scaled(&mut pix, SIDEBAR_W, 0.0, 1.0, PANEL_H, dpi, COL_BORDER);

        draw_text(&mut pix, text_renderer, "CHARACTERS", 14.0, 16.0, 11.0, true, COL_MUTED, dpi);

        // Sidebar character list
        let list_top = 38.0;
        let list_bottom = 422.0;
        let content_h = (chars.len() as f32 * 66.0 - 8.0).max(0.0);
        self.max_scroll_y = (content_h - (list_bottom - list_top - 4.0)).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll_y);

        for (idx, c) in chars.iter().enumerate() {
            let cy = 40.0 + idx as f32 * 66.0 - self.scroll_y;
            if cy + 58.0 < list_top || cy > list_bottom {
                continue;
            }
            let cx = 12.0;
            let cw = 186.0;
            let ch = 58.0;
            let selected = self.selected_id.as_deref() == Some(c.id.as_str());
            let active = chosen_ids.contains(&c.id);
            let last_active = active && chosen_ids.len() == 1;
            let hovered = self.hover == Some(HitTarget::SelectChar(c.id.clone()));

            let fill = if selected {
                COL_CARD_SEL
            } else if hovered {
                COL_BTN_HOVER
            } else {
                COL_CARD
            };
            let border = if selected { COL_BORDER_SEL } else { COL_BORDER };
            draw_rounded_card(&mut pix, cx, cy, cw, ch, 12.0, dpi, fill, border, 1.5);

            if let Some(av) = self.avatars.get(&c.id) {
                stamp_avatar(&mut pix, av, cx + 8.0, cy + 8.0, 42.0, dpi);
            }

            draw_text(&mut pix, text_renderer, c.display_name(), cx + 56.0, cy + 12.0, 14.0, true, COL_INK, dpi);
            let status_txt = if active { "On desktop" } else { "Hidden" };
            let status_col = if active { COL_GREEN } else { COL_MUTED };
            draw_text(&mut pix, text_renderer, status_txt, cx + 56.0, cy + 32.0, 11.0, false, status_col, dpi);

            let tx = cx + 146.0;
            let ty = cy + 20.0;
            let tw = 32.0;
            let th = 18.0;
            draw_toggle_switch(&mut pix, tx, ty, tw, th, dpi, active, last_active);

            if cy >= list_top - 4.0 && cy + ch <= list_bottom + 4.0 {
                self.hitboxes.push(HitBox {
                    x: cx,
                    y: cy,
                    w: cw,
                    h: ch,
                    target: HitTarget::SelectChar(c.id.clone()),
                });
                if !last_active {
                    self.hitboxes.push(HitBox {
                        x: tx - 4.0,
                        y: ty - 6.0,
                        w: tw + 8.0,
                        h: th + 12.0,
                        target: HitTarget::ToggleChar(c.id.clone()),
                    });
                }
            }
        }

        // Mask top/bottom of sidebar scroll area cleanly before drawing bottom sidebar buttons
        fill_rect_scaled(&mut pix, 0.0, 0.0, SIDEBAR_W, 38.0, dpi, COL_SIDEBAR);
        draw_text(&mut pix, text_renderer, "CHARACTERS", 14.0, 16.0, 11.0, true, COL_MUTED, dpi);
        fill_rect_scaled(&mut pix, 0.0, 422.0, SIDEBAR_W, PANEL_H - 422.0, dpi, COL_SIDEBAR);

        draw_button(
            &mut pix,
            text_renderer,
            12.0,
            432.0,
            186.0,
            34.0,
            dpi,
            "Open Characters Folder",
            COL_INK,
            self.hover == Some(HitTarget::OpenFolder),
        );
        self.hitboxes.push(HitBox {
            x: 12.0,
            y: 432.0,
            w: 186.0,
            h: 34.0,
            target: HitTarget::OpenFolder,
        });

        draw_button(
            &mut pix,
            text_renderer,
            12.0,
            474.0,
            186.0,
            34.0,
            dpi,
            "Quit Desktop Pet",
            COL_DANGER,
            self.hover == Some(HitTarget::QuitApp),
        );
        self.hitboxes.push(HitBox {
            x: 12.0,
            y: 474.0,
            w: 186.0,
            h: 34.0,
            target: HitTarget::QuitApp,
        });

        // Right detail pane
        if let Some(sel_def) = self
            .selected_id
            .as_ref()
            .and_then(|sid| chars.iter().find(|c| &c.id == sid))
        {
            let id = sel_def.id.clone();
            let own = settings.pet_settings(&id);
            let active = chosen_ids.contains(&id);
            let last_active = active && chosen_ids.len() == 1;

            let rx = 232.0;
            let rw = 386.0;

            // 1. Header Card
            draw_rounded_card(&mut pix, rx, 20.0, rw, 64.0, 14.0, dpi, COL_CARD, COL_BORDER, 1.5);
            if let Some(av) = self.avatars.get(&id) {
                stamp_avatar(&mut pix, av, rx + 10.0, 29.0, 46.0, dpi);
            }
            draw_text(&mut pix, text_renderer, sel_def.display_name(), rx + 66.0, 29.0, 18.0, true, COL_INK, dpi);
            let sub = if last_active {
                "Shown on desktop (last active pet)"
            } else if active {
                "Shown on desktop"
            } else {
                "Currently hidden from desktop"
            };
            draw_text(&mut pix, text_renderer, sub, rx + 66.0, 53.0, 12.0, false, COL_MUTED, dpi);
            draw_toggle_switch(&mut pix, rx + 326.0, 40.0, 44.0, 24.0, dpi, active, last_active);
            if !last_active {
                self.hitboxes.push(HitBox {
                    x: rx + 320.0,
                    y: 34.0,
                    w: 56.0,
                    h: 36.0,
                    target: HitTarget::ToggleChar(id.clone()),
                });
            }

            // 2. Size & Walk Speed Card
            draw_rounded_card(&mut pix, rx, 96.0, rw, 156.0, 14.0, dpi, COL_CARD, COL_BORDER, 1.5);

            draw_text(&mut pix, text_renderer, "Size", rx + 16.0, 108.0, 13.0, true, COL_INK, dpi);
            let size_readout = format!("{:.2}\u{00D7}", own.scale);
            draw_text_right(&mut pix, text_renderer, &size_readout, rx + rw - 16.0, 108.0, 13.0, true, COL_ACCENT, dpi);

            let pill_w = 66.0;
            let pill_gap = 6.0;
            for (pi, &(lbl, scale)) in SIZES.iter().enumerate() {
                let px = rx + 16.0 + pi as f32 * (pill_w + pill_gap);
                let py = 132.0;
                let selected_pill = (own.scale - scale).abs() < 0.02;
                let hovered_pill = self.hover == Some(HitTarget::SetSizePreset(id.clone(), scale));
                let bg = if selected_pill {
                    COL_ACCENT
                } else if hovered_pill {
                    COL_BTN_HOVER
                } else {
                    COL_SIDEBAR
                };
                let fg = if selected_pill { COL_CARD } else { COL_INK };
                let border = if selected_pill { COL_ACCENT } else { COL_BORDER };
                draw_rounded_card(&mut pix, px, py, pill_w, 24.0, 8.0, dpi, bg, border, 1.0);
                draw_text_centered(&mut pix, text_renderer, lbl, px, py, pill_w, 24.0, 12.0, selected_pill, fg, dpi);
                self.hitboxes.push(HitBox {
                    x: px,
                    y: py,
                    w: pill_w,
                    h: 24.0,
                    target: HitTarget::SetSizePreset(id.clone(), scale),
                });
            }

            let track_x = rx + 16.0;
            let track_w = rw - 32.0;
            let size_t = ((own.scale - SIZE_MIN) / (SIZE_MAX - SIZE_MIN)).clamp(0.0, 1.0);
            draw_slider(&mut pix, track_x, 171.0, track_w, size_t, dpi);
            self.hitboxes.push(HitBox {
                x: track_x - 6.0,
                y: 160.0,
                w: track_w + 12.0,
                h: 22.0,
                target: HitTarget::Slider(SliderKind::Size, id.clone(), track_x, track_w),
            });

            draw_text(&mut pix, text_renderer, "Walk Speed", rx + 16.0, 194.0, 13.0, true, COL_INK, dpi);
            let eff_speed = (sel_def.config.speed * own.speed_scale).round() as i32;
            let speed_readout = format!("{:.2}\u{00D7} ({} px/s)", own.speed_scale, eff_speed);
            draw_text_right(&mut pix, text_renderer, &speed_readout, rx + rw - 16.0, 194.0, 13.0, true, COL_ACCENT, dpi);

            let speed_t = ((own.speed_scale - SPEED_MIN) / (SPEED_MAX - SPEED_MIN)).clamp(0.0, 1.0);
            draw_slider(&mut pix, track_x, 228.0, track_w, speed_t, dpi);
            self.hitboxes.push(HitBox {
                x: track_x - 6.0,
                y: 217.0,
                w: track_w + 12.0,
                h: 22.0,
                target: HitTarget::Slider(SliderKind::Speed, id.clone(), track_x, track_w),
            });

            // 3. Behavior Card
            draw_rounded_card(&mut pix, rx, 264.0, rw, 188.0, 14.0, dpi, COL_CARD, COL_BORDER, 1.5);
            let rows = [
                ("Stay in Place", "Sit still where placed, even in mid-air", own.stay, HitTarget::ToggleStay(id.clone())),
                ("Sleep Mode", "Close eyes and doze with floating z's", own.sleep, HitTarget::ToggleSleep(id.clone())),
                ("Mute Speech", "Hide speech bubbles while moving & reacting", own.mute, HitTarget::ToggleMute(id.clone())),
                ("Chase Mouse", "Run after and jump at nearby cursor", own.chase, HitTarget::ToggleChase(id.clone())),
            ];
            for (ri, (title, desc, checked, target)) in rows.into_iter().enumerate() {
                let row_y = 264.0 + ri as f32 * 47.0;
                if ri > 0 {
                    fill_rect_scaled(&mut pix, rx + 14.0, row_y, rw - 28.0, 1.0, dpi, COL_HAIRLINE);
                }
                if self.hover.as_ref() == Some(&target) {
                    fill_rect_scaled(&mut pix, rx + 2.0, row_y + 2.0, rw - 4.0, 43.0, dpi, COL_BG);
                }
                draw_text(&mut pix, text_renderer, title, rx + 16.0, row_y + 8.0, 13.0, true, COL_INK, dpi);
                draw_text(&mut pix, text_renderer, desc, rx + 16.0, row_y + 26.0, 11.0, false, COL_MUTED, dpi);
                draw_toggle_switch(&mut pix, rx + 326.0, row_y + 12.0, 44.0, 24.0, dpi, checked, false);
                self.hitboxes.push(HitBox {
                    x: rx,
                    y: row_y,
                    w: rw,
                    h: 47.0,
                    target,
                });
            }

            // 4. Bottom Action Buttons (Butt Dance!, Bring to Floor, Reset Defaults)
            let btn_w = 122.0;
            let btn_gap = 10.0;
            draw_button(
                &mut pix,
                text_renderer,
                rx,
                466.0,
                btn_w,
                34.0,
                dpi,
                "Butt Dance!",
                COL_ACCENT,
                self.hover == Some(HitTarget::TriggerDance(id.clone())),
            );
            self.hitboxes.push(HitBox {
                x: rx,
                y: 466.0,
                w: btn_w,
                h: 34.0,
                target: HitTarget::TriggerDance(id.clone()),
            });

            let bx2 = rx + btn_w + btn_gap;
            draw_button(
                &mut pix,
                text_renderer,
                bx2,
                466.0,
                btn_w,
                34.0,
                dpi,
                "Bring to Floor",
                COL_INK,
                self.hover == Some(HitTarget::BringToFloor(id.clone())),
            );
            self.hitboxes.push(HitBox {
                x: bx2,
                y: 466.0,
                w: btn_w,
                h: 34.0,
                target: HitTarget::BringToFloor(id.clone()),
            });

            let bx3 = bx2 + btn_w + btn_gap;
            draw_button(
                &mut pix,
                text_renderer,
                bx3,
                466.0,
                btn_w,
                34.0,
                dpi,
                "Reset Defaults",
                COL_INK,
                self.hover == Some(HitTarget::ResetDefaults(id.clone())),
            );
            self.hitboxes.push(HitBox {
                x: bx3,
                y: 466.0,
                w: btn_w,
                h: 34.0,
                target: HitTarget::ResetDefaults(id),
            });
        }

        self.pixmap = Some(pix);
        (pw, ph)
    }

    #[cfg(test)]
    pub fn hitbox_count(&self) -> usize {
        self.hitboxes.len()
    }
}

impl Drop for ControlPanel {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            unsafe {
                DestroyWindow(self.hwnd);
            }
            self.hwnd = std::ptr::null_mut();
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

fn fill_rect_scaled(pix: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, dpi: f32, rgba: [u8; 4]) {
    if let Some(rect) = Rect::from_xywh(x * dpi, y * dpi, w * dpi, h * dpi) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
        pix.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_rounded_card(
    pix: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    dpi: f32,
    fill: [u8; 4],
    border: [u8; 4],
    border_w: f32,
) {
    let mut pb = PathBuilder::new();
    append_rounded_rect(&mut pb, x * dpi, y * dpi, w * dpi, h * dpi, r * dpi);
    if let Some(path) = pb.finish() {
        let mut fp = Paint::default();
        fp.set_color_rgba8(fill[0], fill[1], fill[2], fill[3]);
        fp.anti_alias = true;
        pix.fill_path(&path, &fp, FillRule::Winding, Transform::identity(), None);

        if border_w > 0.0 {
            let mut sp = Paint::default();
            sp.set_color_rgba8(border[0], border[1], border[2], border[3]);
            sp.anti_alias = true;
            let stroke = Stroke {
                width: border_w * dpi,
                ..Default::default()
            };
            pix.stroke_path(&path, &sp, &stroke, Transform::identity(), None);
        }
    }
}

fn draw_toggle_switch(
    pix: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    dpi: f32,
    on: bool,
    disabled: bool,
) {
    let bg = if on {
        if disabled { COL_GREEN_DIM } else { COL_GREEN }
    } else {
        COL_SWITCH_OFF
    };
    draw_rounded_card(pix, x, y, w, h, h * 0.5, dpi, bg, bg, 0.0);

    let pad = 3.0;
    let knob_d = h - pad * 2.0;
    let kx = if on { x + w - pad - knob_d } else { x + pad };
    let ky = y + pad;
    draw_rounded_card(pix, kx, ky, knob_d, knob_d, knob_d * 0.5, dpi, COL_CARD, COL_CARD, 0.0);
}

fn draw_slider(pix: &mut Pixmap, x: f32, y: f32, w: f32, t: f32, dpi: f32) {
    let track_h = 6.0;
    let ty = y - track_h * 0.5;
    draw_rounded_card(pix, x, ty, w, track_h, 3.0, dpi, COL_BORDER, COL_BORDER, 0.0);
    let fill_w = (w * t).max(track_h);
    draw_rounded_card(pix, x, ty, fill_w, track_h, 3.0, dpi, COL_ACCENT, COL_ACCENT, 0.0);

    let knob_r = 8.0;
    let kx = x + w * t - knob_r;
    let ky = y - knob_r;
    draw_rounded_card(
        pix,
        kx,
        ky,
        knob_r * 2.0,
        knob_r * 2.0,
        knob_r,
        dpi,
        COL_CARD,
        COL_ACCENT,
        2.5,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_button(
    pix: &mut Pixmap,
    tr: &mut TextRenderer,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    dpi: f32,
    label: &str,
    text_color: [u8; 4],
    hovered: bool,
) {
    let bg = if hovered { COL_BTN_HOVER } else { COL_CARD };
    draw_rounded_card(pix, x, y, w, h, 10.0, dpi, bg, COL_BORDER, 1.5);
    draw_text_centered(pix, tr, label, x, y, w, h, 12.0, true, text_color, dpi);
}

fn stamp_avatar(pix: &mut Pixmap, av: &Pixmap, x: f32, y: f32, size: f32, dpi: f32) {
    let s = (size * dpi) / av.width().max(1) as f32;
    let paint = PixmapPaint {
        opacity: 1.0,
        blend_mode: BlendMode::SourceOver,
        quality: FilterQuality::Bilinear,
    };
    pix.draw_pixmap(
        0,
        0,
        av.as_ref(),
        &paint,
        Transform::from_row(s, 0.0, 0.0, s, x * dpi, y * dpi),
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    pix: &mut Pixmap,
    tr: &mut TextRenderer,
    text: &str,
    x: f32,
    y: f32,
    font_dip: f32,
    bold: bool,
    color: [u8; 4],
    dpi: f32,
) {
    let glyph = tr
        .rasterize_weighted(text, (font_dip * dpi).round().max(8.0) as u32, bold)
        .clone();
    draw_glyph_on_pixmap(pix, &glyph, (x * dpi).round() as i32, (y * dpi).round() as i32, color);
}

#[allow(clippy::too_many_arguments)]
fn draw_text_right(
    pix: &mut Pixmap,
    tr: &mut TextRenderer,
    text: &str,
    right_x: f32,
    y: f32,
    font_dip: f32,
    bold: bool,
    color: [u8; 4],
    dpi: f32,
) {
    let glyph = tr
        .rasterize_weighted(text, (font_dip * dpi).round().max(8.0) as u32, bold)
        .clone();
    let dx = (right_x * dpi).round() as i32 - glyph.width as i32;
    draw_glyph_on_pixmap(pix, &glyph, dx, (y * dpi).round() as i32, color);
}

#[allow(clippy::too_many_arguments)]
fn draw_text_centered(
    pix: &mut Pixmap,
    tr: &mut TextRenderer,
    text: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    font_dip: f32,
    bold: bool,
    color: [u8; 4],
    dpi: f32,
) {
    let glyph = tr
        .rasterize_weighted(text, (font_dip * dpi).round().max(8.0) as u32, bold)
        .clone();
    let dx = (x * dpi).round() as i32 + ((w * dpi).round() as i32 - glyph.width as i32) / 2;
    let dy = (y * dpi).round() as i32 + ((h * dpi).round() as i32 - glyph.height as i32) / 2;
    draw_glyph_on_pixmap(pix, &glyph, dx, dy, color);
}
