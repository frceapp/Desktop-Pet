use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

pub const WIN_W: f32 = 260.0;
pub const WIN_H: f32 = 300.0;

pub const SIZES: &[(&str, f32)] = &[
    ("Tiny", 0.6),
    ("Small", 0.8),
    ("Normal", 1.0),
    ("Large", 1.5),
    ("Huge", 2.0),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetSettings {
    #[serde(default = "default_scale")]
    pub scale: f32,
    #[serde(default = "default_scale")]
    pub speed_scale: f32,
    #[serde(default)]
    pub stay: bool,
    #[serde(default)]
    pub sleep: bool,
    #[serde(default)]
    pub mute: bool,
    #[serde(default = "default_true")]
    pub chase: bool,
}

impl Default for PetSettings {
    fn default() -> Self {
        Self {
            scale: 1.0,
            speed_scale: 1.0,
            stay: false,
            sleep: false,
            mute: false,
            chase: true,
        }
    }
}

fn default_scale() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub characters: Vec<String>,
    #[serde(default)]
    pub pets: HashMap<String, PetSettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            characters: vec!["chiikawa".to_string()],
            pets: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct LegacySettings {
    character: Option<String>,
    scale: Option<f32>,
    stay: Option<bool>,
    sleep: Option<bool>,
}

impl Settings {
    pub fn load() -> Self {
        let path = settings_file_path();
        let Ok(raw) = fs::read_to_string(&path) else {
            return Self::default();
        };
        if let Ok(parsed) = serde_json::from_str::<Settings>(&raw) {
            if !parsed.characters.is_empty() {
                return parsed;
            }
        }
        if let Ok(legacy) = serde_json::from_str::<LegacySettings>(&raw) {
            if let Some(character) = legacy.character {
                let mut pets = HashMap::new();
                pets.insert(
                    character.clone(),
                    PetSettings {
                        scale: legacy.scale.unwrap_or(1.0),
                        stay: legacy.stay.unwrap_or(false),
                        sleep: legacy.sleep.unwrap_or(false),
                        ..PetSettings::default()
                    },
                );
                return Self {
                    characters: vec![character],
                    pets,
                };
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = settings_file_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    pub fn pet_settings(&self, id: &str) -> PetSettings {
        self.pets.get(id).cloned().unwrap_or_default()
    }
}

pub fn settings_file_path() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("desktop-pet").join("settings.json");
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("settings.json");
        }
    }
    PathBuf::from("settings.json")
}

pub fn find_characters_dir() -> PathBuf {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("characters"));
            if let Some(p1) = dir.parent() {
                candidates.push(p1.join("characters"));
                if let Some(p2) = p1.parent() {
                    candidates.push(p2.join("characters"));
                }
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("characters"));
    }
    for c in &candidates {
        if c.is_dir() {
            return c.clone();
        }
    }
    candidates.into_iter().last().unwrap_or_else(|| PathBuf::from("characters"))
}

#[derive(Debug, Clone)]
pub enum ExpressionOp {
    Include(String),
    Clear([f32; 4]),
    Copy {
        sx: f32,
        sy: f32,
        w: f32,
        h: f32,
        dx: f32,
        dy: f32,
        dw: f32,
        dh: f32,
    },
    Stroke {
        width: f32,
        points: Vec<[f32; 2]>,
    },
}

impl<'de> Deserialize<'de> for ExpressionOp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OpVisitor;

        impl<'de> Visitor<'de> for OpVisitor {
            type Value = ExpressionOp;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("expression string name or {clear|copy|stroke} object")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(ExpressionOp::Include(v.to_string()))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: de::MapAccess<'de>,
            {
                let mut result: Option<ExpressionOp> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "clear" => {
                            let arr: [f32; 4] = map.next_value()?;
                            result = Some(ExpressionOp::Clear(arr));
                        }
                        "copy" => {
                            let arr: Vec<f32> = map.next_value()?;
                            if arr.len() < 6 {
                                return Err(de::Error::invalid_length(arr.len(), &"6 or 8 numbers"));
                            }
                            let w = arr[2];
                            let h = arr[3];
                            result = Some(ExpressionOp::Copy {
                                sx: arr[0],
                                sy: arr[1],
                                w,
                                h,
                                dx: arr[4],
                                dy: arr[5],
                                dw: arr.get(6).copied().unwrap_or(w),
                                dh: arr.get(7).copied().unwrap_or(h),
                            });
                        }
                        "stroke" => {
                            struct StrokeSeed;
                            impl<'de> de::DeserializeSeed<'de> for StrokeSeed {
                                type Value = ExpressionOp;
                                fn deserialize<D2>(self, d: D2) -> Result<Self::Value, D2::Error>
                                where
                                    D2: Deserializer<'de>,
                                {
                                    struct StrokeVisitor;
                                    impl<'de> Visitor<'de> for StrokeVisitor {
                                        type Value = ExpressionOp;
                                        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                                            f.write_str("[width, [x, y], ...]")
                                        }
                                        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                                        where
                                            A: SeqAccess<'de>,
                                        {
                                            let width: f32 = seq
                                                .next_element()?
                                                .ok_or_else(|| de::Error::missing_field("width"))?;
                                            let mut points = Vec::new();
                                            while let Some(pt) = seq.next_element::<[f32; 2]>()? {
                                                points.push(pt);
                                            }
                                            Ok(ExpressionOp::Stroke { width, points })
                                        }
                                    }
                                    d.deserialize_seq(StrokeVisitor)
                                }
                            }
                            result = Some(map.next_value_seed(StrokeSeed)?);
                        }
                        _ => {
                            let _: serde_json::Value = map.next_value()?;
                        }
                    }
                }
                result.ok_or_else(|| de::Error::missing_field("clear, copy, or stroke"))
            }
        }

        deserializer.deserialize_any(OpVisitor)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TearsConfig {
    pub x: f32,
    pub y: f32,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub file: String,
    pub node: Option<String>,
    pub yaw: Option<f32>,
    pub ink: Option<String>,
    pub tears: Option<TearsConfig>,
    #[serde(default)]
    pub expressions: HashMap<String, Vec<ExpressionOp>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpritesConfig {
    pub idle: String,
    pub walk: Option<String>,
    pub talk: Option<String>,
    pub annoyed: Option<String>,
    pub cry: Option<String>,
    pub drag: Option<String>,
    pub sit: Option<String>,
    pub sleep: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LinesConfig {
    #[serde(default)]
    pub idle: Vec<String>,
    #[serde(default)]
    pub click: Vec<String>,
    #[serde(default)]
    pub warn: Vec<String>,
    #[serde(default)]
    pub annoyed: Vec<String>,
    #[serde(default)]
    pub cry: Vec<String>,
    #[serde(default)]
    pub comfort: Vec<String>,
    #[serde(default)]
    pub special: Vec<String>,
    #[serde(default)]
    pub land: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlackHoleConfig {
    #[serde(rename = "afterTantrums", default = "default_tantrums")]
    pub after_tantrums: usize,
    #[serde(default = "default_bh_size")]
    pub size: f32,
    #[serde(default = "default_bh_seconds")]
    pub seconds: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CrackConfig {
    #[serde(rename = "fromHeight", default = "default_crack_height")]
    pub from_height: f32,
    #[serde(default = "default_crack_w")]
    pub width: f32,
    #[serde(default = "default_crack_h")]
    pub height: f32,
    #[serde(default = "default_crack_sec")]
    pub seconds: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CharacterConfig {
    pub name: Option<String>,
    #[serde(default = "default_char_size")]
    pub size: f32,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(rename = "annoyAfterClicks", default = "default_annoy")]
    pub annoy_after_clicks: usize,
    #[serde(rename = "clickWindowMs", default = "default_click_window")]
    pub click_window_ms: u64,
    #[serde(rename = "cryMs", default = "default_cry_ms")]
    pub cry_ms: u64,
    pub sprites: Option<SpritesConfig>,
    pub model: Option<ModelConfig>,
    pub lines: Option<LinesConfig>,
    #[serde(rename = "blackHole")]
    pub black_hole: Option<BlackHoleConfig>,
    pub crack: Option<CrackConfig>,
}

fn default_char_size() -> f32 { 150.0 }
fn default_speed() -> f32 { 60.0 }
fn default_annoy() -> usize { 5 }
fn default_click_window() -> u64 { 4000 }
fn default_cry_ms() -> u64 { 6000 }
fn default_tantrums() -> usize { 3 }
fn default_bh_size() -> f32 { 260.0 }
fn default_bh_seconds() -> f32 { 6.0 }
fn default_crack_height() -> f32 { 200.0 }
fn default_crack_w() -> f32 { 420.0 }
fn default_crack_h() -> f32 { 260.0 }
fn default_crack_sec() -> f32 { 5.0 }

#[derive(Debug, Clone)]
pub struct CharacterDef {
    pub id: String,
    pub dir: PathBuf,
    pub config: CharacterConfig,
}

impl CharacterDef {
    pub fn display_name(&self) -> &str {
        self.config.name.as_deref().unwrap_or(&self.id)
    }
}

pub fn load_characters(char_dir: &Path) -> Vec<CharacterDef> {
    let mut list = Vec::new();
    let Ok(entries) = fs::read_dir(char_dir) else {
        return list;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        let cfg_path = dir.join("character.json");
        let Ok(raw) = fs::read_to_string(&cfg_path) else {
            continue;
        };
        let Ok(config) = serde_json::from_str::<CharacterConfig>(&raw) else {
            continue;
        };
        let has_sprites = config.sprites.as_ref().is_some_and(|s| !s.idle.is_empty());
        let has_model = config.model.as_ref().is_some_and(|m| !m.file.is_empty());
        if !has_sprites && !has_model {
            continue;
        }
        list.push(CharacterDef { id, dir, config });
    }
    list.sort_by(|a, b| a.id.cmp(&b.id));
    list
}

pub fn chosen_characters(char_dir: &Path, settings: &Settings) -> Vec<CharacterDef> {
    let all = load_characters(char_dir);
    let chosen: Vec<_> = all
        .iter()
        .filter(|c| settings.characters.contains(&c.id))
        .cloned()
        .collect();
    if chosen.is_empty() {
        all.into_iter().take(1).collect()
    } else {
        chosen
    }
}

pub fn parse_hex_color(s: &str) -> [u8; 4] {
    let hex = s.trim().trim_start_matches('#');
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).unwrap_or(0) * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).unwrap_or(0) * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).unwrap_or(0) * 17;
            [r, g, b, 255]
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
            [r, g, b, 255]
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
            let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or(255);
            [r, g, b, a]
        }
        _ => [57, 26, 28, 255],
    }
}
