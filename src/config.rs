// SPDX-License-Identifier: MIT
//! Runtime equivalents of Theurgy's small client/server configuration files.
//! The file is optional: defaults match upstream Java defaults. Tests can select
//! an isolated file with `THEURGY_CONFIG_FILE`.

use std::collections::HashMap;
use std::sync::OnceLock;

use mod_api::serde_json;

#[derive(Clone, Debug)]
pub struct Settings {
    pub render_sulfur_source_item: bool,
    pub enable_item_hud: bool,
    pub enable_in_world_hud: bool,
    pub enable_held_stack_fit_outline: bool,
    pub held_stack_fit_outline_render_mode: String,
    pub hud_scale: f32,
    pub wire_line_width: i32,
    pub use_simple_wire_renderer: bool,
    pub sulfur_source_to_block_mapping: HashMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            render_sulfur_source_item: true,
            enable_item_hud: true,
            enable_in_world_hud: true,
            enable_held_stack_fit_outline: true,
            held_stack_fit_outline_render_mode: "GREEN_ONLY".to_owned(),
            hud_scale: 0.7,
            wire_line_width: 2,
            use_simple_wire_renderer: false,
            sulfur_source_to_block_mapping: HashMap::new(),
        }
    }
}

fn json_bool(json: &serde_json::Value, key: &str, fallback: bool) -> bool {
    json.get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(fallback)
}

fn json_float(json: &serde_json::Value, key: &str, fallback: f32, min: f32, max: f32) -> f32 {
    json.get(key)
        .and_then(serde_json::Value::as_f64)
        .map_or(fallback, |v| (v as f32).clamp(min, max))
}

fn json_int(json: &serde_json::Value, key: &str, fallback: i32, min: i32, max: i32) -> i32 {
    json.get(key)
        .and_then(serde_json::Value::as_i64)
        .map_or(fallback, |v| (v as i32).clamp(min, max))
}

pub fn parse_json(text: &str) -> Settings {
    let json = serde_json::from_str::<serde_json::Value>(text).unwrap_or(serde_json::Value::Null);
    let defaults = Settings::default();
    let mut mapping = HashMap::new();
    if let Some(entries) = json
        .get("sulfurSourceToBlockMapping")
        .and_then(serde_json::Value::as_object)
    {
        for (source, target) in entries {
            if let Some(target) = target.as_str() {
                mapping.insert(source.clone(), target.to_owned());
            }
        }
    }
    Settings {
        render_sulfur_source_item: json_bool(
            &json,
            "renderSulfurSourceItem",
            defaults.render_sulfur_source_item,
        ),
        enable_item_hud: json_bool(&json, "enableItemHUD", defaults.enable_item_hud),
        enable_in_world_hud: json_bool(&json, "enableInWorldHUD", defaults.enable_in_world_hud),
        enable_held_stack_fit_outline: json_bool(
            &json,
            "enableHeldStackFitOutline",
            defaults.enable_held_stack_fit_outline,
        ),
        held_stack_fit_outline_render_mode: json
            .get("heldStackFitOutlineRenderMode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&defaults.held_stack_fit_outline_render_mode)
            .to_owned(),
        hud_scale: json_float(&json, "hudScale", defaults.hud_scale, 0.25, 3.0),
        wire_line_width: json_int(&json, "wireLineWidth", defaults.wire_line_width, 1, 64),
        use_simple_wire_renderer: json_bool(
            &json,
            "useSimpleWireRenderer",
            defaults.use_simple_wire_renderer,
        ),
        sulfur_source_to_block_mapping: mapping,
    }
}

fn load() -> Settings {
    let path = std::env::var("THEURGY_CONFIG_FILE").unwrap_or_else(|_| "config/theurgy.json".to_owned());
    match std::fs::read_to_string(path) {
        Ok(text) => parse_json(&text),
        Err(_) => Settings::default(),
    }
}

static SETTINGS: OnceLock<Settings> = OnceLock::new();

pub fn get() -> &'static Settings {
    SETTINGS.get_or_init(load)
}

pub fn source_override(source: &str) -> Option<&'static str> {
    let settings = get();
    settings
        .sulfur_source_to_block_mapping
        .get(source)
        .map(String::as_str)
        .or_else(|| {
            settings
                .sulfur_source_to_block_mapping
                .get(&format!("#{source}"))
                .map(String::as_str)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_java() {
        let settings = Settings::default();
        assert!(settings.render_sulfur_source_item);
        assert!(
            settings.enable_item_hud
                && settings.enable_in_world_hud
                && settings.enable_held_stack_fit_outline
        );
        assert_eq!(settings.held_stack_fit_outline_render_mode, "GREEN_ONLY");
        assert_eq!(settings.hud_scale, 0.7);
        assert_eq!(settings.wire_line_width, 2);
        assert!(!settings.use_simple_wire_renderer);
    }

    #[test]
    fn json_clamps_ranges_and_reads_overrides() {
        let s = parse_json(
            r##"{"hudScale": 9.0, "wireLineWidth": 0, "sulfurSourceToBlockMapping": {"#c:ores/iron": "minecraft:iron_ore"}}"##,
        );
        assert_eq!(s.hud_scale, 3.0);
        assert_eq!(s.wire_line_width, 1);
        assert_eq!(
            s.sulfur_source_to_block_mapping
                .get("#c:ores/iron")
                .map(String::as_str),
            Some("minecraft:iron_ore")
        );
    }
}
