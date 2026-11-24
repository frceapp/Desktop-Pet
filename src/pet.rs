use std::time::{Duration, Instant};

use crate::config::{CharacterDef, PetSettings, WIN_H, WIN_W};
use crate::effects::EffectRequest;
use crate::model_view::{ModelView, BASE_FRAME, FRAME};
use crate::sprite_view::SpriteView;
use crate::ui_overlay::{draw_bubble, draw_zzz, TextRenderer};

const GRAVITY: f32 = 2600.0;
const DRAG_THRESHOLD: f32 = 6.0;
const NOTICE: f32 = 170.0;
const BORED_AFTER: f32 = 4.0;
const TIRED_AFTER: f32 = 7.0;
const REST: f32 = 5.0;

#[derive(Debug, Clone, Copy)]
pub struct WorkArea {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub dpi: f32,
}

#[derive(Debug, Clone)]
pub struct OtherPet {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub state: String,
    pub dpi: f32,
    pub dance_kind: Option<String>,
    pub dance_elapsed: f32,
}

pub fn is_dance_state(state: &str) -> bool {
    matches!(
        state,
        "dance" | "butt_dance" | "boogie" | "spin" | "spin_dance" | "excited"
    )
}

#[derive(Debug, Clone)]
pub struct MeetRequest {
    pub with: String,
    pub kind: String,
    pub seconds: f32,
    pub lead: bool,
}

struct DragPress {
    sx: f32,
    sy: f32,
    px: f32,
    py: f32,
    dragging: bool,
}

struct MeetingState {
    with: String,
    kind: String,
    lead: bool,
    time: f32,
    spoken: usize,
}

struct ComfortState {
    with: String,
    said: bool,
}

struct BubbleState {
    text: String,
    shown_at: Instant,
    expires_at: Instant,
}

pub enum PetView {
    Model(ModelView),
    Sprite(SpriteView),
}

impl PetView {
    pub fn width_ratio(&self) -> f32 {
        match self {
            PetView::Model(m) => m.width_ratio(),
            PetView::Sprite(s) => s.width_ratio(),
        }
    }

    pub fn height_ratio(&self) -> f32 {
        match self {
            PetView::Model(m) => m.height_ratio(),
            PetView::Sprite(s) => s.height_ratio(),
        }
    }

    pub fn set_state(&mut self, next: &str) {
        match self {
            PetView::Model(m) => m.set_state(next),
            PetView::Sprite(s) => s.set_state(next),
        }
    }

    pub fn set_facing(&mut self, dir: i32) {
        match self {
            PetView::Model(m) => m.set_facing(dir),
            PetView::Sprite(s) => s.set_facing(dir),
        }
    }

    pub fn set_sitting(&mut self, sitting: bool) {
        match self {
            PetView::Model(m) => m.set_sitting(sitting),
            PetView::Sprite(s) => s.set_sitting(sitting),
        }
    }

    pub fn set_speed_scale(&mut self, scale: f32) {
        if let PetView::Model(m) = self {
            m.set_speed_scale(scale);
        }
    }

    pub fn update(&mut self, dt: f32) {
        match self {
            PetView::Model(m) => m.update(dt),
            PetView::Sprite(s) => s.update(dt),
        }
    }

    pub fn sync_phase(&mut self, elapsed: f32) {
        match self {
            PetView::Model(m) => m.sync_phase(elapsed),
            PetView::Sprite(s) => s.sync_phase(elapsed),
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
        match self {
            PetView::Model(m) => m.render(dst_bgra, dst_w, dst_h, dst_x, dst_y, box_px),
            PetView::Sprite(s) => s.render(dst_bgra, dst_w, dst_h, dst_x, dst_y, box_px),
        }
    }
}

#[inline]
fn rand_range(min: f32, max: f32) -> f32 {
    min + fastrand::f32() * (max - min)
}

pub struct Pet {
    pub id: String,
    pub def: CharacterDef,
    pub settings: PetSettings,
    pub view: PetView,
    pub area: WorkArea,
    pub win_w: u32,
    pub win_h: u32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pet_size_px: f32,

    pub state: String,
    state_time: f32,
    facing: i32,
    target_x: f32,
    fall_speed: f32,
    fall_from: f32,
    sob_timer: f32,
    clicks: Vec<Instant>,
    tantrums: usize,

    press: Option<DragPress>,
    bubble: Option<BubbleState>,
    cursor: Option<(f32, f32, Instant)>,
    ignore_pointer_until: Option<Instant>,
    noticed_at: Option<Instant>,
    others: Vec<OtherPet>,
    meeting: Option<MeetingState>,
    comforting: Option<ComfortState>,
    comfort_again_at: Option<Instant>,
    joining_dance: Option<(String, String)>,
    join_dance_cooldown: Option<Instant>,
    pub dance_elapsed: f32,
    sleep_time: f32,

    pub over: bool,
    pub bgra_buf: Vec<u8>,
    pub pending_effects: Vec<EffectRequest>,
}

impl Pet {
    pub fn new(def: CharacterDef, settings: PetSettings, area: WorkArea) -> Result<Self, String> {
        let mut view = if let Some(model_cfg) = &def.config.model {
            PetView::Model(ModelView::load(&def.dir, model_cfg)?)
        } else if let Some(sprite_cfg) = &def.config.sprites {
            PetView::Sprite(SpriteView::load(&def.dir, sprite_cfg)?)
        } else {
            return Err(format!("Character {} has neither model nor sprites", def.id));
        };

        view.set_facing(1);
        view.set_sitting(settings.stay);
        view.set_speed_scale(settings.speed_scale);

        let win_w_f = (WIN_W * area.dpi).round();
        let win_h_f = (WIN_H * area.dpi).round();
        let start_x = area.x + rand_range(0.0, (area.width - win_w_f).max(0.0)).round();
        let start_y = area.y + area.height - win_h_f;

        let mut pet = Self {
            id: def.id.clone(),
            def,
            settings,
            view,
            area,
            win_w: win_w_f as u32,
            win_h: win_h_f as u32,
            pos_x: start_x,
            pos_y: start_y,
            pet_size_px: 150.0,
            state: "idle".to_string(),
            state_time: 2.0,
            facing: 1,
            target_x: start_x,
            fall_speed: 0.0,
            fall_from: 0.0,
            sob_timer: 0.0,
            clicks: Vec::new(),
            tantrums: 0,
            press: None,
            bubble: None,
            cursor: None,
            ignore_pointer_until: None,
            noticed_at: None,
            others: Vec::new(),
            meeting: None,
            comforting: None,
            comfort_again_at: None,
            joining_dance: None,
            join_dance_cooldown: None,
            dance_elapsed: 0.0,
            sleep_time: 0.0,
            over: false,
            bgra_buf: Vec::new(),
            pending_effects: Vec::new(),
        };

        pet.apply_size();
        pet.set_state("idle", 2.0);
        pet.view.update(0.0);
        Ok(pet)
    }

    #[inline]
    pub fn is_held(&self) -> bool {
        self.press.is_some()
    }

    #[inline]
    pub fn middle_x(&self) -> f32 {
        self.pos_x + self.win_w as f32 * 0.5
    }

    #[inline]
    pub fn body_width(&self) -> f32 {
        self.pet_size_px * self.view.width_ratio()
    }

    #[inline]
    pub fn body_height(&self) -> f32 {
        self.pet_size_px * self.view.height_ratio()
    }

    #[inline]
    fn margin(&self) -> f32 {
        (self.win_w as f32 - self.pet_size_px) * 0.5
    }

    #[inline]
    fn min_x(&self) -> f32 {
        self.area.x - self.margin()
    }

    #[inline]
    fn max_x(&self) -> f32 {
        self.area.x + self.area.width - self.win_w as f32 + self.margin()
    }

    #[inline]
    fn min_y(&self) -> f32 {
        self.area.y - (self.win_h as f32 - self.pet_size_px)
    }

    #[inline]
    fn floor_y(&self) -> f32 {
        self.area.y + self.area.height - self.win_h as f32
    }

    #[inline]
    fn beside(&self, other: &OtherPet) -> bool {
        (other.y - self.pos_y).abs() < 40.0 * self.area.dpi
    }

    #[inline]
    fn elbow_room(&self) -> f32 {
        self.pet_size_px * 0.55 + 32.0 * self.area.dpi
    }

    #[inline]
    fn speed_px(&self) -> f32 {
        self.def.config.speed * self.settings.speed_scale * self.area.dpi
    }

    pub fn hit_test(&self, cursor_x: f32, cursor_y: f32) -> bool {
        if self.is_held() {
            return true;
        }
        let dpi = self.area.dpi;
        let dx = (cursor_x - self.middle_x()) / (self.body_width() * 0.5 + 3.0 * dpi);
        let dy = (cursor_y - (self.pos_y + self.win_h as f32 - self.body_height() * 0.5))
            / (self.body_height() * 0.5 + 3.0 * dpi);
        dx * dx + dy * dy < 1.0
    }

    pub fn apply_size(&mut self) {
        let dpi = self.area.dpi;
        let win_w = (WIN_W * dpi).round();
        let win_h = (WIN_H * dpi).round();
        self.win_w = win_w as u32;
        self.win_h = win_h as u32;
        self.pet_size_px = (self.def.config.size * dpi * self.settings.scale)
            .min(win_w - 20.0 * dpi)
            .min(win_h - 90.0 * dpi)
            .round()
            .max(16.0);
        self.bgra_buf.resize(self.win_w as usize * self.win_h as usize * 4, 0);
        self.pos_x = self.pos_x.clamp(self.min_x(), self.max_x());
    }

    pub fn apply_settings(&mut self, next: PetSettings) {
        let released = self.settings.stay && !next.stay;
        self.settings = next;
        self.apply_size();
        self.view.set_sitting(self.settings.stay);
        self.view.set_speed_scale(self.settings.speed_scale);
        if self.settings.mute {
            self.bubble = None;
        }
        if self.settings.stay && self.state == "walk" {
            self.set_state("idle", rand_range(1.0, 3.0));
        }
        if self.settings.sleep && matches!(self.state.as_str(), "idle" | "walk" | "talk") {
            self.bubble = None;
            self.set_state("idle", f32::INFINITY);
        } else if !self.settings.sleep && self.state == "sleep" {
            self.set_state("idle", rand_range(1.0, 2.0));
        }
        if released
            && matches!(self.state.as_str(), "idle" | "sleep")
            && self.pos_y < self.floor_y()
        {
            self.start_fall();
        }
    }

    pub fn update_area(&mut self, area: WorkArea) {
        self.area = area;
        self.apply_size();
        self.pos_x = self.pos_x.clamp(self.min_x(), self.max_x());
        self.pos_y = if self.settings.stay {
            self.pos_y.clamp(self.min_y(), self.floor_y())
        } else {
            self.floor_y()
        };
    }

    pub fn bring_to_floor(&mut self, area: WorkArea) {
        self.area = area;
        self.apply_size();
        self.pos_x = self.pos_x.clamp(self.min_x(), self.max_x());
        self.pos_y = self.floor_y();
        self.set_state("idle", 2.0);
    }

    pub fn set_cursor(&mut self, x: f32, y: f32, now: Instant) {
        self.cursor = Some((x, y, now));
    }

    pub fn set_others(&mut self, list: Vec<OtherPet>) {
        let alarming = list.iter().find(|other| {
            other.state == "special"
                && self
                    .others
                    .iter()
                    .find(|was| was.id == other.id)
                    .is_none_or(|was| was.state != "special")
        });
        let alarm_x = alarming.map(|o| o.x);
        self.others = list;
        if let Some(ax) = alarm_x {
            if matches!(self.state.as_str(), "idle" | "walk")
                && (ax - self.middle_x()).abs() < 500.0 * self.area.dpi
            {
                self.facing = if ax >= self.middle_x() { 1 } else { -1 };
                self.view.set_facing(self.facing);
                self.set_state("talk", 1.3);
                self.say("click", 1.2);
            }
        }
    }

    pub fn on_meet(&mut self, req: MeetRequest) {
        if self.state != "idle" && self.state != "walk" {
            return;
        }
        self.joining_dance = None;
        self.dance_elapsed = 0.0;
        self.meeting = Some(MeetingState {
            with: req.with,
            kind: req.kind,
            lead: req.lead,
            time: 0.0,
            spoken: 0,
        });
        self.set_state("meet", req.seconds);
        self.view.sync_phase(0.0);
    }

    pub fn active_dance_info(&self) -> (Option<String>, f32) {
        if self.state == "meet" {
            let kind = self
                .meeting
                .as_ref()
                .and_then(|m| is_dance_state(&m.kind).then(|| m.kind.clone()));
            return (kind, self.dance_elapsed);
        }
        if is_dance_state(&self.state) {
            return (Some(self.state.clone()), self.dance_elapsed);
        }
        (None, 0.0)
    }

    pub fn start_dance(&mut self, kind: &str, seconds: f32) {
        if self.settings.sleep || self.state == "drag" || self.state == "fall" {
            return;
        }
        self.meeting = None;
        self.comforting = None;
        self.joining_dance = None;
        self.dance_elapsed = 0.0;
        self.set_state(kind, seconds);
        self.view.sync_phase(0.0);
        self.say("click", 1.4);
    }

    pub fn on_pointer_down(&mut self, screen_x: f32, screen_y: f32) {
        self.press = Some(DragPress {
            sx: screen_x,
            sy: screen_y,
            px: self.pos_x,
            py: self.pos_y,
            dragging: false,
        });
    }

    pub fn on_pointer_move(&mut self, screen_x: f32, screen_y: f32) {
        let Some(press) = self.press.as_mut() else {
            return;
        };
        let dx = screen_x - press.sx;
        let dy = screen_y - press.sy;
        if !press.dragging {
            if dx.hypot(dy) < DRAG_THRESHOLD * self.area.dpi {
                return;
            }
            press.dragging = true;
            self.bubble = None;
            let px = press.px;
            let py = press.py;
            self.pos_x = px + dx;
            self.pos_y = py + dy;
            self.set_state("drag", f32::INFINITY);
        } else {
            self.pos_x = press.px + dx;
            self.pos_y = press.py + dy;
        }
    }

    pub fn on_pointer_up(&mut self, is_click: bool, drop_area: WorkArea) {
        let Some(press) = self.press.take() else {
            return;
        };
        if press.dragging {
            self.area = drop_area;
            self.apply_size();
            self.pos_x = self.pos_x.clamp(self.min_x(), self.max_x());
            self.pos_y = self.pos_y.clamp(self.min_y(), self.floor_y());
            if self.settings.stay {
                self.set_state("idle", rand_range(1.0, 3.0));
            } else {
                self.start_fall();
            }
        } else if is_click {
            self.on_click();
        }
    }

    fn set_state(&mut self, mut next: &str, mut seconds: f32) {
        if next == "idle" && self.settings.sleep {
            next = "sleep";
            seconds = f32::INFINITY;
        }
        if next == "sleep" && self.state != "sleep" {
            self.sleep_time = 0.0;
        }
        self.state = next.to_string();
        self.state_time = seconds;
        let view_state = if next == "meet" {
            self.meeting
                .as_ref()
                .map(|m| m.kind.as_str())
                .unwrap_or("idle")
        } else {
            next
        };
        self.view.set_state(view_state);
    }

    fn say(&mut self, kind: &str, seconds: f32) {
        if self.settings.mute {
            return;
        }
        let Some(lines) = &self.def.config.lines else {
            return;
        };
        let list = match kind {
            "idle" => &lines.idle,
            "click" => &lines.click,
            "warn" => &lines.warn,
            "annoyed" => &lines.annoyed,
            "cry" => &lines.cry,
            "comfort" => &lines.comfort,
            "special" => &lines.special,
            "land" => &lines.land,
            _ => return,
        };
        if list.is_empty() {
            return;
        }
        let line = list[fastrand::usize(..list.len())].clone();
        let now = Instant::now();
        self.bubble = Some(BubbleState {
            text: line,
            shown_at: now,
            expires_at: now + Duration::from_secs_f32(seconds),
        });
    }

    fn start_walk(&mut self) {
        let mut tx = rand_range(self.min_x(), self.max_x());
        let candidates: Vec<&OtherPet> = self
            .others
            .iter()
            .filter(|o| self.beside(o) && o.state != "sleep")
            .collect();
        if !candidates.is_empty() && fastrand::f32() < 0.4 {
            let friend = candidates[fastrand::usize(..candidates.len())];
            let side = if fastrand::bool() { -1.0 } else { 1.0 };
            tx = (friend.x - self.win_w as f32 * 0.5 + side * self.elbow_room() * 1.3)
                .clamp(self.min_x(), self.max_x());
        }
        self.target_x = tx;
        if (self.target_x - self.pos_x).abs() < 40.0 * self.area.dpi {
            self.set_state("idle", rand_range(1.0, 3.0));
            return;
        }
        self.facing = if self.target_x > self.pos_x { 1 } else { -1 };
        self.view.set_facing(self.facing);
        self.set_state("walk", f32::INFINITY);
    }

    fn start_fall(&mut self) {
        self.fall_speed = 0.0;
        self.fall_from = self.pos_y;
        self.set_state("fall", f32::INFINITY);
    }

    fn land(&mut self) {
        let dropped = self.floor_y() - self.fall_from;
        self.pos_y = self.floor_y();
        self.set_state("idle", 1.0);
        let Some(crack) = self.def.config.crack.clone() else {
            return;
        };
        let dpi = self.area.dpi;
        if dropped < crack.from_height * dpi {
            return;
        }
        if !self.settings.sleep {
            self.say("land", 1.6);
        }
        let w = (crack.width * dpi).round();
        let h = (crack.height * dpi).round();
        let x = (self.middle_x() - w * 0.5)
            .clamp(self.area.x, self.area.x + self.area.width - w)
            .round() as i32;
        let y = (self.area.y + self.area.height - h).round() as i32;
        self.pending_effects.push(EffectRequest::Crack {
            x,
            y,
            width: w as u32,
            height: h as u32,
            seconds: crack.seconds,
            dpi,
        });
    }

    fn open_black_hole(&mut self) {
        let Some(bh) = &self.def.config.black_hole else {
            return;
        };
        let dpi = self.area.dpi;
        let size = (bh.size * dpi).round();
        let pet_x = self.middle_x();
        let pet_y = self.pos_y + self.win_h as f32 - self.pet_size_px * 0.5;
        self.facing = if pet_x < self.area.x + self.area.width * 0.5 {
            1
        } else {
            -1
        };
        self.view.set_facing(self.facing);

        let x = (pet_x + self.facing as f32 * (self.pet_size_px + size) * 0.5 - size * 0.5)
            .clamp(self.area.x, self.area.x + self.area.width - size)
            .round() as i32;
        let y = (pet_y - size * 0.8)
            .clamp(self.area.y, self.area.y + self.area.height - size)
            .round() as i32;

        self.pending_effects.push(EffectRequest::BlackHole {
            x,
            y,
            size: size as u32,
            seconds: bh.seconds,
        });
        let secs = bh.seconds;
        self.set_state("special", secs);
        self.say("special", 2.2);
    }

    fn on_click(&mut self) {
        if matches!(
            self.state.as_str(),
            "annoyed" | "cry" | "special" | "sleep"
        ) {
            return;
        }
        let now = Instant::now();
        let win = Duration::from_millis(self.def.config.click_window_ms);
        self.clicks.retain(|&t| now.saturating_duration_since(t) < win);
        self.clicks.push(now);

        if self.clicks.len() >= self.def.config.annoy_after_clicks {
            self.clicks.clear();
            self.tantrums += 1;
            self.set_state("annoyed", 1.8);
            self.say("annoyed", 1.8);
        } else {
            let has_warn = self
                .def
                .config
                .lines
                .as_ref()
                .is_some_and(|l| !l.warn.is_empty());
            let warn = self.clicks.len() == self.def.config.annoy_after_clicks.saturating_sub(1)
                && has_warn;
            self.set_state("talk", 1.2);
            self.say(if warn { "warn" } else { "click" }, 1.2);
        }
    }

    fn pointer_nearby(&self, range: f32, now: Instant) -> Option<(f32, f32)> {
        if !self.settings.chase {
            return None;
        }
        let (cx, cy, moved_at) = self.cursor?;
        if let Some(until) = self.ignore_pointer_until {
            if now < until {
                return None;
            }
        }
        if now.saturating_duration_since(moved_at).as_secs_f32() > BORED_AFTER {
            return None;
        }
        let dx = cx - self.middle_x();
        let dy = cy - (self.pos_y + self.win_h as f32 - self.pet_size_px * 0.5);
        if dx.hypot(dy) < range {
            Some((dx, dy))
        } else {
            None
        }
    }

    pub fn tick(&mut self, dt: f32, now: Instant) {
        let dt = dt.min(0.1);
        self.state_time -= dt;
        if self.state == "sleep" {
            self.sleep_time += dt;
        }
        let currently_dancing = is_dance_state(&self.state)
            || (self.state == "meet"
                && self.meeting.as_ref().is_some_and(|m| is_dance_state(&m.kind)));
        if currently_dancing {
            self.dance_elapsed += dt;
        } else {
            self.dance_elapsed = 0.0;
        }
        if let Some(b) = &self.bubble {
            if now >= b.expires_at {
                self.bubble = None;
            }
        }

        let dpi = self.area.dpi;
        let range = NOTICE * dpi + self.pet_size_px * 0.5;
        if (self.state == "idle" || self.state == "walk") && self.pointer_nearby(range, now).is_some()
        {
            let next = if self.settings.stay { "reach" } else { "chase" };
            self.set_state(next, TIRED_AFTER);
            let can_say = self
                .noticed_at
                .is_none_or(|t| now.saturating_duration_since(t).as_secs_f32() > 6.0);
            if can_say {
                self.noticed_at = Some(now);
                self.say("click", 1.2);
            }
        }

        let can_comfort = self.comfort_again_at.is_none_or(|t| now > t);
        if (self.state == "idle" || self.state == "walk") && can_comfort {
            let reach = if self.settings.stay {
                170.0 * dpi
            } else {
                700.0 * dpi
            };
            let sad_id = self
                .others
                .iter()
                .find(|o| {
                    o.state == "cry" && self.beside(o) && (o.x - self.middle_x()).abs() < reach
                })
                .map(|o| o.id.clone());
            if let Some(id) = sad_id {
                self.comforting = Some(ComfortState {
                    with: id,
                    said: false,
                });
                self.set_state("comfort", 9.0);
            }
        }

        // Nearby friend is dancing: walk over and join the dance in sync!
        let can_join = self.join_dance_cooldown.is_none_or(|t| now > t);
        if (self.state == "idle" || self.state == "walk") && can_join {
            let dancer = self.others.iter().find(|o| {
                o.dance_kind.is_some()
                    && self.beside(o)
                    && (o.x - self.middle_x()).abs() < 340.0 * dpi
            });
            if let Some(d) = dancer {
                let id = d.id.clone();
                let kind = d.dance_kind.clone().unwrap_or_else(|| "dance".to_string());
                self.joining_dance = Some((id, kind));
                self.set_state("join_dance", 5.5);
            }
        }

        let speed = self.speed_px();
        match self.state.as_str() {
            "chase" | "reach" => {
                if let Some((dx, dy)) = self.pointer_nearby(range * 1.3, now) {
                    if self.state_time <= 0.0 {
                        self.ignore_pointer_until = Some(now + Duration::from_secs_f32(REST));
                        self.set_state("idle", rand_range(1.0, 2.5));
                    } else {
                        self.facing = if dx >= 0.0 { 1 } else { -1 };
                        self.view.set_facing(self.facing);
                        let overhead = dy < -self.pet_size_px * 0.3;
                        let arm = if overhead {
                            self.pet_size_px * 0.3
                        } else {
                            self.body_width() * 0.5 + 10.0 * dpi
                        };
                        if self.state == "reach" {
                            if !self.settings.stay && dx.abs() > arm * 1.8 {
                                self.set_state("chase", self.state_time);
                            }
                        } else if dx.abs() <= arm {
                            self.set_state("reach", self.state_time);
                        } else {
                            self.pos_x = (self.pos_x + self.facing as f32 * speed * 1.2 * dt)
                                .clamp(self.min_x(), self.max_x());
                        }
                    }
                } else {
                    self.set_state("idle", rand_range(1.0, 2.5));
                }
            }
            "meet" => {
                let partner = self
                    .meeting
                    .as_ref()
                    .and_then(|m| self.others.iter().find(|o| o.id == m.with))
                    .cloned();
                let mut end_meet = self.state_time <= 0.0 || partner.is_none();
                if let (Some(m), Some(p)) = (self.meeting.as_mut(), &partner) {
                    m.time += dt;
                    if m.time > 0.8 && p.state != "meet" {
                        end_meet = true;
                    }
                }
                if end_meet {
                    self.meeting = None;
                    self.set_state("idle", rand_range(1.0, 3.0));
                } else if let Some(p) = partner {
                    let dx = p.x - self.middle_x();
                    self.facing = if dx >= 0.0 { 1 } else { -1 };
                    self.view.set_facing(self.facing);
                    let space = if is_dance_state(&self
                        .meeting
                        .as_ref()
                        .map(|m| m.kind.as_str())
                        .unwrap_or(""))
                    {
                        self.elbow_room() * 1.05
                    } else {
                        self.elbow_room()
                    };
                    if dx.abs() < space && !self.settings.stay {
                        self.pos_x = (self.pos_x - self.facing as f32 * speed * 0.7 * dt)
                            .clamp(self.min_x(), self.max_x());
                    }
                    let mut should_say: Option<&'static str> = None;
                    if let Some(m) = self.meeting.as_mut() {
                        let offset = if m.lead { 0.0 } else { 1.0 };
                        let is_d = is_dance_state(&m.kind);
                        let turns: &[f32] = match m.kind.as_str() {
                            "greet" => &[0.3],
                            "chat" => &[0.3, 2.3],
                            "butt_dance" | "boogie" | "spin_dance" => &[0.4, 2.4],
                            _ => &[0.5],
                        };
                        if let Some(&turn_at) = turns.get(m.spoken) {
                            if m.time >= turn_at + offset {
                                m.spoken += 1;
                                should_say = Some(if is_d { "click" } else { "idle" });
                            }
                        }
                    }
                    if let Some(kind) = should_say {
                        self.say(kind, 1.2);
                    }
                }
            }
            "join_dance" => {
                let friend = self
                    .joining_dance
                    .as_ref()
                    .and_then(|(fid, _)| self.others.iter().find(|o| o.id == *fid))
                    .cloned();
                if friend.as_ref().is_none_or(|f| f.dance_kind.is_none()) || self.state_time <= 0.0
                {
                    self.joining_dance = None;
                    self.join_dance_cooldown = Some(now + Duration::from_secs(6));
                    self.set_state("idle", rand_range(1.0, 2.0));
                } else if let Some(f) = friend {
                    let mut side = if self.middle_x() >= f.x { 1.0 } else { -1.0 };
                    let taken = self.others.iter().any(|o| {
                        o.state == "join_dance"
                            && o.id < self.id
                            && (if o.x >= f.x { 1.0 } else { -1.0 }) == side
                    });
                    if taken {
                        side = -side;
                    }
                    let spot = (f.x + side * self.elbow_room() * 1.15 - self.win_w as f32 * 0.5)
                        .clamp(self.min_x(), self.max_x());
                    let walking = (spot - self.pos_x).abs() > 4.0 * dpi && !self.settings.stay;
                    if walking {
                        self.facing = if spot > self.pos_x { 1 } else { -1 };
                        self.pos_x +=
                            self.facing as f32 * (speed * 1.35 * dt).min((spot - self.pos_x).abs());
                        self.view.set_facing(self.facing);
                        self.view.set_state("walk");
                    } else {
                        self.facing = if f.x >= self.middle_x() { 1 } else { -1 };
                        self.view.set_facing(self.facing);
                        let kind = f
                            .dance_kind
                            .or_else(|| self.joining_dance.as_ref().map(|(_, k)| k.clone()))
                            .unwrap_or_else(|| "dance".to_string());
                        self.joining_dance = None;
                        self.dance_elapsed = f.dance_elapsed;
                        self.set_state(&kind, 3.8);
                        self.view.sync_phase(f.dance_elapsed);
                        self.say("click", 1.3);
                        self.join_dance_cooldown = Some(now + Duration::from_secs(10));
                    }
                }
            }
            "comfort" => {
                let sad = self
                    .comforting
                    .as_ref()
                    .and_then(|c| self.others.iter().find(|o| o.id == c.with))
                    .cloned();
                if sad.as_ref().is_none_or(|s| s.state != "cry") || self.state_time <= 0.0 {
                    self.comforting = None;
                    self.comfort_again_at = Some(now + Duration::from_secs(8));
                    self.set_state("idle", rand_range(1.0, 2.0));
                } else if let Some(s) = sad {
                    let mut side = if self.middle_x() >= s.x { 1.0 } else { -1.0 };
                    let taken = self.others.iter().any(|o| {
                        o.state == "comfort"
                            && o.id < self.id
                            && (if o.x >= s.x { 1.0 } else { -1.0 }) == side
                    });
                    if taken {
                        side = -side;
                    }
                    let spot = (s.x + side * self.body_width() * 0.95 - self.win_w as f32 * 0.5)
                        .clamp(self.min_x(), self.max_x());
                    let walking = (spot - self.pos_x).abs() > 2.0 * dpi && !self.settings.stay;
                    if walking {
                        self.facing = if spot > self.pos_x { 1 } else { -1 };
                        self.pos_x +=
                            self.facing as f32 * (speed * 1.2 * dt).min((spot - self.pos_x).abs());
                    } else {
                        self.facing = if s.x >= self.middle_x() { 1 } else { -1 };
                    }
                    self.view.set_facing(self.facing);
                    self.view.set_state(if walking { "walk" } else { "comfort" });
                    if !walking {
                        let need_say = self.comforting.as_ref().is_some_and(|c| !c.said);
                        if need_say {
                            if let Some(c) = self.comforting.as_mut() {
                                c.said = true;
                            }
                            self.say("comfort", 2.0);
                        }
                    }
                }
            }
            "idle" => {
                if self.state_time <= 0.0 {
                    let r = fastrand::f32();
                    if r < 0.11 {
                        self.set_state("butt_dance", 4.6);
                        self.say("click", 1.4);
                    } else if r < 0.18 {
                        self.set_state("boogie", 4.0);
                        self.say("idle", 1.4);
                    } else if r < 0.23 {
                        self.set_state("spin", 3.2);
                        self.say("click", 1.2);
                    } else if r < 0.27 {
                        self.set_state("excited", 2.8);
                    } else if r < 0.48 {
                        self.set_state("talk", 1.8);
                        self.say("idle", 1.6);
                    } else if self.settings.stay {
                        self.set_state("idle", rand_range(2.2, 5.2));
                    } else {
                        self.start_walk();
                    }
                }
            }
            "dance" | "butt_dance" | "boogie" | "spin" | "spin_dance" | "excited" => {
                if self.state == "boogie" && !self.settings.stay {
                    self.pos_x = (self.pos_x + (self.dance_elapsed * 3.1).cos() * 18.0 * dpi * dt)
                        .clamp(self.min_x(), self.max_x());
                }
                if self.state_time <= 0.0 {
                    self.join_dance_cooldown = Some(now + Duration::from_secs(5));
                    self.set_state("idle", rand_range(1.8, 3.8));
                }
            }
            "walk" => {
                self.pos_x += speed * dt * self.facing as f32;
                if (self.target_x - self.pos_x) * self.facing as f32 <= 0.0 {
                    self.pos_x = self.target_x;
                    self.set_state("idle", rand_range(1.5, 4.0));
                }
            }
            "talk" | "special" => {
                if self.state_time <= 0.0 {
                    self.set_state("idle", rand_range(1.0, 3.0));
                }
            }
            "annoyed" => {
                if self.state_time <= 0.0 {
                    let bh_ready = self
                        .def
                        .config
                        .black_hole
                        .as_ref()
                        .is_some_and(|b| self.tantrums >= b.after_tantrums);
                    let can_cry = self
                        .def
                        .config
                        .lines
                        .as_ref()
                        .is_some_and(|l| !l.cry.is_empty());
                    if bh_ready {
                        self.tantrums = 0;
                        self.open_black_hole();
                    } else if can_cry {
                        let secs = self.def.config.cry_ms as f32 / 1000.0;
                        self.set_state("cry", secs);
                        self.sob_timer = 0.0;
                    } else {
                        self.set_state("idle", rand_range(1.0, 3.0));
                    }
                }
            }
            "cry" => {
                self.sob_timer -= dt;
                if self.sob_timer <= 0.0 && self.state_time > 1.0 {
                    self.sob_timer = 1.7;
                    self.say("cry", 1.5);
                }
                if self.state_time <= 0.0 {
                    self.set_state("idle", rand_range(2.0, 4.0));
                }
            }
            "fall" => {
                self.fall_speed += GRAVITY * dpi * dt;
                self.pos_y += self.fall_speed * dt;
                if self.pos_y >= self.floor_y() {
                    self.land();
                }
            }
            _ => {}
        }

        self.view.update(dt);
    }

    pub fn render(&mut self, text_renderer: &mut TextRenderer, now: Instant) {
        self.bgra_buf.fill(0);
        let win_w = self.win_w as usize;
        let win_h = self.win_h as usize;
        let box_px = match &self.view {
            PetView::Model(_) => (self.pet_size_px * (FRAME / BASE_FRAME))
                .round()
                .min(self.win_w as f32)
                .min(self.win_h as f32)
                .max(1.0) as u32,
            PetView::Sprite(_) => self.pet_size_px.round().max(1.0) as u32,
        };
        let pet_x = ((self.win_w as i32 - box_px as i32) / 2).max(0);
        let pet_y = (self.win_h as i32 - box_px as i32).max(0);

        self.view
            .render(&mut self.bgra_buf, win_w, win_h, pet_x, pet_y, box_px);

        if self.state == "sleep" {
            draw_zzz(
                text_renderer,
                &mut self.bgra_buf,
                win_w,
                win_h,
                self.pet_size_px,
                self.area.dpi,
                self.sleep_time,
            );
        }

        if let Some(b) = &self.bubble {
            let age = now.saturating_duration_since(b.shown_at).as_secs_f32();
            draw_bubble(
                text_renderer,
                &mut self.bgra_buf,
                win_w,
                win_h,
                self.pet_size_px,
                self.area.dpi,
                &b.text,
                age,
            );
        }
    }
}

pub fn render_character_avatar(def: &CharacterDef, px: u32) -> Option<tiny_skia::Pixmap> {
    let sz = px.max(1);
    let mut view = if let Some(model_cfg) = &def.config.model {
        PetView::Model(ModelView::load(&def.dir, model_cfg).ok()?)
    } else if let Some(sprite_cfg) = &def.config.sprites {
        PetView::Sprite(SpriteView::load(&def.dir, sprite_cfg).ok()?)
    } else {
        return None;
    };
    view.set_facing(1);
    view.update(0.0);
    let box_px = match &view {
        PetView::Model(_) => (sz as f32 * (FRAME / BASE_FRAME)).round().max(1.0) as u32,
        PetView::Sprite(_) => sz,
    };
    let dx = (sz as i32 - box_px as i32) / 2;
    let dy = sz as i32 - box_px as i32;
    let mut bgra = vec![0u8; (sz * sz * 4) as usize];
    view.render(&mut bgra, sz as usize, sz as usize, dx, dy, box_px);

    let mut pix = tiny_skia::Pixmap::new(sz, sz)?;
    for (src, dst) in bgra.chunks_exact(4).zip(pix.data_mut().chunks_exact_mut(4)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = src[3];
    }
    Some(pix)
}
