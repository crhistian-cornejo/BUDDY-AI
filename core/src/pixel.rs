//! Pixel characters drawn in code (docs/design/MASCOTA-PIXEL.md).
//!
//! A character is a JSON grid: each frame is `size` strings of `size` characters, each character a palette key
//! (`.` is transparent). The core validates it and rasterizes it to ARGB so both apps paint the very same pixels.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::CoreError;

/// The characters that ship with Buddy, by id.
const BUILTIN: &[(&str, &str)] = &[("buddy-base", include_str!("../characters/buddy-base.json"))];

/// At most this many visible colors per character.
pub const MAX_COLORS: usize = 16;
pub const MAX_FPS: u32 = 30;

#[derive(Debug, Clone, Deserialize)]
pub struct Character {
    pub id: String,
    pub name: String,
    pub size: usize,
    /// The square the apps crop as the character's avatar (in idle frame 0).
    #[serde(default)]
    pub face: Option<FaceRect>,
    /// Key → `#RRGGBB`, or null for transparent.
    pub palette: BTreeMap<String, Option<String>>,
    pub states: BTreeMap<String, CharacterState>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CharacterState {
    pub fps: u32,
    pub frames: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FaceRect {
    pub x: u32,
    pub y: u32,
    pub size: u32,
}

/// A character ready to paint: every frame is `size × size` pixels, row by row, as `0xAARRGGBB` (0 = transparent).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Sprite {
    pub id: String,
    pub name: String,
    pub size: u32,
    /// The avatar square, when the character has one.
    pub face: Option<FaceRect>,
    pub states: Vec<SpriteState>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SpriteState {
    pub name: String,
    pub fps: u32,
    pub frames: Vec<Vec<u32>>,
}

impl Character {
    pub fn parse(json: &str) -> Result<Self, CoreError> {
        let character: Character =
            serde_json::from_str(json).map_err(|e| CoreError::Character(format!("JSON inválido: {e}")))?;
        character.validate()?;
        Ok(character)
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        let fail = |msg: String| Err(CoreError::Character(format!("{}: {msg}", self.id)));
        if self.size == 0 || self.size > 128 {
            return fail(format!("tamaño {} fuera de rango", self.size));
        }
        let mut colors = 0;
        for (key, color) in &self.palette {
            if key.chars().count() != 1 {
                return fail(format!("la clave de paleta «{key}» debe ser un solo carácter"));
            }
            match color {
                Some(hex) if parse_hex(hex).is_none() => return fail(format!("color «{hex}» no es #RRGGBB")),
                Some(_) => colors += 1,
                None => {}
            }
        }
        if let Some(f) = self.face {
            if f.size == 0 || (f.x + f.size) as usize > self.size || (f.y + f.size) as usize > self.size {
                return fail("la cara se sale del personaje".into());
            }
        }
        if colors > MAX_COLORS {
            return fail(format!("{colors} colores; el máximo es {MAX_COLORS}"));
        }
        if !self.states.contains_key("idle") {
            return fail("falta el estado «idle»".into());
        }
        for (name, state) in &self.states {
            if state.fps == 0 || state.fps > MAX_FPS {
                return fail(format!("«{name}»: fps {} fuera de 1…{MAX_FPS}", state.fps));
            }
            if state.frames.is_empty() {
                return fail(format!("«{name}» no tiene fotogramas"));
            }
            for (f, frame) in state.frames.iter().enumerate() {
                if frame.len() != self.size {
                    return fail(format!("«{name}» fotograma {f}: {} filas, se esperaban {}", frame.len(), self.size));
                }
                for (r, row) in frame.iter().enumerate() {
                    let width = row.chars().count();
                    if width != self.size {
                        return fail(format!("«{name}» fotograma {f} fila {r}: {width} columnas"));
                    }
                    if let Some(bad) = row.chars().find(|c| !self.palette.contains_key(&c.to_string())) {
                        return fail(format!("«{name}» fotograma {f} fila {r}: «{bad}» no está en la paleta"));
                    }
                }
            }
        }
        Ok(())
    }

    /// Rasterizes every state. `idle` comes first; the rest follow in name order.
    pub fn rasterize(&self) -> Sprite {
        let argb: BTreeMap<char, u32> = self
            .palette
            .iter()
            .filter_map(|(k, v)| Some((k.chars().next()?, v.as_deref().and_then(parse_hex).unwrap_or(0))))
            .collect();
        let mut states: Vec<SpriteState> = self
            .states
            .iter()
            .map(|(name, state)| SpriteState {
                name: name.clone(),
                fps: state.fps,
                frames: state
                    .frames
                    .iter()
                    .map(|frame| frame.iter().flat_map(|row| row.chars().map(|c| argb[&c])).collect())
                    .collect(),
            })
            .collect();
        states.sort_by_key(|s| s.name != "idle");
        Sprite { id: self.id.clone(), name: self.name.clone(), size: self.size as u32, face: self.face, states }
    }
}

/// `#RRGGBB` → opaque `0xFFRRGGBB`.
fn parse_hex(hex: &str) -> Option<u32> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(|rgb| 0xFF00_0000 | rgb)
}

pub fn builtin_ids() -> Vec<&'static str> {
    BUILTIN.iter().map(|(id, _)| *id).collect()
}

pub fn builtin(id: &str) -> Result<Character, CoreError> {
    let (_, json) = BUILTIN
        .iter()
        .find(|(known, _)| *known == id)
        .ok_or_else(|| CoreError::Character(format!("no existe el personaje «{id}»")))?;
    Character::parse(json)
}

pub fn builtin_sprite(id: &str) -> Result<Sprite, CoreError> {
    builtin(id).map(|c| c.rasterize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny(frames: &str) -> String {
        format!(
            r##"{{"id":"t","name":"T","size":2,"palette":{{".":null,"k":"#000000","r":"#FF0000"}},
                "states":{{"idle":{{"fps":2,"frames":{frames}}}}}}}"##
        )
    }

    #[test]
    fn every_builtin_character_is_valid() {
        for id in builtin_ids() {
            let c = builtin(id).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(c.id, id, "the id inside the JSON matches its registry key");
        }
    }

    #[test]
    fn buddy_base_idle_moves_eyes_and_hands_without_moving_the_body() {
        let c = builtin("buddy-base").unwrap();
        assert_eq!(c.size, 48);
        let idle = &c.states["idle"];
        assert_eq!(idle.frames.len(), 8);
        let base = &idle.frames[0];
        let region = |frame: &Vec<String>, rows: std::ops::Range<usize>, cols: std::ops::Range<usize>| {
            rows.map(|y| frame[y][cols.clone()].to_string()).collect::<Vec<_>>()
        };
        assert_ne!(region(base, 22..26, 18..30), region(&idle.frames[2], 22..26, 18..30), "eyes look sideways");
        assert_ne!(region(base, 34..41, 12..15), region(&idle.frames[1], 34..41, 12..15), "left hand moves");
        assert_ne!(region(base, 34..41, 33..37), region(&idle.frames[5], 34..41, 33..37), "right hand moves");
        for frame in &idle.frames {
            assert_eq!(region(base, 42..48, 0..48), region(frame, 42..48, 0..48), "feet stay still");
        }
    }

    #[test]
    fn rasterizes_to_argb_with_idle_first() {
        let c = Character::parse(&tiny(r#"[["kr",".k"]]"#)).unwrap();
        let sprite = c.rasterize();
        assert_eq!(sprite.states[0].name, "idle");
        assert_eq!(sprite.states[0].frames[0], vec![0xFF00_0000, 0xFFFF_0000, 0, 0xFF00_0000]);
    }

    #[test]
    fn rejects_wrong_row_count() {
        assert!(Character::parse(&tiny(r#"[["kr"]]"#)).is_err());
    }

    #[test]
    fn rejects_wrong_row_width() {
        assert!(Character::parse(&tiny(r#"[["krk",".k"]]"#)).is_err());
    }

    #[test]
    fn rejects_colors_outside_the_palette() {
        assert!(Character::parse(&tiny(r#"[["kx",".k"]]"#)).is_err());
    }

    #[test]
    fn rejects_a_state_without_frames_and_a_missing_idle() {
        assert!(Character::parse(&tiny("[]")).is_err());
        let no_idle = tiny(r#"[["kr",".k"]]"#).replace("\"idle\"", "\"think\"");
        assert!(Character::parse(&no_idle).is_err());
    }

    #[test]
    fn rejects_bad_hex_and_too_many_colors() {
        assert!(Character::parse(&tiny(r#"[["kr",".k"]]"#).replace("#FF0000", "red")).is_err());
        let palette: Vec<String> = (0..17).map(|i| format!(r##""{}":"#00000{}""##, (b'a' + i) as char, i % 10)).collect();
        let json = format!(
            r#"{{"id":"t","name":"T","size":1,"palette":{{{}}},"states":{{"idle":{{"fps":1,"frames":[["a"]]}}}}}}"#,
            palette.join(",")
        );
        assert!(Character::parse(&json).is_err());
    }

    #[test]
    fn the_face_must_fit() {
        let with = |face: &str| tiny(r#"[["kr",".k"]]"#).replace(r#""size":2,"#, &format!(r#""size":2,"face":{face},"#));
        assert!(Character::parse(&with(r#"{"x":0,"y":0,"size":2}"#)).is_ok());
        assert!(Character::parse(&with(r#"{"x":1,"y":0,"size":2}"#)).is_err());
        assert_eq!(builtin_sprite("buddy-base").unwrap().face, Some(FaceRect { x: 7, y: 1, size: 34 }));
    }

    #[test]
    fn rejects_out_of_range_fps() {
        assert!(Character::parse(&tiny(r#"[["kr",".k"]]"#).replace("\"fps\":2", "\"fps\":60")).is_err());
    }
}
