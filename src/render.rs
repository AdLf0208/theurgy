// SPDX-License-Identifier: MIT
//! Block entity rendering for Theurgy apparatus:
//! - GeckoLib 5 `.geo.json` + `.animation.json` bone/cube baking and easing
//! - `DigestionVatRenderer` vanilla model parts
//! - `LiquefactionCauldron`, `SalAmmoniacTank`, `SalAmmoniacAccumulator` fluid quads
//! - Copper wire curves (`engine::wire`), reformation trails, held-stack fit outlines
//! - Vessel & pedestal floating items, bounding boxes, update tags, standalone atlas models

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, OnceLock};

use mod_api::serde_json::{self, Value};
use mod_api::{
    BlockEntityDrawContext, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent,
    BlockEntityUpdateTagEvent, BlockState, ClientHost, Compound, CubeFace, Host,
    ItemDisplayContext, ItemStack, ModBlockEntity, ModBlockEntityView, StandaloneModelsEvent, Tag,
};

use crate::engine::recipes::Tank;
use crate::engine::wire::{segment_quads, slack_points};
use crate::model_data::{ANIMATIONS, MODELS};
use crate::{config, filter, host, interactions, reformation, selection};

pub const CAULDRON_CAPACITY: i32 = 2_000;
const FLUID_BOTTOM_Y: f32 = 0.25;
const FLUID_HEIGHT: f32 = 0.875;
const FLUID_INSET: f32 = 0.25;

pub fn standalone_models(event: &mut StandaloneModelsEvent<'_>) {
    for id in [
        "calcination_oven",
        "distiller",
        "incubator_vessel_iron",
        "incubator_vessel_gold",
        "incubator_vessel_bronze",
        "logistics_nexus",
        "sal_ammoniac_accumulator",
        "sal_ammoniac_tank",
    ] {
        event
            .models
            .push(format!("theurgy:block/geckolib_atlas/{id}"));
    }
    for id in [
        "digestion_vat_base",
        "digestion_vat_base_open",
        "digestion_vat_side",
        "digestion_vat_side_front",
        "digestion_vat_side_front_active",
    ] {
        event
            .models
            .push(format!("theurgy:block/digestion_vat_atlas/{id}"));
    }
}

pub fn carries_model(block: &str) -> bool {
    matches!(
        block,
        "theurgy:calcination_oven"
            | "theurgy:distiller"
            | "theurgy:incubator_mercury_vessel"
            | "theurgy:incubator_salt_vessel"
            | "theurgy:incubator_sulfur_vessel"
            | "theurgy:logistics_nexus"
            | "theurgy:sal_ammoniac_accumulator"
            | "theurgy:sal_ammoniac_tank"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CauldronRenderState {
    pub empty: bool,
    pub color: u32,
    pub fluid_light: i32,
    pub bottom_y: f32,
    pub surface_y: f32,
    pub u0: f32,
    pub u1: f32,
    pub v0: f32,
    pub v1: f32,
    pub side_v0: f32,
}

impl CauldronRenderState {
    fn neutral() -> Self {
        Self {
            empty: true,
            color: 0xFFFF_FFFF,
            fluid_light: 0,
            bottom_y: 0.0,
            surface_y: 0.0,
            u0: 0.0,
            u1: 1.0,
            v0: 0.0,
            v1: 1.0,
            side_v0: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteBounds {
    pub u0: f32,
    pub u1: f32,
    pub v0: f32,
    pub v1: f32,
}

impl SpriteBounds {
    pub const FULL: Self = Self {
        u0: 0.0,
        u1: 1.0,
        v0: 0.0,
        v1: 1.0,
    };
}

impl Default for SpriteBounds {
    fn default() -> Self {
        Self::FULL
    }
}

fn solvent_tank(nbt: &Compound) -> Tank {
    host::tank_from_nbt(nbt, 0, CAULDRON_CAPACITY)
}

fn lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + (end - start) * delta
}

pub fn extract_cauldron(
    nbt: &Compound,
    light: i32,
    luminosity: i32,
    color: u32,
    sprite: SpriteBounds,
) -> CauldronRenderState {
    let tank = solvent_tank(nbt);
    if tank.fluid.is_none() || tank.amount <= 0 {
        return CauldronRenderState::neutral();
    }
    let capacity = tank.capacity.max(1);
    let fill_ratio = (tank.amount as f32 / capacity as f32).clamp(0.0, 1.0);
    let block_light_in = (light >> 4) & 0xF;
    let lit = block_light_in.max(luminosity.clamp(0, 0xF));
    let bottom_y = FLUID_BOTTOM_Y;
    let surface_y = bottom_y + fill_ratio * FLUID_HEIGHT;
    let side_v0 = lerp(1.0 - fill_ratio, sprite.v0, sprite.v1);
    CauldronRenderState {
        empty: false,
        color,
        fluid_light: (light & 0xF0_0000) | (lit << 4),
        bottom_y,
        surface_y,
        u0: sprite.u0,
        u1: sprite.u1,
        v0: sprite.v0,
        v1: sprite.v1,
        side_v0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub positions: [[f32; 3]; 4],
    pub uvs: [[f32; 2]; 4],
    pub color: u32,
    pub light: i32,
    pub normal: [f32; 3],
}

const UP: [f32; 3] = [0.0, 1.0, 0.0];
const NORTH: [f32; 3] = [0.0, 0.0, -1.0];
const SOUTH: [f32; 3] = [0.0, 0.0, 1.0];
const WEST: [f32; 3] = [-1.0, 0.0, 0.0];
const EAST: [f32; 3] = [1.0, 0.0, 0.0];

pub fn cauldron_quads(state: &CauldronRenderState) -> Vec<Quad> {
    fluid_quads(FLUID_INSET, state)
}

fn fluid_quads(inset: f32, state: &CauldronRenderState) -> Vec<Quad> {
    if state.empty {
        return Vec::new();
    }
    let min = inset;
    let max = 1.0 - min;
    let y_min = state.bottom_y;
    let y_max = state.surface_y;
    let mut quads = Vec::with_capacity(5);
    quads.push(Quad {
        positions: [
            [min, y_max, min],
            [min, y_max, max],
            [max, y_max, max],
            [max, y_max, min],
        ],
        uvs: [
            [state.u0, state.v0],
            [state.u0, state.v1],
            [state.u1, state.v1],
            [state.u1, state.v0],
        ],
        color: state.color,
        light: state.fluid_light,
        normal: UP,
    });
    let side = |normal: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| Quad {
        positions: [a, b, c, d],
        uvs: [
            [state.u0, state.v1],
            [state.u0, state.side_v0],
            [state.u1, state.side_v0],
            [state.u1, state.v1],
        ],
        color: state.color,
        light: state.fluid_light,
        normal,
    };
    quads.push(side(
        NORTH,
        [min, y_min, min],
        [min, y_max, min],
        [max, y_max, min],
        [max, y_min, min],
    ));
    quads.push(side(
        SOUTH,
        [max, y_min, max],
        [max, y_max, max],
        [min, y_max, max],
        [min, y_min, max],
    ));
    quads.push(side(
        WEST,
        [min, y_min, max],
        [min, y_max, max],
        [min, y_max, min],
        [min, y_min, min],
    ));
    quads.push(side(
        EAST,
        [max, y_min, min],
        [max, y_max, min],
        [max, y_max, max],
        [max, y_min, max],
    ));
    quads
}

pub fn cauldron_color(nbt: &Compound, fallback: u32) -> u32 {
    if solvent_tank(nbt).fluid.is_some() {
        host::int(nbt, "fluidColor", fallback as i32) as u32
    } else {
        fallback
    }
}

pub fn cauldron_sprite(nbt: &Compound) -> SpriteBounds {
    let Some(Tag::Compound(map)) = nbt.get(&"FluidSprite".into()) else {
        return SpriteBounds::FULL;
    };
    let read = |key: &str| -> f32 {
        match map.get(&key.into()) {
            Some(Tag::Float(v)) => f32::from_bits(*v),
            Some(Tag::Double(v)) => *v as f32,
            Some(Tag::Int(v)) => *v as f32,
            _ => 0.0,
        }
    };
    SpriteBounds {
        u0: read("u0"),
        u1: read("u1"),
        v0: read("v0"),
        v1: read("v1"),
    }
}

pub fn cauldron_bounding_box() -> ([f32; 3], [f32; 3]) {
    ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0])
}

/// Mirror slot items into `entity.nbt["Items"]` on the server side (where `&dyn Host`
/// is available) so `block_entity_update_tag` delivers them to client renderers.
pub fn mirror_client_items(host_ref: &dyn Host, entity: &mut ModBlockEntityView<'_>) {
    let items: Vec<Tag> = entity
        .items
        .iter()
        .enumerate()
        .filter_map(|(slot, stack)| {
            let stack = stack.as_ref()?;
            let mut record = stack.to_nbt(host_ref)?;
            record.insert("Slot".into(), Tag::Byte(slot as i8));
            Some(Tag::Compound(record))
        })
        .collect();
    if items.is_empty() {
        entity.nbt.remove(&"Items".into());
    } else {
        entity.nbt.insert("Items".into(), Tag::List(items));
    }
}

/// Client update tag (`block_entity_update_tag`): includes mirrored `Items`
/// and the live `isProcessing` flag.
pub fn update_tag(event: &mut BlockEntityUpdateTagEvent<'_>) -> Option<Compound> {
    let entity = event.entity;
    let mut nbt = entity.nbt.clone();
    nbt.insert(
        "isProcessing".into(),
        Tag::Byte(i8::from(entity.data.get(4).copied().unwrap_or(0) != 0)),
    );
    Some(nbt)
}

// ---------------------------------------------------------------------------
// GeckoLib JSON parsing (using mod_api::serde_json::Value directly)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct GeoModelFile {
    geometry: Vec<GeoGeometry>,
}

#[derive(Debug, Clone, Default)]
struct GeoGeometry {
    description: Option<GeoDescription>,
    bones: Vec<GeoBoneJson>,
}

#[derive(Debug, Clone, Default)]
struct GeoDescription {
    texture_width: Option<f32>,
    texture_height: Option<f32>,
    visible_bounds_width: Option<f32>,
    visible_bounds_height: Option<f32>,
    visible_bounds_offset: Option<[f32; 3]>,
}

#[derive(Debug, Clone, Default)]
struct GeoBoneJson {
    name: String,
    parent: Option<String>,
    pivot: [f32; 3],
    rotation: [f32; 3],
    cubes: Vec<GeoCubeJson>,
    bones: Vec<GeoBoneJson>,
    inflate: Option<f32>,
    mirror: Option<bool>,
    neverrender: bool,
}

#[derive(Debug, Clone)]
enum GeoUv {
    Array([f32; 2]),
    Faces(HashMap<String, GeoFaceUv>),
}

impl Default for GeoUv {
    fn default() -> Self {
        Self::Array([0.0, 0.0])
    }
}

#[derive(Debug, Clone, Default)]
struct GeoFaceUv {
    uv: [f32; 2],
    uv_size: [f32; 2],
    uv_rotation: i32,
}

#[derive(Debug, Clone, Default)]
struct GeoCubeJson {
    origin: [f32; 3],
    size: [f32; 3],
    uv: GeoUv,
    inflate: Option<f32>,
    mirror: Option<bool>,
    rotation: [f32; 3],
    pivot: [f32; 3],
}

#[derive(Debug, Clone)]
struct AnimationFile {
    animations: HashMap<String, AnimationDef>,
}

#[derive(Debug, Clone)]
struct AnimationDef {
    animation_length: Option<f32>,
    looping: bool,
    bones: HashMap<String, HashMap<String, Value>>,
}

fn vec3_opt(v: Option<&Value>) -> Option<[f32; 3]> {
    let arr = v?.as_array()?;
    if arr.len() != 3 {
        return None;
    }
    Some([
        arr[0].as_f64()? as f32,
        arr[1].as_f64()? as f32,
        arr[2].as_f64()? as f32,
    ])
}

fn vec2_opt(v: Option<&Value>) -> Option<[f32; 2]> {
    let arr = v?.as_array()?;
    if arr.len() != 2 {
        return None;
    }
    Some([arr[0].as_f64()? as f32, arr[1].as_f64()? as f32])
}

fn parse_geo_uv(v: Option<&Value>) -> GeoUv {
    let Some(v) = v else {
        return GeoUv::default();
    };
    if let Some(arr) = vec2_opt(Some(v)) {
        return GeoUv::Array(arr);
    }
    if let Some(obj) = v.as_object() {
        let mut map = HashMap::new();
        for (dir, face_val) in obj {
            let Some(face_obj) = face_val.as_object() else {
                continue;
            };
            let uv = vec2_opt(face_obj.get("uv")).unwrap_or([0.0, 0.0]);
            let uv_size = vec2_opt(face_obj.get("uv_size")).unwrap_or([0.0, 0.0]);
            let uv_rotation = face_obj
                .get("uv_rotation")
                .and_then(Value::as_i64)
                .unwrap_or(0) as i32;
            map.insert(
                dir.clone(),
                GeoFaceUv {
                    uv,
                    uv_size,
                    uv_rotation,
                },
            );
        }
        return GeoUv::Faces(map);
    }
    GeoUv::default()
}

fn parse_geo_cube(v: &Value) -> GeoCubeJson {
    GeoCubeJson {
        origin: vec3_opt(v.get("origin")).unwrap_or([0.0; 3]),
        size: vec3_opt(v.get("size")).unwrap_or([0.0; 3]),
        uv: parse_geo_uv(v.get("uv")),
        inflate: v.get("inflate").and_then(Value::as_f64).map(|n| n as f32),
        mirror: v.get("mirror").and_then(Value::as_bool),
        rotation: vec3_opt(v.get("rotation")).unwrap_or([0.0; 3]),
        pivot: vec3_opt(v.get("pivot")).unwrap_or([0.0; 3]),
    }
}

fn parse_geo_bone(v: &Value) -> Option<GeoBoneJson> {
    let name = v.get("name")?.as_str()?.to_owned();
    let cubes = v
        .get("cubes")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().map(parse_geo_cube).collect())
        .unwrap_or_default();
    let bones = v
        .get("bones")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_geo_bone).collect())
        .unwrap_or_default();
    Some(GeoBoneJson {
        name,
        parent: v.get("parent").and_then(Value::as_str).map(str::to_owned),
        pivot: vec3_opt(v.get("pivot")).unwrap_or([0.0; 3]),
        rotation: vec3_opt(v.get("rotation")).unwrap_or([0.0; 3]),
        cubes,
        bones,
        inflate: v.get("inflate").and_then(Value::as_f64).map(|n| n as f32),
        mirror: v.get("mirror").and_then(Value::as_bool),
        neverrender: v
            .get("neverrender")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_geo_model(text: &str) -> Option<GeoModelFile> {
    let root: Value = serde_json::from_str(text).ok()?;
    let geoms = root
        .get("minecraft:geometry")
        .and_then(Value::as_array)?
        .iter()
        .map(|g| {
            let description = g.get("description").map(|d| GeoDescription {
                texture_width: d.get("texture_width").and_then(Value::as_f64).map(|n| n as f32),
                texture_height: d
                    .get("texture_height")
                    .and_then(Value::as_f64)
                    .map(|n| n as f32),
                visible_bounds_width: d
                    .get("visible_bounds_width")
                    .and_then(Value::as_f64)
                    .map(|n| n as f32),
                visible_bounds_height: d
                    .get("visible_bounds_height")
                    .and_then(Value::as_f64)
                    .map(|n| n as f32),
                visible_bounds_offset: vec3_opt(d.get("visible_bounds_offset")),
            });
            let bones = g
                .get("bones")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(parse_geo_bone).collect())
                .unwrap_or_default();
            GeoGeometry { description, bones }
        })
        .collect();
    Some(GeoModelFile { geometry: geoms })
}

fn parse_animation_file(text: &str) -> Option<AnimationFile> {
    let root: Value = serde_json::from_str(text).ok()?;
    let anims_obj = root.get("animations")?.as_object()?;
    let mut animations = HashMap::new();
    for (name, val) in anims_obj {
        let animation_length = val
            .get("animation_length")
            .and_then(Value::as_f64)
            .map(|n| n as f32);
        let looping = match val.get("loop") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s == "true",
            _ => false,
        };
        let mut bones = HashMap::new();
        if let Some(bone_map) = val.get("bones").and_then(Value::as_object) {
            for (bone_name, channels_val) in bone_map {
                if let Some(ch_obj) = channels_val.as_object() {
                    let mut channels = HashMap::new();
                    for (ch_name, ch_val) in ch_obj {
                        channels.insert(ch_name.clone(), ch_val.clone());
                    }
                    bones.insert(bone_name.clone(), channels);
                }
            }
        }
        animations.insert(
            name.clone(),
            AnimationDef {
                animation_length,
                looping,
                bones,
            },
        );
    }
    Some(AnimationFile { animations })
}

fn models() -> &'static HashMap<&'static str, GeoModelFile> {
    static MODELS_ONCE: OnceLock<HashMap<&'static str, GeoModelFile>> = OnceLock::new();
    MODELS_ONCE.get_or_init(|| {
        MODELS
            .iter()
            .filter_map(|(name, text)| parse_geo_model(text).map(|m| (*name, m)))
            .collect()
    })
}

fn animations() -> &'static HashMap<&'static str, AnimationFile> {
    static ANIMATIONS_ONCE: OnceLock<HashMap<&'static str, AnimationFile>> = OnceLock::new();
    ANIMATIONS_ONCE.get_or_init(|| {
        ANIMATIONS
            .iter()
            .filter_map(|(name, text)| parse_animation_file(text).map(|a| (*name, a)))
            .collect()
    })
}

fn model_name(block: &str) -> Option<&'static str> {
    match block {
        "theurgy:calcination_oven" => Some("calcination_oven.geo.json"),
        "theurgy:distiller" => Some("distiller.geo.json"),
        "theurgy:incubator_mercury_vessel"
        | "theurgy:incubator_salt_vessel"
        | "theurgy:incubator_sulfur_vessel" => Some("incubator_vessel.geo.json"),
        "theurgy:logistics_nexus" => Some("logistics_nexus.geo.json"),
        "theurgy:sal_ammoniac_accumulator" => Some("sal_ammoniac_accumulator.geo.json"),
        "theurgy:sal_ammoniac_tank" => Some("sal_ammoniac_tank.geo.json"),
        _ => None,
    }
}

fn animation_name(block: &str) -> Option<&'static str> {
    match block {
        "theurgy:calcination_oven" => Some("calcination_oven.animation.json"),
        "theurgy:distiller" => Some("distiller.animation.json"),
        _ => None,
    }
}

fn model_texture(block: &str) -> &'static str {
    match block {
        "theurgy:calcination_oven" => "theurgy:block/calcination_oven",
        "theurgy:distiller" => "theurgy:block/distiller",
        "theurgy:incubator_mercury_vessel" => "theurgy:block/incubator_vessel_gold",
        "theurgy:incubator_salt_vessel" => "theurgy:block/incubator_vessel_bronze",
        "theurgy:incubator_sulfur_vessel" => "theurgy:block/incubator_vessel_iron",
        "theurgy:logistics_nexus" => "theurgy:block/logistics_nexus",
        "theurgy:sal_ammoniac_accumulator" => "theurgy:block/sal_ammoniac_accumulator",
        "theurgy:sal_ammoniac_tank" => "theurgy:block/sal_ammoniac_tank",
        _ => "theurgy:block/distiller",
    }
}

#[derive(Clone, Copy)]
struct AnimationTrack {
    processing: bool,
    changed_at: f32,
    last_seen: f32,
}

static ANIMATION_TRACKS: LazyLock<Mutex<HashMap<(String, [i32; 3]), AnimationTrack>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn animation_id_at(
    block: &str,
    pos: [i32; 3],
    processing: bool,
    now: f32,
) -> Option<(&'static str, f32)> {
    if !matches!(block, "theurgy:calcination_oven" | "theurgy:distiller") {
        return None;
    }
    let mut tracks = ANIMATION_TRACKS.lock().ok()?;
    if tracks.len() > 256 {
        tracks.retain(|_, track| now - track.last_seen < 120.0);
    }
    let track = tracks
        .entry((block.to_owned(), pos))
        .or_insert(AnimationTrack {
            processing,
            changed_at: f32::NEG_INFINITY,
            last_seen: now,
        });
    if track.processing != processing {
        track.processing = processing;
        track.changed_at = now;
    }
    track.last_seen = now;
    let transition = if processing {
        if block == "theurgy:distiller" {
            "animation.distiller.start"
        } else {
            "animation.calcination_oven.start"
        }
    } else if block == "theurgy:distiller" {
        "animation.distiller.stop"
    } else {
        "animation.calcination_oven.stop"
    };
    let looping = if processing {
        if block == "theurgy:distiller" {
            "animation.distiller.on"
        } else {
            "animation.calcination_oven.on"
        }
    } else if block == "theurgy:distiller" {
        "animation.distiller.off"
    } else {
        "animation.calcination_oven.off"
    };
    let elapsed = now - track.changed_at;
    let length = animations()
        .get(animation_name(block)?)
        .and_then(|file| file.animations.get(transition))
        .and_then(|animation| animation.animation_length)
        .unwrap_or(0.0);
    if elapsed >= 0.0 && elapsed < length {
        Some((transition, elapsed))
    } else {
        Some((looping, now))
    }
}

fn ease(name: &str, time: f32, args: &[f32]) -> f32 {
    let t = time.clamp(0.0, 1.0);
    match name {
        "easeInOutCubic" => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        "easeInOutQuart" => {
            if t < 0.5 {
                8.0 * t.powi(4)
            } else {
                1.0 - (-2.0 * t + 2.0).powi(4) / 2.0
            }
        }
        "easeInOutQuint" => {
            if t < 0.5 {
                16.0 * t.powi(5)
            } else {
                1.0 - (-2.0 * t + 2.0).powi(5) / 2.0
            }
        }
        "easeInOutBack" => {
            let n = args.first().map_or(1.70158, |arg| arg * 1.70158);
            let back = |x: f32| x * x * ((n + 1.0) * x - n);
            if t < 0.5 {
                back(t * 2.0) / 2.0
            } else {
                1.0 - back((1.0 - t) * 2.0) / 2.0
            }
        }
        _ => t,
    }
}

fn sample(def: &AnimationDef, bone: &str, channel: &str, seconds: f32) -> Option<[f32; 3]> {
    let value = def.bones.get(bone)?.get(channel)?;
    if let Some(static_vector) = vec3_opt(value.get("vector")) {
        return Some(static_vector);
    }
    let object = value.as_object()?;
    let mut frames = Vec::new();
    for (key, frame) in object {
        let Ok(time) = key.parse::<f32>() else {
            continue;
        };
        let Some(vector) = vec3_opt(frame.get("vector")) else {
            continue;
        };
        let easing = frame
            .get("easing")
            .and_then(Value::as_str)
            .unwrap_or("linear")
            .to_owned();
        let args: Vec<f32> = frame
            .get("easingArgs")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(Value::as_f64)
                    .map(|v| v as f32)
                    .collect()
            })
            .unwrap_or_default();
        frames.push((time, vector, easing, args));
    }
    frames.sort_by(|a, b| a.0.total_cmp(&b.0));
    if frames.is_empty() {
        return None;
    }
    let length = def
        .animation_length
        .unwrap_or(frames.last()?.0.max(f32::EPSILON));
    let time = if def.looping && length > 0.0 {
        seconds.rem_euclid(length)
    } else {
        seconds.clamp(0.0, length)
    };
    if time <= frames[0].0 {
        return Some(frames[0].1);
    }
    for pair in frames.windows(2) {
        let (ta, a) = (pair[0].0, pair[0].1);
        let (tb, b) = (pair[1].0, pair[1].1);
        if time <= tb {
            let factor = if tb > ta {
                ((time - ta) / (tb - ta)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let factor = ease(&pair[1].2, factor, &pair[1].3);
            return Some([
                a[0] + (b[0] - a[0]) * factor,
                a[1] + (b[1] - a[1]) * factor,
                a[2] + (b[2] - a[2]) * factor,
            ]);
        }
    }
    frames.last().map(|frame| frame.1)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeoFace {
    pub direction: &'static str,
    pub corners: [[f32; 3]; 4],
    pub uv: [f32; 4],
}

fn zero_size_face(size: [f32; 3], direction: &str) -> bool {
    let axis_x = matches!(direction, "west" | "east");
    let axis_y = matches!(direction, "up" | "down");
    let axis_z = matches!(direction, "north" | "south");
    if size[0] == 0.0 {
        !axis_x
    } else if size[1] == 0.0 {
        !axis_y
    } else if size[2] == 0.0 {
        !axis_z
    } else {
        false
    }
}

fn box_uv(direction: &str, u: f32, v: f32, size: [f32; 3]) -> ([f32; 2], [f32; 2]) {
    let [x, y, z] = size.map(f32::floor);
    match direction {
        "west" => ([u + z + x, v + z], [z, y]),
        "east" => ([u, v + z], [z, y]),
        "north" => ([u + z, v + z], [x, y]),
        "south" => ([u + z + x + z, v + z], [x, y]),
        "up" => ([u + z, v], [x, z]),
        _ => ([u + z + x, v + z], [x, -z]),
    }
}

fn geo_cube_faces(
    cube: &GeoCubeJson,
    bone_inflate: Option<f32>,
    bone_mirror: Option<bool>,
    texture_width: f32,
    texture_height: f32,
) -> Vec<GeoFace> {
    let mirror = cube.mirror.unwrap_or(bone_mirror == Some(true));
    let inflate = cube.inflate.or(bone_inflate).unwrap_or(0.0) / 16.0;
    let size = cube.size;
    let origin = [
        -(cube.origin[0] + size[0]) / 16.0,
        cube.origin[1] / 16.0,
        cube.origin[2] / 16.0,
    ];
    let (x0, y0, z0) = (origin[0] - inflate, origin[1] - inflate, origin[2] - inflate);
    let (x1, y1, z1) = (
        origin[0] + size[0] / 16.0 + inflate,
        origin[1] + size[1] / 16.0 + inflate,
        origin[2] + size[2] / 16.0 + inflate,
    );
    let bottom_left_back = [x0, y0, z0];
    let bottom_right_back = [x0, y0, z1];
    let top_left_back = [x0, y1, z0];
    let top_right_back = [x0, y1, z1];
    let top_left_front = [x1, y1, z0];
    let top_right_front = [x1, y1, z1];
    let bottom_left_front = [x1, y0, z0];
    let bottom_right_front = [x1, y0, z1];
    let west = [top_right_back, top_left_back, bottom_left_back, bottom_right_back];
    let east = [top_left_front, top_right_front, bottom_right_front, bottom_left_front];
    let north = [top_left_back, top_left_front, bottom_left_front, bottom_left_back];
    let south = [top_right_front, top_right_back, bottom_right_back, bottom_right_front];
    let up = [top_right_back, top_right_front, top_left_front, top_left_back];
    let down = [bottom_left_back, bottom_left_front, bottom_right_front, bottom_right_back];
    let box_uvs = matches!(cube.uv, GeoUv::Array(_));
    let mut faces = Vec::with_capacity(6);
    for direction in ["west", "east", "north", "south", "up", "down"] {
        if zero_size_face(size, direction) {
            continue;
        }
        let (uv, uv_size, rotation) = match &cube.uv {
            GeoUv::Array(base) => {
                let (uv, uv_size) = box_uv(direction, base[0], base[1], size);
                (uv, uv_size, 0)
            }
            GeoUv::Faces(map) => match map.get(direction) {
                Some(face) => (face.uv, face.uv_size, face.uv_rotation),
                None => continue,
            },
        };
        let vertices = match direction {
            "west" => if mirror { east } else { west },
            "east" => if mirror { west } else { east },
            "north" => north,
            "south" => south,
            "up" => if mirror && !box_uvs { down } else { up },
            _ => if mirror && !box_uvs { up } else { down },
        };
        let left = uv[0] / texture_width * 16.0;
        let right = (uv[0] + uv_size[0]) / texture_width * 16.0;
        let top = uv[1] / texture_height * 16.0;
        let bottom = (uv[1] + uv_size[1]) / texture_height * 16.0;
        let (u, u_width) = if mirror { (left, right) } else { (right, left) };
        let start = match ((rotation % 360) + 360) % 360 / 90 {
            1 => 0,
            2 => 3,
            3 => 2,
            _ => 1,
        };
        let corners = std::array::from_fn(|index| vertices[(start + index) % 4]);
        faces.push(GeoFace {
            direction,
            corners,
            uv: [u_width, top, u, bottom],
        });
    }
    faces
}

fn geo_facing_rotation(facing: &str) -> ([f32; 3], f32) {
    match facing {
        "south" => ([0.0, 1.0, 0.0], 180.0),
        "west" => ([0.0, 1.0, 0.0], 90.0),
        "east" => ([0.0, 1.0, 0.0], -90.0),
        "up" => ([1.0, 0.0, 0.0], 90.0),
        "down" => ([1.0, 0.0, 0.0], -90.0),
        _ => ([0.0, 1.0, 0.0], 0.0),
    }
}

fn geo_pivot(values: [f32; 3]) -> [f32; 3] {
    [-values[0] / 16.0, values[1] / 16.0, values[2] / 16.0]
}

fn geo_rotation(values: [f32; 3]) -> [f32; 3] {
    [-values[0], -values[1], values[2]]
}

fn rotate_zyx(draw: &mut dyn BlockEntityDrawContext, degrees: [f32; 3]) {
    if degrees[2] != 0.0 {
        draw.rotate_about([0.0, 0.0, 1.0], degrees[2], [0.0; 3]);
    }
    if degrees[1] != 0.0 {
        draw.rotate_about([0.0, 1.0, 0.0], degrees[1], [0.0; 3]);
    }
    if degrees[0] != 0.0 {
        draw.rotate_about([1.0, 0.0, 0.0], degrees[0], [0.0; 3]);
    }
}

#[derive(Clone, Copy)]
struct FlatBone<'a> {
    bone: &'a GeoBoneJson,
    parent: Option<&'a str>,
}

fn flatten_bones<'a>(
    source: &'a [GeoBoneJson],
    inherited_parent: Option<&'a str>,
    out: &mut Vec<FlatBone<'a>>,
) {
    for bone in source {
        let parent = bone.parent.as_deref().or(inherited_parent);
        out.push(FlatBone { bone, parent });
        flatten_bones(&bone.bones, Some(&bone.name), out);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_bone(
    draw: &mut dyn BlockEntityDrawContext,
    bone: &GeoBoneJson,
    bones: &[FlatBone<'_>],
    hidden: &[&str],
    animation: Option<&AnimationDef>,
    seconds: f32,
    texture: &str,
    texture_width: f32,
    texture_height: f32,
) {
    if bone.neverrender || hidden.contains(&bone.name.as_str()) {
        return;
    }
    draw.push();
    if let Some(offset) = animation.and_then(|def| sample(def, &bone.name, "position", seconds)) {
        draw.translate([-offset[0] / 16.0, offset[1] / 16.0, offset[2] / 16.0]);
    }
    let bone_pivot = geo_pivot(bone.pivot);
    draw.translate(bone_pivot);
    let base = geo_rotation(bone.rotation);
    let animated = geo_rotation(
        animation
            .and_then(|def| sample(def, &bone.name, "rotation", seconds))
            .unwrap_or([0.0; 3]),
    );
    rotate_zyx(
        draw,
        [
            base[0] + animated[0],
            base[1] + animated[1],
            base[2] + animated[2],
        ],
    );
    if let Some(scale) = animation.and_then(|def| sample(def, &bone.name, "scale", seconds)) {
        draw.scale(scale);
    }
    draw.translate(bone_pivot.map(|v| -v));

    // shortcut: SDK lacks END_PORTAL_LAYER quad submission, so the nexus Portal
    // bone renders with a dark end-like tint until an end_portal_quad hook exists.
    let portal = bone.name == "Portal" && texture == "theurgy:block/logistics_nexus";
    for cube in &bone.cubes {
        draw.push();
        let cube_pivot = geo_pivot(cube.pivot);
        draw.translate(cube_pivot);
        rotate_zyx(draw, geo_rotation(cube.rotation));
        draw.translate(cube_pivot.map(|v| -v));
        for face in geo_cube_faces(cube, bone.inflate, bone.mirror, texture_width, texture_height) {
            if portal {
                let _ = draw.quad("block/obsidian", face.corners, [0.0, 0.0, 16.0, 16.0], 0x201040);
            } else {
                let _ = draw.quad(texture, face.corners, face.uv, 0xFF_FFFF);
            }
        }
        draw.pop();
    }
    for child in bones
        .iter()
        .filter(|child| child.parent == Some(bone.name.as_str()))
    {
        render_bone(
            draw,
            child.bone,
            bones,
            hidden,
            animation,
            seconds,
            texture,
            texture_width,
            texture_height,
        );
    }
    draw.pop();
}

// ---------------------------------------------------------------------------
// Vanilla DigestionVatRenderer parts
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) struct VanillaCube {
    pub tex: [f32; 2],
    pub origin: [f32; 3],
    pub size: [f32; 3],
    pub grow: f32,
    pub north_only: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct VanillaPart {
    pub offset: [f32; 3],
    pub rotation: [f32; 3],
    pub cubes: &'static [VanillaCube],
    pub tex_size: [f32; 2],
}

pub(crate) fn vanilla_cube_faces(cube: &VanillaCube, tex_size: [f32; 2]) -> Vec<GeoFace> {
    let [dx, dy, dz] = cube.size;
    let min = cube.origin.map(|value| (value - cube.grow) / 16.0);
    let max = [0, 1, 2].map(|i| (cube.origin[i] + cube.size[i] + cube.grow) / 16.0);
    let v = [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [max[0], max[1], max[2]],
        [min[0], max[1], max[2]],
    ];
    let [u, tv] = cube.tex;
    let (u0, u1, u2, u3, u4, u5) = (
        u,
        u + dz,
        u + dz + dx,
        u + dz + dx + dx,
        u + dz + dx + dz,
        u + dz + dx + dz + dx,
    );
    let (v0, v1, v2) = (tv, tv + dz, tv + dz + dy);
    let polygons: [(&'static str, [usize; 4], [f32; 4], bool); 6] = [
        ("down", [5, 4, 0, 1], [u1, v0, u2, v1], dx == 0.0 || dz == 0.0),
        ("up", [2, 3, 7, 6], [u2, v1, u3, v0], dx == 0.0 || dz == 0.0),
        ("west", [0, 4, 7, 3], [u0, v1, u1, v2], dz == 0.0 || dy == 0.0),
        ("north", [1, 0, 3, 2], [u1, v1, u2, v2], dx == 0.0 || dy == 0.0),
        ("east", [5, 1, 2, 6], [u2, v1, u4, v2], dz == 0.0 || dy == 0.0),
        ("south", [4, 5, 6, 7], [u4, v1, u5, v2], dx == 0.0 || dy == 0.0),
    ];
    let mut faces = Vec::new();
    for (direction, indices, [a, b, c, d], empty) in polygons {
        if empty || (cube.north_only && direction != "north") {
            continue;
        }
        let corners = [v[indices[1]], v[indices[2]], v[indices[3]], v[indices[0]]];
        let uv = [
            a / tex_size[0] * 16.0,
            b / tex_size[1] * 16.0,
            c / tex_size[0] * 16.0,
            d / tex_size[1] * 16.0,
        ];
        faces.push(GeoFace {
            direction,
            corners,
            uv,
        });
    }
    faces
}

const DIGESTION_NECK: &[VanillaCube] = &[
    VanillaCube {
        tex: [0.0, 0.0],
        origin: [4.0, 17.0, 4.0],
        size: [8.0, 3.0, 8.0],
        grow: -0.1,
        north_only: false,
    },
    VanillaCube {
        tex: [0.0, 5.0],
        origin: [5.0, 20.0, 5.0],
        size: [6.0, 1.0, 6.0],
        grow: 0.2,
        north_only: false,
    },
];
const DIGESTION_PLANE: &[VanillaCube] = &[VanillaCube {
    tex: [-14.0, 13.0],
    origin: [0.0; 3],
    size: [14.0, 0.0, 14.0],
    grow: 0.0,
    north_only: false,
}];
const DIGESTION_SIDE: &[VanillaCube] = &[VanillaCube {
    tex: [1.0, 0.0],
    origin: [0.0; 3],
    size: [14.0, 16.0, 0.0],
    grow: 0.0,
    north_only: true,
}];
const PI: f32 = std::f32::consts::PI;

pub(crate) const DIGESTION_BASE_PARTS: [VanillaPart; 3] = [
    VanillaPart {
        offset: [0.0, 37.0, 16.0],
        rotation: [PI, 0.0, 0.0],
        cubes: DIGESTION_NECK,
        tex_size: [32.0, 32.0],
    },
    VanillaPart {
        offset: [1.0, 16.0, 1.0],
        rotation: [0.0; 3],
        cubes: DIGESTION_PLANE,
        tex_size: [32.0, 32.0],
    },
    VanillaPart {
        offset: [1.0, 0.0, 1.0],
        rotation: [0.0; 3],
        cubes: DIGESTION_PLANE,
        tex_size: [32.0, 32.0],
    },
];
pub(crate) const DIGESTION_SIDE_PARTS: [VanillaPart; 4] = [
    VanillaPart {
        offset: [1.0, 16.0, 15.0],
        rotation: [PI, 0.0, 0.0],
        cubes: DIGESTION_SIDE,
        tex_size: [16.0, 16.0],
    },
    VanillaPart {
        offset: [15.0, 16.0, 1.0],
        rotation: [0.0, 0.0, PI],
        cubes: DIGESTION_SIDE,
        tex_size: [16.0, 16.0],
    },
    VanillaPart {
        offset: [1.0, 16.0, 1.0],
        rotation: [0.0, -PI / 2.0, PI],
        cubes: DIGESTION_SIDE,
        tex_size: [16.0, 16.0],
    },
    VanillaPart {
        offset: [15.0, 16.0, 15.0],
        rotation: [0.0, PI / 2.0, PI],
        cubes: DIGESTION_SIDE,
        tex_size: [16.0, 16.0],
    },
];

fn horizontal_y_rot(facing: &str) -> f32 {
    match facing {
        "south" => 0.0,
        "west" => 90.0,
        "east" => 270.0,
        _ => 180.0,
    }
}

fn state_prop(host_ref: &dyn Host, state: BlockState, key: &str) -> Option<String> {
    let s = host_ref.block_state_string(state)?;
    host::parse_state_property(&s, key).map(str::to_owned)
}

fn client_stack(host_ref: &dyn Host, entity: &ModBlockEntity) -> Option<ItemStack> {
    let Tag::List(items) = entity.nbt.get(&"Items".into())? else {
        return None;
    };
    for item in items {
        if let Some(stack) = filter::stack_from_template(host_ref, item) {
            return Some(stack);
        }
    }
    None
}

fn digestion_has_output(host_ref: &dyn Host, entity: &ModBlockEntity) -> bool {
    if entity.items.get(3).is_some_and(Option::is_some) {
        return true;
    }
    let Some(Tag::List(items)) = entity.nbt.get(&"Items".into()) else {
        return false;
    };
    items.iter().any(|item| {
        let Tag::Compound(record) = item else {
            return false;
        };
        matches!(record.get(&"Slot".into()), Some(Tag::Byte(3)))
            && filter::stack_from_template(host_ref, item).is_some()
    })
}

fn submit_vanilla_part(
    draw: &mut dyn BlockEntityDrawContext,
    part: &VanillaPart,
    sprite: &str,
    hidden: Option<&str>,
) {
    draw.push();
    draw.translate(part.offset.map(|value| value / 16.0));
    rotate_zyx(draw, part.rotation.map(f32::to_degrees));
    for cube in part.cubes {
        for face in vanilla_cube_faces(cube, part.tex_size) {
            if Some(face.direction) == hidden {
                continue;
            }
            let _ = draw.quad(sprite, face.corners, face.uv, 0xFF_FFFF);
        }
    }
    draw.pop();
}

fn draw_digestion_vat(
    host_ref: &dyn ClientHost,
    state: BlockState,
    entity: &ModBlockEntity,
    draw: &mut dyn BlockEntityDrawContext,
) {
    let facing = state_prop(host_ref, state, "facing").unwrap_or_else(|| "north".to_owned());
    let open = state_prop(host_ref, state, "open").as_deref() != Some("false");
    let active = digestion_has_output(host_ref, entity);
    draw.push();
    draw.translate([0.5, 0.0, 0.5]);
    draw.rotate_about([0.0, 1.0, 0.0], 180.0 - horizontal_y_rot(&facing), [0.0; 3]);
    draw.translate([-0.5, 0.0, -0.5]);
    let base = if open {
        "theurgy:block/digestion_vat/digestion_vat_base_open"
    } else {
        "theurgy:block/digestion_vat/digestion_vat_base"
    };
    for (part, hidden) in DIGESTION_BASE_PARTS
        .iter()
        .zip([None, Some("down"), Some("up")])
    {
        submit_vanilla_part(draw, part, base, hidden);
    }
    let front = if active {
        "theurgy:block/digestion_vat/digestion_vat_side_front_active"
    } else {
        "theurgy:block/digestion_vat/digestion_vat_side_front"
    };
    for (index, part) in DIGESTION_SIDE_PARTS.iter().enumerate() {
        submit_vanilla_part(
            draw,
            part,
            if index == 0 {
                front
            } else {
                "theurgy:block/digestion_vat/digestion_vat_side"
            },
            None,
        );
    }
    draw.pop();
    draw.keep();
}

fn fluid_tank(entity: &ModBlockEntity, capacity: i32, java_key: &str) -> Tank {
    let native = host::tank_from_nbt(&entity.nbt, 0, capacity);
    if native.amount > 0 {
        return native;
    }
    let Some(Tag::Compound(outer)) = entity.nbt.get(&java_key.into()) else {
        return native;
    };
    let Some(Tag::Compound(fluid)) = outer.get(&"Fluid".into()) else {
        return native;
    };
    let Some(id) = host::string(fluid, "id") else {
        return native;
    };
    let amount = host::int(fluid, "amount", 0).clamp(0, capacity);
    if amount == 0 {
        return native;
    }
    Tank {
        fluid: Some(id),
        amount,
        capacity,
    }
}

fn box_fluid_state(tank: &Tank, light: i32, bottom: f32, surface: f32) -> CauldronRenderState {
    let fill = (tank.amount as f32 / tank.capacity.max(1) as f32).clamp(0.0, 1.0);
    let block_light = (light >> 4) & 0xF;
    CauldronRenderState {
        empty: tank.amount <= 0,
        color: 0xFFFF_FFFF,
        fluid_light: (light & 0xF0_0000) | (block_light << 4),
        bottom_y: bottom,
        surface_y: surface,
        u0: 0.0,
        u1: 1.0,
        v0: 0.0,
        v1: 1.0,
        side_v0: 1.0 - fill,
    }
}

fn submit_fluid_quads(
    draw: &mut dyn BlockEntityDrawContext,
    sprite: &str,
    state: &CauldronRenderState,
    inset: f32,
) {
    if state.empty {
        return;
    }
    let old_light = draw.light();
    draw.set_light(state.fluid_light as u32);
    for quad in fluid_quads(inset, state) {
        let u0 = quad.uvs.iter().map(|uv| uv[0]).fold(f32::INFINITY, f32::min);
        let u1 = quad
            .uvs
            .iter()
            .map(|uv| uv[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let v0 = quad.uvs.iter().map(|uv| uv[1]).fold(f32::INFINITY, f32::min);
        let v1 = quad
            .uvs
            .iter()
            .map(|uv| uv[1])
            .fold(f32::NEG_INFINITY, f32::max);
        let _ = draw.quad(
            sprite,
            quad.positions,
            [u0 * 16.0, v0 * 16.0, u1 * 16.0, v1 * 16.0],
            state.color & 0xFF_FFFF,
        );
    }
    draw.set_light(old_light);
}

fn draw_wire(pos: [i32; 3], entity: &ModBlockEntity, draw: &mut dyn BlockEntityDrawContext) {
    let settings = config::get();
    let Some(Tag::List(records)) = entity.nbt.get(&"wireConnections".into()) else {
        return;
    };
    let width = settings.wire_line_width.clamp(1, 64) as f32 / 64.0;
    for record in records {
        let Tag::Compound(record) = record else {
            continue;
        };
        let (Some(Tag::Long(from)), Some(Tag::Long(to))) =
            (record.get(&"from".into()), record.get(&"to".into()))
        else {
            continue;
        };
        let from = crate::items::block_pos_from_long(*from);
        let to = crate::items::block_pos_from_long(*to);
        if from != pos && to != pos {
            continue;
        }
        let base = [0, 1, 2].map(|i| (from[i] - pos[i]) as f32 + 0.5);
        let delta = [0, 1, 2].map(|i| f64::from(to[i] - from[i]));
        let points: Vec<[f32; 3]> = if settings.use_simple_wire_renderer {
            vec![[0.0; 3], delta.map(|v| v as f32)]
        } else {
            slack_points(delta).to_vec()
        };
        for pair in points.windows(2) {
            let [a, b] = [pair[0], pair[1]].map(|p| [0, 1, 2].map(|i| base[i] + p[i]));
            for corners in segment_quads(a, b, width) {
                let _ = draw.quad(
                    "block/white_concrete",
                    corners,
                    [7.0, 7.0, 9.0, 9.0],
                    0xB8_7333,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_reformation_trail(
    draw: &mut dyn BlockEntityDrawContext,
    origin: [i32; 3],
    from: [f64; 3],
    to: [f64; 3],
    start: [f32; 3],
    end: [f32; 3],
    phase: f32,
) {
    let face = Some(CubeFace::new("theurgy:block/distiller").with_uv([0.0, 0.0, 16.0, 16.0]));
    for step in 0..=12 {
        let t = (step as f32 / 12.0 + phase).fract();
        let color = reformation::trail_color(start, end, t);
        let packed = (((color[0].clamp(0.0, 1.0) * 255.0) as u32) << 16)
            | (((color[1].clamp(0.0, 1.0) * 255.0) as u32) << 8)
            | (color[2].clamp(0.0, 1.0) * 255.0) as u32;
        let half = 0.12;
        let from_texels = std::array::from_fn(|axis| {
            ((from[axis] + (to[axis] - from[axis]) * f64::from(t)
                - f64::from(origin[axis])
                - half)
                * 16.0) as f32
        });
        let to_texels = std::array::from_fn(|axis| {
            ((from[axis] + (to[axis] - from[axis]) * f64::from(t)
                - f64::from(origin[axis])
                + half)
                * 16.0) as f32
        });
        let _ = draw.cube(from_texels, to_texels, [face; 6], packed);
    }
}

fn draw_reformation(
    host_ref: &dyn ClientHost,
    pos: [i32; 3],
    state: BlockState,
    entity: &ModBlockEntity,
    draw: &mut dyn BlockEntityDrawContext,
) {
    if entity.id() != "theurgy:sulfuric_flux_emitter" {
        return;
    }
    let processing = entity.data.get(4).copied().unwrap_or(0) != 0
        || host::bool(&entity.nbt, "isProcessing")
        || host::int(&entity.nbt, "progress", 0) > 0;
    if !processing {
        return;
    }
    let sources = selection::read_points(&entity.nbt, "sourcePedestals");
    let Some(target) = selection::read_point(&entity.nbt, "targetPedestal") else {
        return;
    };
    let Some(result) = selection::read_point(&entity.nbt, "resultPedestal") else {
        return;
    };
    let facing = state_prop(host_ref, state, "facing").unwrap_or_else(|| "up".to_owned());
    let normal = match facing.as_str() {
        "down" => [0.0, -1.0, 0.0],
        "north" => [0.0, 0.0, -1.0],
        "south" => [0.0, 0.0, 1.0],
        "west" => [-1.0, 0.0, 0.0],
        "east" => [1.0, 0.0, 0.0],
        _ => [0.0, 1.0, 0.0],
    };
    let emitter = std::array::from_fn(|axis| f64::from(pos[axis]) + 0.5 - normal[axis] * 0.5);
    let target_from =
        std::array::from_fn(|axis| f64::from(target[axis]) + 0.5 + if axis == 1 { 0.5 } else { 0.0 });
    let phase = (draw.game_time().rem_euclid(40) as f32 + draw.partial()) / 40.0;
    let old_light = draw.light();
    draw.set_light(0xF0F0);
    draw_reformation_trail(
        draw,
        pos,
        emitter,
        target_from,
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 1.0],
        phase,
    );
    for source in sources {
        let source_from = std::array::from_fn(|axis| {
            f64::from(source[axis]) + 0.5 + if axis == 1 { 0.7 } else { 0.0 }
        });
        draw_reformation_trail(
            draw,
            pos,
            target_from,
            source_from,
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            phase,
        );
        let result_to = std::array::from_fn(|axis| {
            f64::from(result[axis]) + 0.5 + if axis == 1 { 0.7 } else { 0.0 }
        });
        draw_reformation_trail(
            draw,
            pos,
            source_from,
            result_to,
            [2.0 / 3.0, 8.0 / 255.0, 2.0 / 3.0],
            [0.0, 1.0, 0.0],
            phase,
        );
    }
    draw.set_light(old_light);
}

fn draw_held_fit(
    host_ref: &dyn ClientHost,
    block: &str,
    draw: &mut dyn BlockEntityDrawContext,
) {
    let settings = config::get();
    if !settings.enable_held_stack_fit_outline {
        return;
    }
    let Some(player) = host_ref.local_player() else {
        return;
    };
    let Some(stack) = host_ref
        .living(player.entity())
        .and_then(|living| living.hand_item(host_ref, mod_api::Hand::Main))
    else {
        return;
    };
    let accepted = interactions::accepts_input(host_ref, block, &stack);
    if !accepted && settings.held_stack_fit_outline_render_mode == "GREEN_ONLY" {
        return;
    }
    let texture = if accepted {
        "block/lime_concrete"
    } else {
        "block/red_concrete"
    };
    let face = Some(CubeFace::new(texture));
    let w = 1.0 / 32.0;
    for (from, to) in [
        ([0.0, 0.0, 0.0], [w, 1.0, w]),
        ([1.0 - w, 0.0, 0.0], [1.0, 1.0, w]),
        ([0.0, 0.0, 1.0 - w], [w, 1.0, 1.0]),
        ([1.0 - w, 0.0, 1.0 - w], [1.0, 1.0, 1.0]),
        ([0.0, 0.0, 0.0], [1.0, w, w]),
        ([0.0, 1.0 - w, 0.0], [1.0, 1.0, w]),
        ([0.0, 0.0, 1.0 - w], [1.0, w, 1.0]),
        ([0.0, 1.0 - w, 1.0 - w], [1.0, 1.0, 1.0]),
        ([0.0, 0.0, 0.0], [w, w, 1.0]),
        ([1.0 - w, 0.0, 0.0], [1.0, w, 1.0]),
        ([0.0, 1.0 - w, 0.0], [w, 1.0, 1.0]),
        ([1.0 - w, 1.0 - w, 0.0], [1.0, 1.0, 1.0]),
    ] {
        let _ = draw.cube(from.map(|v| v * 16.0), to.map(|v| v * 16.0), [face; 6], 0xFF_FFFF);
    }
}

pub fn submit(host_ref: &dyn ClientHost, event: &mut BlockEntitySubmitEvent<'_>) {
    let pos = event.pos;
    let block = event.block;
    let state = event.state;
    let entity = event.entity;
    let draw = &mut *event.draw;

    if draw.light() == 0 {
        draw.set_light(0xF0F0);
    }
    draw_wire(pos, entity, draw);
    draw_held_fit(host_ref, block, draw);
    draw_reformation(host_ref, pos, state, entity, draw);

    if block == "theurgy:digestion_vat" {
        draw_digestion_vat(host_ref, state, entity, draw);
        return;
    }

    if block == "theurgy:liquefaction_cauldron" {
        let color = cauldron_color(&entity.nbt, 0xFFFF_FFFF);
        let sprite = cauldron_sprite(&entity.nbt);
        let cs = extract_cauldron(&entity.nbt, draw.light() as i32, 0, color, sprite);
        submit_fluid_quads(draw, "theurgy:block/sal_ammoniac/still", &cs, FLUID_INSET);
    }

    // The distiller is a two-block apparatus: its upper half carries the same block
    // entity, but the GeckoLib model already spans both blocks, so only the lower half
    // draws it. Drawing it on both halves produced the "two copies" with the top sliding
    // into the lower copy.
    let draws_model = !(block == "theurgy:distiller"
        && state_prop(host_ref, state, "half").as_deref() == Some("upper"));
    if let Some(name) = model_name(block)
        && draws_model
        && let Some(model) = models().get(name)
        && let Some(geometry) = model.geometry.first()
    {
        let texture = model_texture(block);
        let width = geometry
            .description
            .as_ref()
            .and_then(|d| d.texture_width)
            .unwrap_or(128.0)
            .max(1.0);
        let height = geometry
            .description
            .as_ref()
            .and_then(|d| d.texture_height)
            .unwrap_or(128.0)
            .max(1.0);
        let processing = entity.data.get(4).copied().unwrap_or(0) != 0
            || host::bool(&entity.nbt, "isProcessing");
        let now = draw.game_time() as f32 / 20.0 + draw.partial() / 20.0;
        let (animation_name_id, seconds) =
            animation_id_at(block, pos, processing, now).unwrap_or(("", now));
        let animation = animation_name(block)
            .and_then(|n| animations().get(n))
            .and_then(|file| file.animations.get(animation_name_id));
        let mut bones = Vec::new();
        flatten_bones(&geometry.bones, None, &mut bones);
        draw.push();
        draw.translate([0.5, 0.0, 0.5]);
        let facing = ["facing", "horizontal_facing", "vertical_direction"]
            .iter()
            .find_map(|key| state_prop(host_ref, state, key));
        let (axis, degrees) = geo_facing_rotation(facing.as_deref().unwrap_or("north"));
        if degrees != 0.0 {
            draw.rotate_about(axis, degrees, [0.0; 3]);
        }
        let hidden: Vec<&str> = if block == "theurgy:logistics_nexus" {
            [
                ("Top", "up"),
                ("Bottom", "down"),
                ("North", "north"),
                ("South", "south"),
                ("East", "east"),
                ("West", "west"),
            ]
            .into_iter()
            .filter(|(_, key)| state_prop(host_ref, state, key).as_deref() == Some("false"))
            .map(|(bone, _)| bone)
            .collect()
        } else {
            Vec::new()
        };
        for flat in bones.iter().filter(|bone| bone.parent.is_none()) {
            render_bone(
                draw,
                flat.bone,
                &bones,
                &hidden,
                animation,
                seconds,
                texture,
                width,
                height,
            );
        }
        draw.pop();
        if animation.is_none()
            && !matches!(
                block,
                "theurgy:sal_ammoniac_tank" | "theurgy:sal_ammoniac_accumulator"
            )
        {
            draw.keep();
        }
    }

    if matches!(
        block,
        "theurgy:sal_ammoniac_tank" | "theurgy:sal_ammoniac_accumulator"
    ) {
        let tank_block = block == "theurgy:sal_ammoniac_tank";
        let capacity = if tank_block { 2_000 } else { 10_000 };
        let tank = fluid_tank(entity, capacity, if tank_block { "tank" } else { "waterTank" });
        let fill = (tank.amount as f32 / tank.capacity.max(1) as f32).clamp(0.0, 1.0);
        let (block_height, cap_height, inset) = if tank_block {
            (15.0 / 16.0 - 2.0 / 128.0, 0.25, 2.0 / 16.0 + 2.0 / 128.0)
        } else {
            (12.0 / 16.0 - 2.0 / 128.0, 0.0, 3.0 / 16.0 + 2.0 / 128.0)
        };
        let total_height = block_height - 2.0 * cap_height - 1.0 / 16.0;
        let bottom = cap_height;
        let surface = bottom + 1.0 / 16.0 + fill * total_height;
        let mut bs = box_fluid_state(&tank, draw.light() as i32, bottom, surface);
        if tank.fluid.as_deref() == Some("minecraft:water") {
            bs.color = 0xFF3F_76E4;
        }
        let sprite = if tank.fluid.as_deref() == Some("minecraft:water") {
            "block/water_still"
        } else {
            "theurgy:block/sal_ammoniac/still"
        };
        submit_fluid_quads(draw, sprite, &bs, inset);
    }

    if matches!(
        block,
        "theurgy:incubator_mercury_vessel"
            | "theurgy:incubator_salt_vessel"
            | "theurgy:incubator_sulfur_vessel"
            | "theurgy:reformation_source_pedestal"
            | "theurgy:reformation_target_pedestal"
            | "theurgy:reformation_result_pedestal"
    ) && let Some(stack) = client_stack(host_ref, entity)
    {
        draw.push();
        draw.translate([0.5, 1.25, 0.5]);
        let _ = draw.item(&stack, ItemDisplayContext::Fixed, None, 0);
        draw.pop();
    }
}

pub fn bounds(event: &mut BlockEntityRenderBoundingBoxEvent<'_>) {
    if matches!(
        event.entity.nbt.get(&"wireConnections".into()),
        Some(Tag::List(records)) if !records.is_empty()
    ) {
        *event.bounds = [-32.0, -32.0, -32.0, 33.0, 33.0, 33.0];
        return;
    }
    if event.block == "theurgy:sulfuric_flux_emitter" {
        *event.bounds = [-16.0, -1.0, -16.0, 16.0, 4.0, 16.0];
        return;
    }
    if let Some(model) = model_name(event.block).and_then(|name| models().get(name))
        && let Some(description) = model.geometry.first().and_then(|g| g.description.as_ref())
    {
        let width = description.visible_bounds_width.unwrap_or(2.0);
        let height = description.visible_bounds_height.unwrap_or(2.5);
        let offset = description.visible_bounds_offset.unwrap_or([0.0, 0.75, 0.0]);
        *event.bounds = [
            -width / 2.0,
            offset[1] - 1.0,
            -width / 2.0,
            width / 2.0,
            offset[1] + height + 1.0,
            width / 2.0,
        ];
        return;
    }
    if event.block == "theurgy:liquefaction_cauldron" {
        *event.bounds = [-1.0, -1.0, -1.0, 2.0, 2.0, 2.0];
    }
    if event.block == "theurgy:digestion_vat" {
        *event.bounds = [-0.5, -0.5, -0.5, 1.5, 2.0, 1.5];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nbt_with(fluid: Option<(&str, i32)>, color: Option<u32>) -> Compound {
        let mut nbt = Compound::new();
        if let Some((id, amount)) = fluid {
            let mut tank = Compound::new();
            tank.insert("Fluid".into(), Tag::String(id.into()));
            tank.insert("Amount".into(), Tag::Int(amount));
            nbt.insert("fluids".into(), Tag::List(vec![Tag::Compound(tank)]));
        }
        if let Some(c) = color {
            nbt.insert("fluidColor".into(), Tag::Int(c as i32));
        }
        nbt
    }

    fn cube(origin: [f32; 3], size: [f32; 3], uv: GeoUv) -> GeoCubeJson {
        GeoCubeJson {
            origin,
            size,
            uv,
            inflate: None,
            mirror: None,
            rotation: [0.0; 3],
            pivot: [0.0; 3],
        }
    }

    fn face<'a>(faces: &'a [GeoFace], direction: &str) -> &'a GeoFace {
        faces.iter().find(|f| f.direction == direction).expect(direction)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn cauldron_quads_and_light_match_java() {
        assert!(extract_cauldron(&nbt_with(None, None), 0xF0, 0, 0xFFFF_FFFF, SpriteBounds::FULL).empty);
        assert!(extract_cauldron(&nbt_with(Some(("theurgy:sal_ammoniac", 0)), None), 0xF0, 0, 0xFFFF_FFFF, SpriteBounds::FULL).empty);

        let nbt = nbt_with(Some(("theurgy:sal_ammoniac", CAULDRON_CAPACITY / 2)), None);
        let s = extract_cauldron(&nbt, 0xF0, 0, 0xFFFF_FFFF, SpriteBounds::FULL);
        assert!(!s.empty);
        assert!(close(s.bottom_y, FLUID_BOTTOM_Y));
        assert!((s.bottom_y + FLUID_HEIGHT * 0.5 - s.surface_y).abs() < 1e-4);
        assert_eq!(cauldron_quads(&s).len(), 5);

        let over = nbt_with(Some(("theurgy:sal_ammoniac", CAULDRON_CAPACITY + 500)), Some(0xAABBCCDD));
        assert_eq!(cauldron_color(&over, 0), 0xAABBCCDD);
        let s2 = extract_cauldron(&over, 0x500, 15, 0xFFFF_FFFF, SpriteBounds::FULL);
        assert_eq!((s2.fluid_light >> 4) & 0xF, 15);
    }

    #[test]
    fn geo_cube_mirrors_x_winds_outward_and_matches_box_uv() {
        let faces = geo_cube_faces(
            &cube([1.0, 2.0, -3.0], [2.0, 4.0, 6.0], GeoUv::Array([0.0, 0.0])),
            None,
            None,
            64.0,
            64.0,
        );
        assert_eq!(faces.len(), 6);
        assert!(face(&faces, "west").corners.iter().all(|c| close(c[0], -3.0 / 16.0)));
        assert!(face(&faces, "east").corners.iter().all(|c| close(c[0], -1.0 / 16.0)));

        let faces = geo_cube_faces(
            &cube([-3.0, 15.1, -3.0], [6.0, 2.0, 6.0], GeoUv::Array([73.0, 0.0])),
            None,
            None,
            128.0,
            128.0,
        );
        let texel = |v: f32| v / 128.0 * 16.0;
        assert_eq!(
            face(&faces, "north").uv,
            [texel(79.0), texel(6.0), texel(85.0), texel(8.0)]
        );
        assert_eq!(
            face(&faces, "down").uv,
            [texel(85.0), texel(6.0), texel(91.0), texel(0.0)]
        );
    }

    #[test]
    fn real_models_and_animations_parse_with_sane_uvs() {
        assert!(models().len() >= 6);
        assert!(!animations().is_empty());
        for (name, file) in models() {
            let geometry = &file.geometry[0];
            let desc = geometry.description.as_ref().unwrap();
            let (tw, th) = (desc.texture_width.unwrap(), desc.texture_height.unwrap());
            let mut bones = Vec::new();
            flatten_bones(&geometry.bones, None, &mut bones);
            let mut count = 0;
            for flat in &bones {
                for c in &flat.bone.cubes {
                    for f in geo_cube_faces(c, flat.bone.inflate, flat.bone.mirror, tw, th) {
                        count += 1;
                        for v in f.uv {
                            assert!(v.is_finite() && (-1.0..=17.0).contains(&v), "{name}: {v}");
                        }
                    }
                }
            }
            assert!(count > 0, "{name} baked nothing");
        }
    }

    #[test]
    fn digestion_vat_vanilla_parts_and_easing() {
        assert_eq!(vanilla_cube_faces(&DIGESTION_SIDE[0], [16.0, 16.0]).len(), 1);
        assert_eq!(vanilla_cube_faces(&DIGESTION_PLANE[0], [32.0, 32.0]).len(), 2);
        assert_eq!(vanilla_cube_faces(&DIGESTION_NECK[1], [32.0, 32.0]).len(), 6);
        assert!(close(ease("easeInOutCubic", 0.25, &[]), 4.0 * 0.25f32.powi(3)));
        assert_eq!(cauldron_bounding_box(), ([0.0; 3], [1.0; 3]));
        assert!(carries_model("theurgy:distiller"));
    }
}
