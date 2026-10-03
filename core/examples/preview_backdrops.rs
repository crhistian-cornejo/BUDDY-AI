//! Prints every weather backdrop as JSON (`"<icon>|<night>"` → backdrop), for the Windows app's browser preview:
//! `cargo run --example preview_backdrops > ../apps/windows/preview/weather-backdrops.json`.

use buddy_core::cards::{ICONS, weather_backdrop};

fn main() {
    let mut all = serde_json::Map::new();
    for icon in ICONS {
        for night in [false, true] {
            all.insert(format!("{icon}|{night}"), serde_json::to_value(weather_backdrop(icon.to_string(), night)).unwrap());
        }
    }
    println!("{}", serde_json::Value::Object(all));
}
