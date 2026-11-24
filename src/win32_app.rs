use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetMonitorInfoW,
    MonitorFromPoint, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CheckMenuRadioItem, CreateIconIndirect, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetWindowLongPtrW, LoadCursorW, MsgWaitForMultipleObjectsEx, PeekMessageW, PostMessageW,
    PostQuitMessage, RegisterClassExW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, TrackPopupMenu, TranslateMessage, UpdateLayeredWindow, CS_HREDRAW, CS_VREDRAW,
    GWL_EXSTYLE, HICON, HTCLIENT, HWND_TOPMOST, ICONINFO, IDC_ARROW, MA_NOACTIVATE, MF_BYCOMMAND,
    MF_CHECKED, MF_ENABLED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG,
    MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOW,
    SW_SHOWNOACTIVATE, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, ULW_ALPHA, WM_APP,
    WM_CAPTURECHANGED, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCHITTEST, WM_NULL, WM_QUIT, WM_RBUTTONUP, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::config::{chosen_characters, find_characters_dir, load_characters, Settings, SIZES};
use crate::control_panel::{ControlPanel, PanelAction};
use crate::effects::{
    capture_screen_patch, BlackHoleEffect, CrackEffect, CrackRenderResult, EffectRequest,
};
use crate::pet::{MeetRequest, OtherPet, Pet, WorkArea};
use crate::ui_overlay::TextRenderer;

const WM_TRAYICON: u32 = WM_APP + 1;
const TRAY_ICON_ID: u32 = 1;

#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(uperiod: u32) -> u32;
    fn timeEndPeriod(uperiod: u32) -> u32;
}

struct TimerPeriodGuard;
impl TimerPeriodGuard {
    fn new() -> Self {
        unsafe {
            timeBeginPeriod(1);
        }
        Self
    }
}
impl Drop for TimerPeriodGuard {
    fn drop(&mut self) {
        unsafe {
            timeEndPeriod(1);
        }
    }
}

#[derive(Debug)]
enum AppEvent {
    PointerDown { hwnd: isize, x: i32, y: i32 },
    PointerMove { hwnd: isize, x: i32, y: i32 },
    PointerUp { hwnd: isize, is_click: bool },
    RightClick { hwnd: isize, x: i32, y: i32 },
    TrayMenu { x: i32, y: i32 },
    DisplayChanged,
    Quit,
}

thread_local! {
    static EVENT_QUEUE: RefCell<VecDeque<AppEvent>> = const { RefCell::new(VecDeque::new()) };
}

fn push_event(ev: AppEvent) {
    EVENT_QUEUE.with(|q| q.borrow_mut().push_back(ev));
}

fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn work_area_at(x: i32, y: i32, fallback_hwnd: HWND) -> WorkArea {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut mi) != 0 {
            let dpi_val = if !fallback_hwnd.is_null() {
                GetDpiForWindow(fallback_hwnd)
            } else {
                GetDpiForSystem()
            };
            let dpi = (dpi_val.max(96) as f32) / 96.0;
            return WorkArea {
                x: mi.rcWork.left as f32,
                y: mi.rcWork.top as f32,
                width: (mi.rcWork.right - mi.rcWork.left) as f32,
                height: (mi.rcWork.bottom - mi.rcWork.top) as f32,
                dpi,
            };
        }
    }
    WorkArea {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1040.0,
        dpi: 1.0,
    }
}

fn work_area_of_pet(pet: &Pet, hwnd: HWND) -> WorkArea {
    let cx = (pet.pos_x + pet.win_w as f32 * 0.5).round() as i32;
    let cy = (pet.pos_y + pet.win_h as f32 * 0.5).round() as i32;
    let mut area = work_area_at(cx, cy, hwnd);
    if !hwnd.is_null() {
        let win_dpi = unsafe { GetDpiForWindow(hwnd) };
        if win_dpi >= 96 {
            area.dpi = win_dpi as f32 / 96.0;
        }
    }
    area
}

struct LayeredWindow {
    hwnd: HWND,
    mem_dc: HDC,
    hbmp: HBITMAP,
    old_bmp: HGDIOBJ,
    bits: *mut u8,
    width: u32,
    height: u32,
    click_through: bool,
}

impl LayeredWindow {
    fn new(class_name: &[u16], title: &[u16], x: i32, y: i32, w: u32, h: u32, click_through: bool) -> Result<Self, String> {
        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let mut ex_style = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
            if click_through {
                ex_style |= WS_EX_TRANSPARENT;
            }
            let hwnd = CreateWindowExW(
                ex_style,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                x,
                y,
                w as i32,
                h as i32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null_mut(),
            );
            if hwnd.is_null() {
                return Err("CreateWindowExW failed".to_string());
            }

            let mem_dc = CreateCompatibleDC(std::ptr::null_mut());
            let mut win = Self {
                hwnd,
                mem_dc,
                hbmp: std::ptr::null_mut(),
                old_bmp: std::ptr::null_mut(),
                bits: std::ptr::null_mut(),
                width: 0,
                height: 0,
                click_through,
            };
            win.ensure_dib(w, h);
            Ok(win)
        }
    }

    fn ensure_dib(&mut self, w: u32, h: u32) {
        let w = w.max(1);
        let h = h.max(1);
        if self.width == w && self.height == h && !self.hbmp.is_null() {
            return;
        }
        unsafe {
            if !self.old_bmp.is_null() && !self.mem_dc.is_null() {
                SelectObject(self.mem_dc, self.old_bmp);
                self.old_bmp = std::ptr::null_mut();
            }
            if !self.hbmp.is_null() {
                DeleteObject(self.hbmp);
                self.hbmp = std::ptr::null_mut();
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
            let mut bits: *mut c_void = std::ptr::null_mut();
            let hbmp = CreateDIBSection(
                self.mem_dc,
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if !hbmp.is_null() && !bits.is_null() {
                self.old_bmp = SelectObject(self.mem_dc, hbmp);
                self.hbmp = hbmp;
                self.bits = bits as *mut u8;
                self.width = w;
                self.height = h;
            }
        }
    }

    fn present(&mut self, x: i32, y: i32, w: u32, h: u32, bgra: &[u8]) {
        self.ensure_dib(w, h);
        if self.bits.is_null() || self.mem_dc.is_null() {
            return;
        }
        let expected = (self.width * self.height * 4) as usize;
        if bgra.len() < expected {
            return;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), self.bits, expected);
            let pt_dst = POINT { x, y };
            let size = SIZE {
                cx: self.width as i32,
                cy: self.height as i32,
            };
            let pt_src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                self.hwnd,
                std::ptr::null_mut(),
                &pt_dst,
                &size,
                self.mem_dc,
                &pt_src,
                0,
                &blend,
                ULW_ALPHA,
            );
        }
    }

    fn show_inactive(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    fn set_click_through(&mut self, click_through: bool) {
        if self.click_through == click_through {
            return;
        }
        self.click_through = click_through;
        unsafe {
            let mut ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            if click_through {
                ex |= WS_EX_TRANSPARENT as isize;
            } else {
                ex &= !(WS_EX_TRANSPARENT as isize);
            }
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex);
        }
    }

    fn move_top(&self) {
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

impl Drop for LayeredWindow {
    fn drop(&mut self) {
        unsafe {
            if !self.old_bmp.is_null() && !self.mem_dc.is_null() {
                SelectObject(self.mem_dc, self.old_bmp);
            }
            if !self.hbmp.is_null() {
                DeleteObject(self.hbmp);
            }
            if !self.mem_dc.is_null() {
                DeleteDC(self.mem_dc);
            }
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
        }
    }
}

struct PetInstance {
    pet: Pet,
    win: LayeredWindow,
}

enum EffectKind {
    BlackHole(BlackHoleEffect),
    Crack(CrackEffect),
}

struct ActiveEffect {
    win: LayeredWindow,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    started: Instant,
    bgra: Vec<u8>,
    effect: EffectKind,
}

unsafe extern "system" fn pet_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_NCHITTEST => HTCLIENT as LRESULT,
        WM_LBUTTONDOWN => {
            SetCapture(hwnd);
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            push_event(AppEvent::PointerDown {
                hwnd: hwnd as isize,
                x: pt.x,
                y: pt.y,
            });
            0
        }
        WM_MOUSEMOVE => {
            if GetCapture() == hwnd {
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                push_event(AppEvent::PointerMove {
                    hwnd: hwnd as isize,
                    x: pt.x,
                    y: pt.y,
                });
            }
            0
        }
        WM_LBUTTONUP => {
            push_event(AppEvent::PointerUp {
                hwnd: hwnd as isize,
                is_click: true,
            });
            ReleaseCapture();
            0
        }
        WM_CAPTURECHANGED => {
            push_event(AppEvent::PointerUp {
                hwnd: hwnd as isize,
                is_click: false,
            });
            0
        }
        WM_RBUTTONUP => {
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            push_event(AppEvent::RightClick {
                hwnd: hwnd as isize,
                x: pt.x,
                y: pt.y,
            });
            0
        }
        WM_DISPLAYCHANGE => {
            push_event(AppEvent::DisplayChanged);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn helper_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            let ev = (lparam as u32) & 0xFFFF;
            if ev == WM_LBUTTONUP || ev == WM_RBUTTONUP {
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                push_event(AppEvent::TrayMenu { x: pt.x, y: pt.y });
            }
            0
        }
        WM_DISPLAYCHANGE => {
            push_event(AppEvent::DisplayChanged);
            0
        }
        WM_CLOSE | WM_DESTROY => {
            push_event(AppEvent::Quit);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn create_icon_from_pet(pet: &Pet) -> HICON {
    let win_w = pet.win_w as usize;
    let win_h = pet.win_h as usize;
    let box_px = pet.pet_size_px.round().max(1.0) as i32;
    let sx0 = ((pet.win_w as i32 - box_px) / 2).max(0);
    let sy0 = (pet.win_h as i32 - box_px).max(0);

    let mut icon_bgra = [0u8; 32 * 32 * 4];
    for dy in 0..32i32 {
        let sy = sy0 + ((dy as f32 + 0.5) * box_px as f32 / 32.0).floor() as i32;
        if sy < 0 || sy >= win_h as i32 {
            continue;
        }
        for dx in 0..32i32 {
            let sx = sx0 + ((dx as f32 + 0.5) * box_px as f32 / 32.0).floor() as i32;
            if sx < 0 || sx >= win_w as i32 {
                continue;
            }
            let s_off = (sy as usize * win_w + sx as usize) * 4;
            let d_off = (dy as usize * 32 + dx as usize) * 4;
            icon_bgra[d_off..d_off + 4].copy_from_slice(&pet.bgra_buf[s_off..s_off + 4]);
        }
    }

    unsafe {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 32,
            biHeight: -32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let hbm_color = CreateDIBSection(
            std::ptr::null_mut(),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if hbm_color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(icon_bgra.as_ptr(), bits as *mut u8, icon_bgra.len());
        let hbm_mask = CreateBitmap(32, 32, 1, 1, std::ptr::null());
        let ii = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };
        let hicon = CreateIconIndirect(&ii);
        DeleteObject(hbm_color);
        if !hbm_mask.is_null() {
            DeleteObject(hbm_mask);
        }
        hicon
    }
}

pub struct Win32App {
    char_dir: PathBuf,
    settings: Settings,
    pet_class: Vec<u16>,
    effect_class: Vec<u16>,
    helper_hwnd: HWND,
    tray_icon: HICON,
    tray_added: bool,
    pets: HashMap<String, PetInstance>,
    effects: HashMap<&'static str, ActiveEffect>,
    meet_again: HashMap<(String, String), Instant>,
    text_renderer: TextRenderer,
    control_panel: ControlPanel,
    last_cursor: Option<(i32, i32)>,
    running: bool,
}

impl Win32App {
    pub fn new() -> Result<Self, String> {
        let char_dir = find_characters_dir();
        let settings = Settings::load();

        let pet_class = wide_null("DesktopPetWindowClass");
        let effect_class = wide_null("DesktopPetEffectClass");
        let helper_class = wide_null("DesktopPetHelperClass");

        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let cursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);

            let wc_pet = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(pet_wnd_proc),
                hInstance: hinst,
                hCursor: cursor,
                lpszClassName: pet_class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassExW(&wc_pet);

            let wc_effect = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(pet_wnd_proc),
                hInstance: hinst,
                lpszClassName: effect_class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassExW(&wc_effect);

            let wc_helper = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(helper_wnd_proc),
                hInstance: hinst,
                lpszClassName: helper_class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassExW(&wc_helper);

            let helper_title = wide_null("Desktop Pet");
            let helper_hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                helper_class.as_ptr(),
                helper_title.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null_mut(),
            );

            let mut app = Self {
                char_dir,
                settings,
                pet_class,
                effect_class,
                helper_hwnd,
                tray_icon: std::ptr::null_mut(),
                tray_added: false,
                pets: HashMap::new(),
                effects: HashMap::new(),
                meet_again: HashMap::new(),
                text_renderer: TextRenderer::new(),
                control_panel: ControlPanel::new(),
                last_cursor: None,
                running: true,
            };
            app.sync_pets();
            Ok(app)
        }
    }

    fn sync_pets(&mut self) {
        let chosen = chosen_characters(&self.char_dir, &self.settings);
        let chosen_ids: Vec<String> = chosen.iter().map(|c| c.id.clone()).collect();

        self.pets.retain(|id, _| chosen_ids.contains(id));

        let primary_area = work_area_at(0, 0, self.helper_hwnd);
        let now = Instant::now();
        let mut added_any = false;

        for def in chosen {
            if self.pets.contains_key(&def.id) {
                continue;
            }
            let pset = self.settings.pet_settings(&def.id);
            let Ok(mut pet) = Pet::new(def, pset, primary_area) else {
                continue;
            };
            let title = wide_null(pet.def.display_name());
            let Ok(mut win) = LayeredWindow::new(
                &self.pet_class,
                &title,
                pet.pos_x.round() as i32,
                pet.pos_y.round() as i32,
                pet.win_w,
                pet.win_h,
                true,
            ) else {
                continue;
            };
            let real_area = work_area_of_pet(&pet, win.hwnd);
            if (real_area.dpi - pet.area.dpi).abs() > 0.01 {
                pet.update_area(real_area);
            }
            pet.render(&mut self.text_renderer, now);
            win.present(
                pet.pos_x.round() as i32,
                pet.pos_y.round() as i32,
                pet.win_w,
                pet.win_h,
                &pet.bgra_buf,
            );
            win.show_inactive();

            if !self.tray_added || added_any || self.tray_icon.is_null() {
                let icon = create_icon_from_pet(&pet);
                self.set_tray_hicon(icon);
            }
            self.pets.insert(pet.id.clone(), PetInstance { pet, win });
            added_any = true;
        }

        if !self.tray_added {
            let icon = self.pets.values().next().map(|first| create_icon_from_pet(&first.pet));
            if let Some(hicon) = icon {
                self.set_tray_hicon(hicon);
            }
        }
    }

    fn set_tray_hicon(&mut self, new_icon: HICON) {
        if new_icon.is_null() {
            return;
        }
        unsafe {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.helper_hwnd;
            nid.uID = TRAY_ICON_ID;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = new_icon;
            let tip: Vec<u16> = "Desktop Pet\0".encode_utf16().collect();
            let len = tip.len().min(nid.szTip.len());
            nid.szTip[..len].copy_from_slice(&tip[..len]);

            let msg = if self.tray_added { NIM_MODIFY } else { NIM_ADD };
            if Shell_NotifyIconW(msg, &nid) != 0 {
                self.tray_added = true;
                if !self.tray_icon.is_null() {
                    DestroyIcon(self.tray_icon);
                }
                self.tray_icon = new_icon;
            } else {
                DestroyIcon(new_icon);
            }
        }
    }

    fn open_effect(&mut self, req: EffectRequest, creator_hwnd: HWND) {
        match req {
            EffectRequest::BlackHole {
                x,
                y,
                size,
                seconds,
            } => {
                if self.effects.contains_key("blackhole") {
                    return;
                }
                let screen_patch = capture_screen_patch(x, y, size, size);
                let title = wide_null("Black hole");
                let Ok(mut win) =
                    LayeredWindow::new(&self.effect_class, &title, x, y, size, size, true)
                else {
                    return;
                };
                let effect = BlackHoleEffect::new(size, size, seconds, screen_patch);
                let mut bgra = vec![0u8; (size * size * 4) as usize];
                effect.render(0.0, &mut bgra);
                win.present(x, y, size, size, &bgra);
                win.show_inactive();
                unsafe {
                    SetWindowPos(
                        creator_hwnd,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
                self.effects.insert(
                    "blackhole",
                    ActiveEffect {
                        win,
                        x,
                        y,
                        w: size,
                        h: size,
                        started: Instant::now(),
                        bgra,
                        effect: EffectKind::BlackHole(effect),
                    },
                );
            }
            EffectRequest::Crack {
                x,
                y,
                width,
                height,
                seconds,
                dpi,
            } => {
                if self.effects.contains_key("crack") {
                    return;
                }
                let title = wide_null("Crack");
                let Ok(mut win) =
                    LayeredWindow::new(&self.effect_class, &title, x, y, width, height, true)
                else {
                    return;
                };
                let mut effect = CrackEffect::new(width, height, seconds, dpi);
                let mut bgra = vec![0u8; (width * height * 4) as usize];
                effect.render(0.0, &mut bgra);
                win.present(x, y, width, height, &bgra);
                win.show_inactive();
                unsafe {
                    SetWindowPos(
                        creator_hwnd,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
                self.effects.insert(
                    "crack",
                    ActiveEffect {
                        win,
                        x,
                        y,
                        w: width,
                        h: height,
                        started: Instant::now(),
                        bgra,
                        effect: EffectKind::Crack(effect),
                    },
                );
            }
        }
    }

    fn keep_company(&mut self, now: Instant) {
        let mut snapshot: Vec<OtherPet> = self
            .pets
            .values()
            .map(|inst| {
                let (dance_kind, dance_elapsed) = inst.pet.active_dance_info();
                OtherPet {
                    id: inst.pet.id.clone(),
                    x: inst.pet.middle_x(),
                    y: inst.pet.pos_y,
                    state: inst.pet.state.clone(),
                    dpi: inst.pet.area.dpi,
                    dance_kind,
                    dance_elapsed,
                }
            })
            .collect();

        for inst in self.pets.values_mut() {
            let others: Vec<OtherPet> = snapshot
                .iter()
                .filter(|o| o.id != inst.pet.id)
                .cloned()
                .collect();
            inst.pet.set_others(others);
        }

        snapshot.sort_by(|a, b| a.id.cmp(&b.id));
        let meetings = [
            ("greet", 3.4_f32),
            ("chat", 4.8),
            ("dance", 5.0),
            ("butt_dance", 5.6),
            ("butt_dance", 5.6),
            ("boogie", 5.0),
            ("spin_dance", 4.4),
        ];

        for i in 0..snapshot.len() {
            for j in (i + 1)..snapshot.len() {
                let a_state = &snapshot[i].state;
                let b_state = &snapshot[j].state;
                if !(a_state == "idle" || a_state == "walk")
                    || !(b_state == "idle" || b_state == "walk")
                {
                    continue;
                }
                let pair = (snapshot[i].id.clone(), snapshot[j].id.clone());
                if self.meet_again.get(&pair).is_some_and(|&t| now < t) {
                    continue;
                }
                let dpi = snapshot[i].dpi.max(snapshot[j].dpi);
                if (snapshot[i].x - snapshot[j].x).abs() > 150.0 * dpi
                    || (snapshot[i].y - snapshot[j].y).abs() > 30.0 * dpi
                {
                    continue;
                }
                let (kind, seconds) = meetings[fastrand::usize(..meetings.len())];
                if let Some(inst_a) = self.pets.get_mut(&pair.0) {
                    inst_a.pet.on_meet(MeetRequest {
                        with: pair.1.clone(),
                        kind: kind.to_string(),
                        seconds,
                        lead: true,
                    });
                }
                if let Some(inst_b) = self.pets.get_mut(&pair.1) {
                    inst_b.pet.on_meet(MeetRequest {
                        with: pair.0.clone(),
                        kind: kind.to_string(),
                        seconds,
                        lead: false,
                    });
                }
                snapshot[i].state = "meet".to_string();
                snapshot[j].state = "meet".to_string();
                let wait_secs = 14.0 + fastrand::f32() * 12.0;
                self.meet_again
                    .insert(pair, now + Duration::from_secs_f32(wait_secs));
            }
        }
    }

    fn show_context_menu(&mut self, pet_id: Option<String>, x: i32, y: i32) {
        let all_chars = load_characters(&self.char_dir);
        let chosen_ids: Vec<String> = chosen_characters(&self.char_dir, &self.settings)
            .into_iter()
            .map(|c| c.id)
            .collect();

        enum MenuAction {
            OpenControlPanel,
            ButtDance(String),
            DanceParty,
            ToggleChar(String),
            SetSize(String, f32),
            ToggleStay(String),
            ToggleSleep(String),
            ToggleMute(String),
            ToggleChase(String),
            OpenFolder,
            Quit,
        }

        let mut actions: HashMap<u32, MenuAction> = HashMap::new();
        let mut next_cmd: u32 = 100;

        unsafe {
            let root = CreatePopupMenu();

            let panel_cmd = next_cmd;
            next_cmd += 1;
            let panel_lbl = wide_null("Control Panel...");
            AppendMenuW(root, MF_STRING, panel_cmd as usize, panel_lbl.as_ptr());
            actions.insert(panel_cmd, MenuAction::OpenControlPanel);

            if let Some(id) = &pet_id {
                let bd_cmd = next_cmd;
                next_cmd += 1;
                let bd_lbl = wide_null("Butt Dance!");
                AppendMenuW(root, MF_STRING, bd_cmd as usize, bd_lbl.as_ptr());
                actions.insert(bd_cmd, MenuAction::ButtDance(id.clone()));
            }
            let party_cmd = next_cmd;
            next_cmd += 1;
            let party_lbl = wide_null("Dance Party!");
            AppendMenuW(root, MF_STRING, party_cmd as usize, party_lbl.as_ptr());
            actions.insert(party_cmd, MenuAction::DanceParty);

            AppendMenuW(root, MF_SEPARATOR, 0, std::ptr::null());

            let char_sub = CreatePopupMenu();

            for c in &all_chars {
                let checked = chosen_ids.contains(&c.id);
                let disabled = chosen_ids.len() == 1 && chosen_ids[0] == c.id;
                let mut flags = MF_STRING;
                flags |= if checked { MF_CHECKED } else { MF_UNCHECKED };
                flags |= if disabled { MF_GRAYED } else { MF_ENABLED };
                let cmd = next_cmd;
                next_cmd += 1;
                let label = wide_null(c.display_name());
                AppendMenuW(char_sub, flags, cmd as usize, label.as_ptr());
                actions.insert(cmd, MenuAction::ToggleChar(c.id.clone()));
            }
            let chars_lbl = wide_null("Characters");
            AppendMenuW(root, MF_POPUP, char_sub as usize, chars_lbl.as_ptr());

            if let Some(id) = &pet_id {
                let own = self.settings.pet_settings(id);
                let size_sub = CreatePopupMenu();
                let first_size_cmd = next_cmd;
                let mut selected_size_cmd = None;
                for &(lbl, scale) in SIZES {
                    let cmd = next_cmd;
                    next_cmd += 1;
                    let wlbl = wide_null(lbl);
                    AppendMenuW(size_sub, MF_STRING, cmd as usize, wlbl.as_ptr());
                    if (scale - own.scale).abs() < 0.01 {
                        selected_size_cmd = Some(cmd);
                    }
                    actions.insert(cmd, MenuAction::SetSize(id.clone(), scale));
                }
                let last_size_cmd = next_cmd - 1;
                if let Some(sel) = selected_size_cmd {
                    CheckMenuRadioItem(size_sub, first_size_cmd, last_size_cmd, sel, MF_BYCOMMAND);
                }
                let size_lbl = wide_null("Size");
                AppendMenuW(root, MF_POPUP, size_sub as usize, size_lbl.as_ptr());

                let toggles = [
                    ("Stay", own.stay, MenuAction::ToggleStay(id.clone())),
                    ("Sleep", own.sleep, MenuAction::ToggleSleep(id.clone())),
                    ("Mute", own.mute, MenuAction::ToggleMute(id.clone())),
                    ("Chase mouse", own.chase, MenuAction::ToggleChase(id.clone())),
                ];
                for (lbl, is_checked, action) in toggles {
                    let cmd = next_cmd;
                    next_cmd += 1;
                    let flags = MF_STRING | if is_checked { MF_CHECKED } else { MF_UNCHECKED };
                    let wlbl = wide_null(lbl);
                    AppendMenuW(root, flags, cmd as usize, wlbl.as_ptr());
                    actions.insert(cmd, action);
                }
            }

            AppendMenuW(root, MF_SEPARATOR, 0, std::ptr::null());
            let open_cmd = next_cmd;
            next_cmd += 1;
            let open_lbl = wide_null("Open characters folder");
            AppendMenuW(root, MF_STRING, open_cmd as usize, open_lbl.as_ptr());
            actions.insert(open_cmd, MenuAction::OpenFolder);

            AppendMenuW(root, MF_SEPARATOR, 0, std::ptr::null());
            let quit_cmd = next_cmd;
            let quit_lbl = wide_null("Quit");
            AppendMenuW(root, MF_STRING, quit_cmd as usize, quit_lbl.as_ptr());
            actions.insert(quit_cmd, MenuAction::Quit);

            SetForegroundWindow(self.helper_hwnd);
            let chosen_cmd = TrackPopupMenu(
                root,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                x,
                y,
                0,
                self.helper_hwnd,
                std::ptr::null(),
            ) as u32;
            PostMessageW(self.helper_hwnd, WM_NULL, 0, 0);
            DestroyMenu(root);

            if let Some(action) = actions.remove(&chosen_cmd) {
                match action {
                    MenuAction::OpenControlPanel => {
                        self.control_panel.open(
                            pet_id.as_deref(),
                            &self.char_dir,
                            &self.settings,
                            &mut self.text_renderer,
                        );
                    }
                    MenuAction::ButtDance(pid) => {
                        if let Some(inst) = self.pets.get_mut(&pid) {
                            inst.pet.start_dance("butt_dance", 5.8);
                        }
                    }
                    MenuAction::DanceParty => {
                        for inst in self.pets.values_mut() {
                            inst.pet.start_dance("butt_dance", 6.2);
                        }
                    }
                    MenuAction::ToggleChar(cid) => {
                        self.toggle_character(&cid);
                    }
                    MenuAction::SetSize(pid, scale) => {
                        self.change_pet_setting(&pid, |s| s.scale = scale);
                    }
                    MenuAction::ToggleStay(pid) => {
                        self.change_pet_setting(&pid, |s| s.stay = !s.stay);
                    }
                    MenuAction::ToggleSleep(pid) => {
                        self.change_pet_setting(&pid, |s| s.sleep = !s.sleep);
                    }
                    MenuAction::ToggleMute(pid) => {
                        self.change_pet_setting(&pid, |s| s.mute = !s.mute);
                    }
                    MenuAction::ToggleChase(pid) => {
                        self.change_pet_setting(&pid, |s| s.chase = !s.chase);
                    }
                    MenuAction::OpenFolder => {
                        self.open_characters_folder();
                    }
                    MenuAction::Quit => {
                        self.running = false;
                        PostQuitMessage(0);
                    }
                }
            }
        }
    }

    fn toggle_character(&mut self, cid: &str) {
        let chosen_ids: Vec<String> = chosen_characters(&self.char_dir, &self.settings)
            .into_iter()
            .map(|c| c.id)
            .collect();
        if chosen_ids.iter().any(|id| id == cid) {
            if chosen_ids.len() <= 1 {
                return;
            }
            self.settings.characters = chosen_ids
                .into_iter()
                .filter(|other| other != cid)
                .collect();
        } else {
            let mut next = chosen_ids;
            next.push(cid.to_string());
            self.settings.characters = next;
        }
        self.settings.save();
        self.sync_pets();
        self.control_panel.mark_dirty();
    }

    fn open_characters_folder(&self) {
        unsafe {
            let op = wide_null("open");
            let path = wide_null(&self.char_dir.to_string_lossy());
            ShellExecuteW(
                std::ptr::null_mut(),
                op.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOW,
            );
        }
    }

    fn handle_panel_actions(&mut self) {
        let actions =
            self.control_panel
                .poll_events(&self.char_dir, &self.settings, &mut self.text_renderer);
        for action in actions {
            match action {
                PanelAction::ToggleCharacter(cid) => {
                    self.toggle_character(&cid);
                }
                PanelAction::ChangePetSettings {
                    id,
                    settings,
                    save_to_disk,
                } => {
                    self.settings.pets.insert(id.clone(), settings.clone());
                    if save_to_disk {
                        self.settings.save();
                    }
                    if let Some(inst) = self.pets.get_mut(&id) {
                        inst.pet.apply_settings(settings);
                    }
                }
                PanelAction::TriggerDance(id) => {
                    if let Some(inst) = self.pets.get_mut(&id) {
                        inst.pet.start_dance("butt_dance", 5.2);
                    }
                }
                PanelAction::BringToFloor(id) => {
                    if let Some(inst) = self.pets.get_mut(&id) {
                        let area = work_area_of_pet(&inst.pet, inst.win.hwnd);
                        inst.pet.bring_to_floor(area);
                    }
                }
                PanelAction::OpenCharactersFolder => {
                    self.open_characters_folder();
                }
                PanelAction::QuitApp => {
                    self.running = false;
                    unsafe { PostQuitMessage(0) };
                }
            }
        }
    }

    fn change_pet_setting(&mut self, id: &str, update: impl FnOnce(&mut crate::config::PetSettings)) {
        let mut pset = self.settings.pet_settings(id);
        update(&mut pset);
        self.settings.pets.insert(id.to_string(), pset.clone());
        self.settings.save();
        if let Some(inst) = self.pets.get_mut(id) {
            inst.pet.apply_settings(pset);
        }
        self.control_panel.mark_dirty();
    }

    fn find_pet_id_by_hwnd(&self, hwnd: isize) -> Option<String> {
        self.pets
            .iter()
            .find(|(_, inst)| (inst.win.hwnd as isize) == hwnd)
            .map(|(id, _)| id.clone())
    }

    fn drain_events(&mut self) {
        let events: Vec<AppEvent> = EVENT_QUEUE.with(|q| q.borrow_mut().drain(..).collect());
        for ev in events {
            match ev {
                AppEvent::PointerDown { hwnd, x, y } => {
                    if let Some(id) = self.find_pet_id_by_hwnd(hwnd) {
                        if let Some(inst) = self.pets.get_mut(&id) {
                            inst.pet.on_pointer_down(x as f32, y as f32);
                            inst.win.set_click_through(false);
                            inst.win.move_top();
                        }
                    }
                }
                AppEvent::PointerMove { hwnd, x, y } => {
                    if let Some(id) = self.find_pet_id_by_hwnd(hwnd) {
                        if let Some(inst) = self.pets.get_mut(&id) {
                            inst.pet.on_pointer_move(x as f32, y as f32);
                        }
                    }
                }
                AppEvent::PointerUp { hwnd, is_click } => {
                    if let Some(id) = self.find_pet_id_by_hwnd(hwnd) {
                        if let Some(inst) = self.pets.get_mut(&id) {
                            let drop_area = work_area_of_pet(&inst.pet, inst.win.hwnd);
                            inst.pet.on_pointer_up(is_click, drop_area);
                        }
                    }
                }
                AppEvent::RightClick { hwnd, x, y } => {
                    if let Some(id) = self.find_pet_id_by_hwnd(hwnd) {
                        self.show_context_menu(Some(id), x, y);
                    }
                }
                AppEvent::TrayMenu { x, y } => {
                    self.show_context_menu(None, x, y);
                }
                AppEvent::DisplayChanged => {
                    for inst in self.pets.values_mut() {
                        let area = work_area_of_pet(&inst.pet, inst.win.hwnd);
                        inst.pet.update_area(area);
                    }
                }
                AppEvent::Quit => {
                    self.running = false;
                    unsafe { PostQuitMessage(0) };
                }
            }
        }
    }

    fn watch_cursor(&mut self, now: Instant) {
        let mut pt = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut pt) } == 0 {
            return;
        }
        let cx = pt.x as f32;
        let cy = pt.y as f32;

        for inst in self.pets.values_mut() {
            let over = inst.pet.hit_test(cx, cy);
            if over != inst.pet.over {
                inst.pet.over = over;
                inst.win.set_click_through(!over);
            }
        }

        let pos = (pt.x, pt.y);
        if self.last_cursor != Some(pos) {
            self.last_cursor = Some(pos);
            for inst in self.pets.values_mut() {
                inst.pet.set_cursor(cx, cy, now);
            }
        }
    }

    pub fn run(&mut self) {
        let _timer_guard = TimerPeriodGuard::new();
        let frame_interval = Duration::from_micros(16_667);
        let mut last_tick = Instant::now();
        let mut last_company = Instant::now();

        while self.running {
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    if msg.message == WM_QUIT {
                        self.running = false;
                        break;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if !self.running {
                break;
            }

            self.drain_events();
            if !self.running {
                break;
            }
            self.handle_panel_actions();
            if !self.running {
                break;
            }

            let now = Instant::now();
            self.watch_cursor(now);

            if now.duration_since(last_company) >= Duration::from_millis(100) {
                last_company = now;
                self.keep_company(now);
            }

            let dt = now.duration_since(last_tick).as_secs_f32();
            if now.duration_since(last_tick) >= Duration::from_micros(15_500) {
                last_tick = now;

                let mut spawned: Vec<(EffectRequest, HWND)> = Vec::new();
                for inst in self.pets.values_mut() {
                    inst.pet.tick(dt, now);
                    for req in inst.pet.pending_effects.drain(..) {
                        spawned.push((req, inst.win.hwnd));
                    }
                    inst.pet.render(&mut self.text_renderer, now);
                    inst.win.present(
                        inst.pet.pos_x.round() as i32,
                        inst.pet.pos_y.round() as i32,
                        inst.pet.win_w,
                        inst.pet.win_h,
                        &inst.pet.bgra_buf,
                    );
                }

                for (req, creator_hwnd) in spawned {
                    self.open_effect(req, creator_hwnd);
                }

                let mut finished_effects = Vec::new();
                for (&key, eff) in self.effects.iter_mut() {
                    let t = now.duration_since(eff.started).as_secs_f32();
                    match &mut eff.effect {
                        EffectKind::BlackHole(bh) => {
                            if bh.render(t, &mut eff.bgra) {
                                eff.win.present(eff.x, eff.y, eff.w, eff.h, &eff.bgra);
                            } else {
                                finished_effects.push(key);
                            }
                        }
                        EffectKind::Crack(cr) => match cr.render(t, &mut eff.bgra) {
                            CrackRenderResult::Updated => {
                                eff.win.present(eff.x, eff.y, eff.w, eff.h, &eff.bgra);
                            }
                            CrackRenderResult::Unchanged => {}
                            CrackRenderResult::Finished => {
                                finished_effects.push(key);
                            }
                        },
                    }
                }
                for key in finished_effects {
                    self.effects.remove(key);
                }
            }

            let next_tick = last_tick + frame_interval;
            let wait_ms = next_tick
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(16) as u32;
            unsafe {
                MsgWaitForMultipleObjectsEx(
                    0,
                    std::ptr::null(),
                    wait_ms,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                );
            }
        }
    }
}

impl Drop for Win32App {
    fn drop(&mut self) {
        unsafe {
            if self.tray_added {
                let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
                nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = self.helper_hwnd;
                nid.uID = TRAY_ICON_ID;
                Shell_NotifyIconW(NIM_DELETE, &nid);
            }
            if !self.tray_icon.is_null() {
                DestroyIcon(self.tray_icon);
            }
            if !self.helper_hwnd.is_null() {
                DestroyWindow(self.helper_hwnd);
            }
        }
    }
}
