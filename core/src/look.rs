//! Each agent's face («cara»): Buddy's own character in another colour, with an accessory and other eyes.
//!
//! A look is a hood colour from a small palette (each colour gets the same shade ramp as Buddy's mint), an
//! accessory drawn over the head, an eye style and, optionally, the accessory's colour («insignia»). It comes from
//! `cara: color=…, accesorio=…, ojos=…` in the agent's agent.md, overridden by the setting `agent.<id>.cara` that
//! Settings writes (the same `k=v` syntax, or JSON). The sprite is made here from buddy-base, so both apps paint
//! the very same pixels and the avatar square (`face`) still frames the head. Buddy's default is buddy-base itself.

use std::collections::BTreeMap;
use std::path::Path;

use crate::pixel::{Character, Sprite};
use crate::CoreError;

/// The hood colours: id, label, the main tone (`b` in Buddy's palette). Menta is Buddy's.
pub const COLORS: [(&str, &str, &str); 8] = [
    ("menta", "Menta", "#86DDB8"),
    ("cielo", "Cielo", "#8CC8F2"),
    ("lavanda", "Lavanda", "#B9A5F0"),
    ("rosa", "Rosa", "#F4A6CB"),
    ("fresa", "Fresa", "#F28A8C"),
    ("mandarina", "Mandarina", "#F6B26E"),
    ("limon", "Limón", "#EBD86A"),
    ("grafito", "Grafito", "#A8B0BC"),
];

/// Buddy's mint ramp, from outline to light: the palette keys it fills and their colours.
const MINT: [(char, &str); 5] = [('o', "#1E4D3C"), ('d', "#3E9474"), ('B', "#5FBF98"), ('b', "#86DDB8"), ('l', "#B9F0D6")];
/// The accessory's own ramp uses these keys (same order as `MINT`).
const ACCESSORY_KEYS: [char; 5] = ['O', 'Q', 'q', 'a', 'A'];

pub const ACCESSORIES: [(&str, &str); 7] = [
    ("ninguno", "Ninguno"),
    ("gorra", "Gorra"),
    ("lentes", "Lentes"),
    ("audifonos", "Audífonos"),
    ("corona", "Corona"),
    ("bandana", "Bandana"),
    ("gorro", "Gorro de lana"),
];

pub const EYES: [(&str, &str); 4] = [("normales", "Normales"), ("felices", "Felices"), ("serios", "Serios"), ("guino", "Guiño")];

/// An accessory as pixels: offset from the hood's top-left corner (column 9, row 11 in idle frame 0) and rows of
/// palette keys (`.` leaves the pixel). `O Q q a A` are the accessory's ramp; `w c r` are Buddy's white, blush, red.
struct Stamp {
    id: &'static str,
    dx: i32,
    dy: i32,
    rows: &'static [&'static str],
    /// Hats cover the sprout, so it is taken away.
    hides_sprout: bool,
    /// Its colour when the look has no `insignia`: the first that differs from the hood.
    colors: &'static [&'static str],
}

const STAMPS: [Stamp; 6] = [
    Stamp {
        id: "gorra",
        dx: 1,
        dy: -5,
        hides_sprout: true,
        colors: &["fresa", "cielo"],
        rows: &[
            ".............qq................",
            ".........OOOOOOOOOO............",
            "......OOOAaaaaqaaaaOOO.........",
            "....OOAaaaaaaaqaaaaaaqOO.......",
            "...OAAAawwaaaaqaaaaaaqqqO......",
            "..OAAAAwccwaaaqaaaaaaqqqqO.....",
            ".OAAAAAawwaaaaqaaaaaaqqqqqO....",
            ".OaaaaaaaaaaaaqaaaaaaqqqqqO....",
            ".OaOOOOOOOOOOOOOOOOOOOOOOOOOOO.",
            "OOOAAAAAAAAAAAAAAAAAAAAAAAaqqqO",
            "..OaaaaaaaaaaaaaaaaaaaaaaaaqqqO",
            "..OqqqqqqqqqqqqqqqqqqqqqqqqqqqO",
            "...OOOOOOOOOOOOOOOOOOOOOOOOOOO.",
        ],
    },
    Stamp {
        id: "lentes",
        dx: 6,
        dy: 10,
        hides_sprout: false,
        colors: &["grafito"],
        rows: &[
            "..OOOOOO..OOOOOO..",
            "OOO...wOOOO...wOOO",
            "..O....O..O....O..",
            "..O....O..O....O..",
            "..O....O..O....O..",
            "..OOOOOO..OOOOOO..",
        ],
    },
    Stamp {
        id: "audifonos",
        dx: -2,
        dy: -2,
        hides_sprout: true,
        colors: &["grafito", "cielo"],
        rows: &[
            "............OOOOOOOOOO............",
            ".........OOOqOOOOOOOOqOOO.........",
            "........OqOOO........OOOqO........",
            "......OOOO..............OOOO......",
            ".....OqO..................OqO.....",
            "....OqO....................OqO....",
            "...OqO......................OqO...",
            "...OO........................OO...",
            "..OqO........................OqO..",
            ".OOO..........................OOO.",
            "OAaqO........................OAaqO",
            "OAaqO........................OAaqO",
            "OAaqO........................OAaqO",
            "OAaqO........................OAaqO",
            "OaaqO........................OaaqO",
            "OaaqO........................OaaqO",
            "OaaqO........................OaaqO",
            "OqqqO........................OqqqO",
            "OqqqO........................OqqqO",
            ".OOO..........................OOO.",
        ],
    },
    Stamp {
        id: "corona",
        dx: 5,
        dy: -8,
        hides_sprout: true,
        colors: &["limon", "mandarina"],
        rows: &[
            ".........ww.........",
            ".w.......OO.......w.",
            ".O......OOOO......O.",
            "OOO.....OAaO.....OOO",
            "OAO.OOO.OAaO.OOO.OAO",
            "OAO.OAO.OAaO.OAO.OAO",
            "OAaOaAaOaAaaOaAaOaAO",
            "OAAAAAAAArrAAAAAAAAO",
            "OaaawaaaarraaaawaaaO",
            "OOOOOOOOOOOOOOOOOOOO",
        ],
    },
    Stamp {
        id: "bandana",
        dx: 0,
        dy: 2,
        hides_sprout: false,
        colors: &["fresa", "cielo"],
        rows: &[
            "..OOOOOOOOOOOOOOOOOOOOOOOOOOOOO.",
            ".OAwAAAAwAAAAwAAAAwAAAAwAAAOaaqO",
            ".OaaaaaaaaaaaaaaaaaaaaaaaaaOaaqO",
            "OqqqwqqqqwqqqqwqqqqwqqqqwqqOaaqO",
            "OOOOOOOOOOOOOOOOOOOOOOOOOOOOOOO.",
            "............................OaO.",
            "............................OaqO",
            ".............................OO.",
        ],
    },
    Stamp {
        id: "gorro",
        dx: 0,
        dy: -8,
        hides_sprout: true,
        colors: &["mandarina", "fresa"],
        rows: &[
            "............OOOOOO............",
            "...........OwwwwAAO...........",
            "...........OwwwwAAO...........",
            "........OOOOOAAAAOOOOO........",
            "......OOaqaaqOOOOaqaaqOO......",
            ".....OqaaqaaqaaqaaqaaqaaO.....",
            "....OAqaaqaaqaaqaaqaaqaaqO....",
            "...OAAqaaqaaqaaqaaqaaqaaqqO...",
            "..OqAAqaaqaaqaaqaaqaaqaaqqqO..",
            ".OAqAAqaaqaaqaaqaaqaaqaaqqqqO.",
            ".OOOOOOOOOOOOOOOOOOOOOOOOOOOO.",
            "OAAAAAAAAAAAAAAAAAAAAAAAAAAAAO",
            "OqaqaqaqaqaqaqaqaqaqaqaqaqaqaO",
            "OqaqaqaqaqaqaqaqaqaqaqaqaqaqaO",
            "OqqqqqqqqqqqqqqqqqqqqqqqqqqqqO",
            ".OOOOOOOOOOOOOOOOOOOOOOOOOOOO.",
        ],
    },
];

/// An agent's face. Every field is an id from `look_options`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AgentLook {
    pub color: String,
    pub accessory: String,
    pub eyes: String,
    /// The accessory's colour (a colour id); none picks one that stands out from the hood.
    pub badge: Option<String>,
}

/// One choice in the face editor; `hex` is the swatch of a colour.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LookOption {
    pub id: String,
    pub label: String,
    pub hex: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct LookOptions {
    pub colors: Vec<LookOption>,
    pub accessories: Vec<LookOption>,
    pub eyes: Vec<LookOption>,
}

pub fn options() -> LookOptions {
    let plain = |list: &[(&str, &str)]| {
        list.iter().map(|(id, label)| LookOption { id: (*id).into(), label: (*label).into(), hex: None }).collect()
    };
    LookOptions {
        colors: COLORS
            .iter()
            .map(|(id, label, hex)| LookOption { id: (*id).into(), label: (*label).into(), hex: Some((*hex).into()) })
            .collect(),
        accessories: plain(&ACCESSORIES),
        eyes: plain(&EYES),
    }
}

/// The settings key of an agent's face.
pub fn look_key(id: &str) -> String {
    format!("agent.{id}.cara")
}

impl AgentLook {
    /// Buddy as it has always been.
    pub fn buddy() -> Self {
        Self { color: "menta".into(), accessory: "ninguno".into(), eyes: "normales".into(), badge: None }
    }

    /// The `k=v` text saved in Settings and written in agent.md.
    pub fn to_text(&self) -> String {
        let mut text = format!("color={}, accesorio={}, ojos={}", self.color, self.accessory, self.eyes);
        if let Some(badge) = &self.badge {
            text.push_str(&format!(", insignia={badge}"));
        }
        text
    }

    /// Fails on an unknown id, naming it.
    pub fn validate(&self) -> Result<(), CoreError> {
        let known = |list: &[&str], v: &str| list.contains(&v);
        let colors: Vec<&str> = COLORS.iter().map(|c| c.0).collect();
        let bad = if !known(&colors, &self.color) {
            Some(&self.color)
        } else if !ACCESSORIES.iter().any(|a| a.0 == self.accessory) {
            Some(&self.accessory)
        } else if !EYES.iter().any(|e| e.0 == self.eyes) {
            Some(&self.eyes)
        } else {
            self.badge.as_ref().filter(|b| !known(&colors, b))
        };
        match bad {
            Some(v) => Err(CoreError::Character(format!("opción de cara desconocida: {v}"))),
            None => Ok(()),
        }
    }

    /// Lays `text` (`k=v` pairs or JSON) over this look; unknown keys and values are skipped.
    fn overlay(&mut self, text: &str) {
        for (key, value) in pairs(text) {
            let value = normalize(&value);
            match normalize(&key).as_str() {
                "color" | "colour" => {
                    if let Some(c) = COLORS.iter().find(|c| c.0 == value) {
                        self.color = c.0.into();
                    }
                }
                "accesorio" | "accessory" => {
                    let value = match value.as_str() {
                        "nada" | "none" | "" => "ninguno",
                        "gafas" | "glasses" => "lentes",
                        "auriculares" | "headphones" => "audifonos",
                        "gorro de lana" | "beanie" => "gorro",
                        "cap" => "gorra",
                        "crown" => "corona",
                        v => v,
                    };
                    if let Some(a) = ACCESSORIES.iter().find(|a| a.0 == value) {
                        self.accessory = a.0.into();
                    }
                }
                "ojos" | "eyes" => {
                    let value = if value == "guiño" { "guino".to_string() } else { value };
                    if let Some(e) = EYES.iter().find(|e| e.0 == value) {
                        self.eyes = e.0.into();
                    }
                }
                "insignia" | "badge" => {
                    self.badge = COLORS.iter().find(|c| c.0 == value).map(|c| c.0.to_string());
                }
                _ => {}
            }
        }
    }
}

/// Lowercase, without accents, trimmed.
fn normalize(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// `{"color": "cielo", …}` or `color=cielo, accesorio=gorra` (`:` also works, `;` too).
fn pairs(text: &str) -> Vec<(String, String)> {
    let text = text.trim();
    if text.starts_with('{') {
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(text) else { return vec![] };
        return map
            .into_iter()
            .map(|(k, v)| (k, v.as_str().map(String::from).unwrap_or_default()))
            .collect();
    }
    text.split([',', ';'])
        .filter_map(|part| {
            let (k, v) = part.split_once('=').or_else(|| part.split_once(':'))?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// The `cara:` line of an agent.md front matter.
pub fn front_matter_look(text: &str) -> Option<String> {
    let rest = text.trim_start().strip_prefix("---")?;
    let head = &rest[..rest.find("\n---")?];
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == "cara").then(|| v.trim().to_string()).filter(|v| !v.is_empty())
    })
}

/// The look an agent has before its agent.md: Buddy's own for Buddy, a colour picked from its id otherwise.
fn base_look(id: &str) -> AgentLook {
    if id == crate::orchestrator::ORCHESTRATOR {
        return AgentLook::buddy();
    }
    let sum: usize = id.bytes().map(usize::from).sum();
    let color = COLORS[1 + sum % (COLORS.len() - 1)].0;
    AgentLook { color: color.into(), ..AgentLook::buddy() }
}

/// The face agent.md gives (or the built-in agent.md, when the file has no `cara:`), without Settings.
pub fn file_look(data_dir: &Path, id: &str) -> AgentLook {
    let mut look = base_look(id);
    let file = std::fs::read_to_string(data_dir.join("agents").join(id).join("agent.md")).ok();
    let text = file.as_deref().and_then(front_matter_look).or_else(|| {
        crate::orchestrator::BUILT_INS.iter().find(|(b, _)| *b == id).and_then(|(_, t)| front_matter_look(t))
    });
    if let Some(text) = text {
        look.overlay(&text);
    }
    look
}

/// The agent's face: agent.md, then what Settings saved.
pub fn resolve(data_dir: &Path, store: &crate::store::Store, id: &str) -> AgentLook {
    let mut look = file_look(data_dir, id);
    if let Some(saved) = store.setting(&look_key(id)).ok().flatten().filter(|s| !s.trim().is_empty()) {
        look.overlay(&saved);
    }
    look
}

/// Saves a face chosen in Settings; the one agent.md already gives clears the setting.
pub fn save(data_dir: &Path, store: &crate::store::Store, id: &str, look: &AgentLook) -> Result<(), CoreError> {
    look.validate()?;
    let value = if *look == file_look(data_dir, id) { String::new() } else { look.to_text() };
    store.set_setting(&look_key(id), &value)
}

// ---- Colours

fn hex_rgb(hex: &str) -> (f64, f64, f64) {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    (((v >> 16) & 0xFF) as f64 / 255.0, ((v >> 8) & 0xFF) as f64 / 255.0, (v & 0xFF) as f64 / 255.0)
}

fn rgb_hsl((r, g, b): (f64, f64, f64)) -> (f64, f64, f64) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_hex(h: f64, s: f64, l: f64) -> String {
    let (h, s, l) = (h.rem_euclid(1.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |mut t: f64| {
        t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round() as u8
    };
    format!("#{:02X}{:02X}{:02X}", channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

/// A colour's five tones (outline, deep shade, shade, main, light), derived the way Buddy's mint ramp is built:
/// the same hue, saturation and lightness steps around the main tone. Menta is Buddy's exact ramp.
pub fn ramp(color: &str) -> [String; 5] {
    let mint = MINT.map(|(_, hex)| hex.to_string());
    let Some((id, _, hex)) = COLORS.iter().find(|c| c.0 == color) else { return mint };
    if *id == "menta" {
        return mint;
    }
    derive_ramp(hex)
}

fn derive_ramp(hex: &str) -> [String; 5] {
    let (h, s, l) = rgb_hsl(hex_rgb(hex));
    let (mh, ms, ml) = rgb_hsl(hex_rgb(MINT[3].1));
    MINT.map(|(_, tone)| {
        let (th, ts, tl) = rgb_hsl(hex_rgb(tone));
        hsl_hex(h + (th - mh), s * (ts / ms), l + (tl - ml))
    })
}

// ---- The sprite

/// The accessory colour: the look's `insignia`, or the accessory's first default that differs from the hood.
fn accessory_color<'a>(look: &'a AgentLook, stamp: &Stamp) -> &'a str {
    if let Some(badge) = &look.badge {
        return badge;
    }
    let pick = stamp.colors.iter().find(|c| **c != look.color).unwrap_or(&stamp.colors[0]);
    COLORS.iter().find(|c| c.0 == *pick).map(|c| c.0).unwrap_or("fresa")
}

/// Where the face window starts in a frame: the first skin pixel of its top row. Everything is placed from it,
/// so head bobs, jumps and shakes carry the accessory along.
fn anchor(grid: &[Vec<char>]) -> Option<(i32, i32)> {
    grid.iter().enumerate().find_map(|(y, row)| row.iter().position(|c| *c == 's').map(|x| (x as i32, y as i32)))
}

/// Where a stamp's pixels land in a frame whose face window starts at `anchor` (hood top-left = anchor − (9, 8)).
fn stamp_pixels(stamp: &Stamp, anchor: (i32, i32)) -> impl Iterator<Item = (i32, i32, char)> + '_ {
    let (hx, hy) = (anchor.0 - 9, anchor.1 - 8);
    stamp.rows.iter().enumerate().flat_map(move |(r, row)| {
        row.chars()
            .enumerate()
            .filter(|(_, c)| *c != '.')
            .map(move |(c, ch)| (hx + stamp.dx + c as i32, hy + stamp.dy + r as i32, ch))
    })
}

const OPEN_EYE: [[char; 2]; 4] = [['w', 'k'], ['k', 'k'], ['k', 'k'], ['k', 'k']];

/// Swaps Buddy's two open eyes for the style (other poses — blinking, looking aside — are left as drawn).
fn draw_eyes(g: &mut [Vec<char>], (ax, ay): (i32, i32), style: &str) {
    if style == "normales" {
        return;
    }
    let (ax, ay) = (ax as usize, ay as usize);
    let ey = ay + 3;
    let eyes = [ax + 1, ax + 9];
    let open = |g: &[Vec<char>], ex: usize| {
        (0..4).all(|r| (0..2).all(|c| g.get(ey + r).and_then(|row| row.get(ex + c)) == Some(&OPEN_EYE[r][c])))
    };
    if !eyes.iter().all(|ex| open(g, *ex)) || ex_out_of_range(ax, ey, g) {
        return;
    }
    for (i, &ex) in eyes.iter().enumerate() {
        if style == "guino" && i == 0 {
            continue;
        }
        for r in 0..4 {
            g[ey + r][ex] = 's';
            g[ey + r][ex + 1] = 's';
        }
        match style {
            "felices" | "guino" => {
                for (x, y) in [(ex - 1, ey + 3), (ex, ey + 2), (ex + 1, ey + 2), (ex + 2, ey + 3)] {
                    g[y][x] = 'k';
                }
            }
            "serios" => {
                for y in [ey + 2, ey + 3] {
                    g[y][ex] = 'k';
                    g[y][ex + 1] = 'k';
                }
                g[ey + 2][if i == 0 { ex } else { ex + 1 }] = 'w';
                let brow = if i == 0 { [ex - 1, ex, ex + 1] } else { [ex, ex + 1, ex + 2] };
                for x in brow {
                    g[ey][x] = 'k';
                }
            }
            _ => {}
        }
    }
}

fn ex_out_of_range(ax: usize, ey: usize, g: &[Vec<char>]) -> bool {
    ax < 1 || ey + 4 > g.len() || g.first().is_none_or(|row| ax + 12 > row.len())
}

/// Buddy's character dressed in `look` (every state and frame).
pub fn dress(base: &Character, look: &AgentLook) -> Character {
    let mut c = base.clone();
    for ((key, _), tone) in MINT.iter().zip(ramp(&look.color)) {
        c.palette.insert(key.to_string(), Some(tone));
    }
    let stamp = STAMPS.iter().find(|s| s.id == look.accessory);
    if let Some(stamp) = stamp {
        for (key, tone) in ACCESSORY_KEYS.iter().zip(ramp(accessory_color(look, stamp))) {
            c.palette.insert(key.to_string(), Some(tone));
        }
    }
    if stamp.is_none() && look.eyes == "normales" {
        return c;
    }
    let size = c.size as i32;
    for state in c.states.values_mut() {
        for frame in &mut state.frames {
            let mut g: Vec<Vec<char>> = frame.iter().map(|row| row.chars().collect()).collect();
            let Some(a) = anchor(&g) else { continue };
            draw_eyes(&mut g, a, &look.eyes);
            if let Some(stamp) = stamp {
                if stamp.hides_sprout {
                    let (hx, hy) = (a.0 - 9, a.1 - 8);
                    for y in (hy - 10).max(0)..=hy.min(size - 1) {
                        for x in (hx + 4).max(0)..=(hx + 26).min(size - 1) {
                            let px = &mut g[y as usize][x as usize];
                            if "gGL".contains(*px) {
                                *px = if y == hy { 'o' } else { '.' };
                            } else if *px == 'o' && y < hy {
                                *px = '.';
                            }
                        }
                    }
                }
                for (x, y, ch) in stamp_pixels(stamp, a) {
                    if (0..size).contains(&x) && (0..size).contains(&y) {
                        g[y as usize][x as usize] = ch;
                    }
                }
            }
            *frame = g.into_iter().map(|row| row.into_iter().collect()).collect();
        }
    }
    c
}

/// The agent's sprite, ready to paint (the same `Sprite` as buddy-base, with its avatar square).
pub fn sprite(look: &AgentLook, id: &str, name: &str) -> Result<Sprite, CoreError> {
    let base = crate::pixel::builtin("buddy-base")?;
    let mut sprite = dress(&base, look).rasterize();
    if !(id == crate::orchestrator::ORCHESTRATOR && *look == AgentLook::buddy()) {
        sprite.id = format!("agent-{id}");
    }
    if !name.is_empty() {
        sprite.name = name.to_string();
    }
    Ok(sprite)
}

/// Only idle frame 0 of a look (to preview a choice cheaply).
pub fn preview(look: &AgentLook) -> Result<Sprite, CoreError> {
    let mut base = crate::pixel::builtin("buddy-base")?;
    base.states = base
        .states
        .into_iter()
        .filter(|(name, _)| name == "idle")
        .map(|(name, mut state)| {
            state.frames.truncate(1);
            (name, state)
        })
        .collect::<BTreeMap<_, _>>();
    let mut sprite = dress(&base, look).rasterize();
    sprite.id = "look-preview".into();
    Ok(sprite)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(color: &str, accessory: &str, eyes: &str) -> AgentLook {
        AgentLook { color: color.into(), accessory: accessory.into(), eyes: eyes.into(), badge: None }
    }

    #[test]
    fn buddys_default_is_buddy_base_byte_for_byte() {
        let base = crate::pixel::builtin_sprite("buddy-base").unwrap();
        let dressed = sprite(&AgentLook::buddy(), "buddy", "").unwrap();
        assert_eq!(dressed, base);
        let dir = tempfile::tempdir().unwrap();
        crate::orchestrator::load(dir.path());
        assert_eq!(file_look(dir.path(), "buddy"), AgentLook::buddy());
    }

    #[test]
    fn every_look_keeps_the_size_and_the_avatar_square() {
        let base = crate::pixel::builtin("buddy-base").unwrap();
        for (i, (accessory, _)) in ACCESSORIES.iter().enumerate() {
            for (j, (eyes, _)) in EYES.iter().enumerate() {
                let color = COLORS[(i + j) % COLORS.len()].0;
                let s = sprite(&look(color, accessory, eyes), "x", "X").unwrap();
                assert_eq!((s.size, s.face), (48, base.face));
                for state in &s.states {
                    assert_eq!(state.frames.len(), base.states[&state.name].frames.len());
                    assert!(state.frames.iter().all(|f| f.len() == 48 * 48));
                }
            }
        }
    }

    #[test]
    fn recolouring_leaves_skin_eyes_sprout_and_symbols_alone() {
        let base = crate::pixel::builtin("buddy-base").unwrap();
        let a = base.rasterize();
        let b = sprite(&look("cielo", "ninguno", "normales"), "x", "").unwrap();
        let mut changed = 0;
        for (sa, sb) in a.states.iter().zip(&b.states) {
            let keys = &base.states[&sa.name].frames;
            for ((fa, fb), rows) in sa.frames.iter().zip(&sb.frames).zip(keys) {
                let keys: Vec<char> = rows.iter().flat_map(|r| r.chars()).collect();
                for ((pa, pb), key) in fa.iter().zip(fb).zip(keys) {
                    if "odBbl".contains(key) {
                        assert_ne!(pa, pb, "{key} is recoloured");
                        changed += 1;
                    } else {
                        assert_eq!(pa, pb, "{key} stays");
                    }
                }
            }
        }
        assert!(changed > 1000);
    }

    #[test]
    fn every_look_leaves_the_laptop_alone() {
        // The laptop (silver lid, its outline, the screen's glow on the face) keeps its own colours in every look:
        // hood colours, accessories and eye styles only touch Buddy.
        let base = crate::pixel::builtin("buddy-base").unwrap();
        let plain = base.rasterize();
        let mut checked = 0;
        for (i, (accessory, _)) in ACCESSORIES.iter().enumerate() {
            for (c, (color, ..)) in COLORS.iter().enumerate() {
                let eyes = EYES[(i + c) % EYES.len()].0;
                let dressed = sprite(&look(color, accessory, eyes), "x", "").unwrap();
                for state in plain.states.iter().filter(|s| s.name.starts_with("laptop")) {
                    let other = dressed.states.iter().find(|s| s.name == state.name).unwrap();
                    for ((rows, a), b) in base.states[&state.name].frames.iter().zip(&state.frames).zip(&other.frames) {
                        let keys: Vec<char> = rows.iter().flat_map(|r| r.chars()).collect();
                        for (p, key) in keys.iter().enumerate() {
                            // The lid and base (their outline is `k` around the silver keys) and the glow on the chin.
                            // (A glasses accessory redraws the lenses, screen-lit glints included: those are Buddy's.)
                            let lid = (crate::pixel::LAPTOP_KEYS.contains(*key) && p / 48 >= 30)
                                || (*key == 'k' && p / 48 >= 34);
                            if lid {
                                assert_eq!(a[p], b[p], "{} {color}/{accessory}: «{key}» at {},{}", state.name, p % 48, p / 48);
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(checked > 100_000, "{checked}");
    }

    #[test]
    fn accessories_stay_inside_the_canvas_and_the_avatar() {
        let base = crate::pixel::builtin("buddy-base").unwrap();
        let face = base.face.unwrap();
        for stamp in &STAMPS {
            for (name, state) in &base.states {
                for frame in &state.frames {
                    let g: Vec<Vec<char>> = frame.iter().map(|r| r.chars().collect()).collect();
                    let a = anchor(&g).unwrap();
                    for (x, y, _) in stamp_pixels(stamp, a) {
                        assert!((0..48).contains(&x) && (0..48).contains(&y), "{} in {name} at {x},{y}", stamp.id);
                    }
                }
            }
            let g: Vec<Vec<char>> = base.states["idle"].frames[0].iter().map(|r| r.chars().collect()).collect();
            for (x, y, _) in stamp_pixels(stamp, anchor(&g).unwrap()) {
                let inside = |v: i32, o: u32| v >= o as i32 && v < (o + face.size) as i32;
                assert!(inside(x, face.x) && inside(y, face.y), "{} leaves the avatar at {x},{y}", stamp.id);
            }
        }
    }

    #[test]
    fn hats_take_the_sprout_and_eyes_change_only_open_eyes() {
        let base = crate::pixel::builtin("buddy-base").unwrap();
        let leaves = |c: &Character| c.states["idle"].frames[0].iter().any(|r| r.contains('L'));
        assert!(leaves(&dress(&base, &look("menta", "bandana", "normales"))));
        assert!(!leaves(&dress(&base, &look("menta", "gorra", "normales"))));
        let happy = dress(&base, &look("menta", "ninguno", "felices"));
        assert_ne!(happy.states["idle"].frames[0], base.states["idle"].frames[0]);
        assert_eq!(happy.states["blink"].frames, base.states["blink"].frames, "closed eyes stay closed");
    }

    #[test]
    fn the_ramp_is_derived_like_buddys_mint() {
        let derived = derive_ramp(MINT[3].1);
        for ((_, mint), tone) in MINT.iter().zip(derived) {
            let (a, b) = (hex_rgb(mint), hex_rgb(&tone));
            assert!((a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02 && (a.2 - b.2).abs() < 0.02, "{mint} ≈ {tone}");
        }
        for (id, ..) in COLORS {
            let r = ramp(id);
            let light = |hex: &str| rgb_hsl(hex_rgb(hex)).2;
            assert!(r.windows(2).all(|w| light(&w[0]) < light(&w[1])), "{id} goes from dark to light");
        }
    }

    #[test]
    fn looks_come_from_agent_md_then_settings() {
        let dir = tempfile::tempdir().unwrap();
        crate::orchestrator::load(dir.path());
        let store = crate::store::Store::open_in_memory().unwrap();
        let parley = resolve(dir.path(), &store, "parley");
        assert_eq!(parley.accessory, "gorra");
        assert_ne!(parley.color, "menta");
        // Settings: k=v (accents and aliases too) or JSON; unknown values are skipped.
        store.set_setting(&look_key("parley"), "color=Limón, accesorio=gafas, ojos=guiño, insignia=nada").unwrap();
        assert_eq!(resolve(dir.path(), &store, "parley"), AgentLook { badge: None, ..look("limon", "lentes", "guino") });
        store.set_setting(&look_key("parley"), r#"{"color":"rosa","accesorio":"corona","ojos":"raros"}"#).unwrap();
        let l = resolve(dir.path(), &store, "parley");
        assert_eq!((l.color.as_str(), l.accessory.as_str(), l.eyes.as_str()), ("rosa", "corona", parley.eyes.as_str()));
        // Saving agent.md's own look clears the setting; unknown ids are refused.
        save(dir.path(), &store, "parley", &parley).unwrap();
        assert_eq!(store.setting(&look_key("parley")).unwrap().as_deref(), Some(""));
        assert!(save(dir.path(), &store, "parley", &look("verde", "ninguno", "normales")).is_err());
        // An agent without `cara:` still gets its own colour, never Buddy's mint.
        assert_ne!(base_look("notas").color, "menta");
        assert_eq!(front_matter_look("---\nid: x\ncara: color=cielo\n---\ncara: no"), Some("color=cielo".into()));
    }

    fn base_face() -> crate::pixel::FaceRect {
        crate::pixel::builtin("buddy-base").unwrap().face.unwrap()
    }

    /// `BUDDY_LOOK_PREVIEW=<folder> cargo test -p buddy-core look::tests::preview_sheets -- --ignored` writes PNG
    /// sheets of every accessory × some colours (8× faces) and every state of one look.
    #[test]
    #[ignore]
    fn preview_sheets() {
        let Ok(folder) = std::env::var("BUDDY_LOOK_PREVIEW") else { return };
        let folder = std::path::PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        let paint = |img: &mut image::RgbaImage, frame: &[u32], x0: u32, y0: u32, crop: (u32, u32, u32), scale: u32| {
            let (cx, cy, size) = crop;
            for y in 0..size {
                for x in 0..size {
                    let argb = frame[((cy + y) * 48 + cx + x) as usize];
                    if argb >> 24 == 0 {
                        continue;
                    }
                    let px = image::Rgba([(argb >> 16) as u8, (argb >> 8) as u8, argb as u8, 255]);
                    for dy in 0..scale {
                        for dx in 0..scale {
                            img.put_pixel(x0 + x * scale + dx, y0 + y * scale + dy, px);
                        }
                    }
                }
            }
        };
        let colors = ["menta", "cielo", "fresa", "limon", "grafito", "lavanda"];
        let (scale, cell) = (8u32, 34 * 8 + 12);
        let mut sheet = image::RgbaImage::from_pixel(cell * colors.len() as u32 + 12, (cell + 40) * 7 + 12, image::Rgba([236, 236, 240, 255]));
        for (r, (accessory, _)) in ACCESSORIES.iter().enumerate() {
            for (c, color) in colors.iter().enumerate() {
                let eyes = EYES[(r + c) % EYES.len()].0;
                let s = sprite(&look(color, accessory, eyes), "x", "").unwrap();
                let f = s.face.unwrap();
                let frame = &s.states[0].frames[0];
                let (x0, y0) = (12 + c as u32 * cell, 12 + r as u32 * (cell + 40));
                paint(&mut sheet, frame, x0, y0, (f.x, f.y, f.size), scale);
                paint(&mut sheet, frame, x0, y0 + 34 * scale + 4, (f.x, f.y, f.size), 1);
            }
        }
        sheet.save(folder.join("looks-accessories.png")).unwrap();
        let s = sprite(&look("cielo", "gorra", "normales"), "parley", "").unwrap();
        let cols = s.states.iter().map(|s| s.frames.len()).max().unwrap() as u32;
        let mut states = image::RgbaImage::from_pixel(cols * 48 * 3, s.states.len() as u32 * 48 * 3, image::Rgba([250, 250, 250, 255]));
        for (i, state) in s.states.iter().enumerate() {
            for (j, frame) in state.frames.iter().enumerate() {
                paint(&mut states, frame, j as u32 * 144, i as u32 * 144, (0, 0, 48), 3);
            }
        }
        states.save(folder.join("looks-states.png")).unwrap();
        // The Windows Settings preview (apps/windows/preview/settings.html) paints faces from this, without the core.
        let f = base_face();
        let mut grids = serde_json::Map::new();
        for (accessory, _) in ACCESSORIES {
            for (eyes, _) in EYES {
                let c = dress(&crate::pixel::builtin("buddy-base").unwrap(), &look("menta", accessory, eyes));
                let rows: Vec<String> = c.states["idle"].frames[0][f.y as usize..(f.y + f.size) as usize]
                    .iter()
                    .map(|r| r.chars().skip(f.x as usize).take(f.size as usize).collect())
                    .collect();
                grids.insert(format!("{accessory}/{eyes}"), rows.into());
            }
        }
        let base = crate::pixel::builtin("buddy-base").unwrap();
        let mock = serde_json::json!({
            "palette": base.palette,
            "ramps": COLORS.iter().map(|c| (c.0.to_string(), ramp(c.0).to_vec().into())).collect::<serde_json::Map<_, _>>(),
            "accessoryColors": STAMPS.iter().map(|s| (s.id.to_string(), s.colors.to_vec().into())).collect::<serde_json::Map<_, _>>(),
            "grids": grids,
        });
        std::fs::write(folder.join("agent-looks.json"), serde_json::to_string(&mock).unwrap() + "\n").unwrap();
    }
}
