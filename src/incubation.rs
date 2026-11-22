// SPDX-License-Identifier: MIT
//! Incubator and its three adjacent vessels (mercury, salt, sulfur).

use mod_api::serde_json::{self, Value};
use mod_api::{
    Compound, Host, HostTransaction, Identifier, ItemStack, LevelRef, ModBlockEntityView,
    ServerHost, Tag,
};

use crate::engine::crafting::Crafting;
use crate::{heat, host, recipes};

pub const DEFAULT_TIME: i32 = 100;

const INCUBATOR: &str = "theurgy:incubator";
const MERCURY_VESSEL: &str = "theurgy:incubator_mercury_vessel";
const SALT_VESSEL: &str = "theurgy:incubator_salt_vessel";
const SULFUR_VESSEL: &str = "theurgy:incubator_sulfur_vessel";
const RECIPE_KIND: &str = "theurgy:incubation";
const MAX_STACK: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vessel {
    Mercury,
    Salt,
    Sulfur,
}

impl Vessel {
    fn of_id(id: &str) -> Option<Self> {
        match id {
            MERCURY_VESSEL => Some(Vessel::Mercury),
            SALT_VESSEL => Some(Vessel::Salt),
            SULFUR_VESSEL => Some(Vessel::Sulfur),
            _ => None,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Vessel::Mercury => "theurgy:alchemical_mercuries",
            Vessel::Salt => "theurgy:alchemical_salts",
            Vessel::Sulfur => "theurgy:alchemical_sulfurs",
        }
    }
}

/// Java's `Direction.Plane.HORIZONTAL` iteration order (north, east, south, west).
const HORIZONTAL: [[i32; 3]; 4] = [[0, 0, -1], [1, 0, 0], [0, 0, 1], [-1, 0, 0]];

fn offer(target: &mut Option<[i32; 3]>, at: [i32; 3]) {
    if target.is_none() {
        *target = Some(at);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vessels {
    pub mercury: Option<[i32; 3]>,
    pub salt: Option<[i32; 3]>,
    pub sulfur: Option<[i32; 3]>,
}

impl Vessels {
    pub fn is_valid(&self) -> bool {
        self.mercury.is_some() && self.salt.is_some() && self.sulfur.is_some()
    }
}

pub fn classify_neighbours<'a, I>(neighbours: I) -> Vessels
where
    I: IntoIterator<Item = (&'a str, [i32; 3])>,
{
    let mut out = Vessels::default();
    for (id, at) in neighbours {
        match Vessel::of_id(id) {
            Some(Vessel::Mercury) => offer(&mut out.mercury, at),
            Some(Vessel::Salt) => offer(&mut out.salt, at),
            Some(Vessel::Sulfur) => offer(&mut out.sulfur, at),
            None => {}
        }
    }
    out
}

pub fn horizontal_neighbours(pos: [i32; 3]) -> [[i32; 3]; 4] {
    HORIZONTAL.map(|[x, y, z]| [pos[0] + x, pos[1] + y, pos[2] + z])
}

#[cfg(test)]
fn read_network(nbt: &Compound) -> Crafting {
    Crafting {
        progress: host::int(nbt, "progress", 0),
        total_time: host::int(nbt, "totalTime", DEFAULT_TIME),
        is_processing: host::bool(nbt, "isProcessing"),
        could_craft_last_tick: host::bool(nbt, "couldCraftLastTick"),
    }
}

#[cfg(test)]
fn write_network(nbt: &mut Compound, state: &Crafting) {
    host::set_int(nbt, "progress", state.progress);
    host::set_int(nbt, "totalTime", state.total_time);
    host::set_int(nbt, "isProcessing", i32::from(state.is_processing));
    host::set_int(
        nbt,
        "couldCraftLastTick",
        i32::from(state.could_craft_last_tick),
    );
}

fn total_time_or_default(state: &Crafting) -> i32 {
    if state.total_time > 0 {
        state.total_time
    } else {
        DEFAULT_TIME
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeShape {
    pub mercury: Value,
    pub salt: Value,
    pub sulfur: Value,
    pub result: Value,
    pub time: i32,
}

impl RecipeShape {
    pub fn parse(json: &Value) -> Option<Self> {
        Some(Self {
            mercury: json.get("mercury")?.clone(),
            salt: json.get("salt")?.clone(),
            sulfur: json.get("sulfur")?.clone(),
            result: json.get("result")?.clone(),
            time: json
                .get("time")
                .and_then(Value::as_i64)
                .map_or(DEFAULT_TIME, |t| {
                    t.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
                }),
        })
    }
}

pub fn tick(
    host_ref: &mut dyn ServerHost,
    entity_id: &str,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    if entity_id == INCUBATOR {
        tick_incubator(host_ref, pos, entity);
    }
}

fn update_connections(host_ref: &mut dyn ServerHost, pos: [i32; 3], vessels: &Vessels) {
    for (property, connected) in [
        ("north", vessels.mercury.is_some()),
        ("east", vessels.salt.is_some()),
        ("south", vessels.sulfur.is_some()),
        ("west", false),
    ] {
        host_ref.set_block_property_with(pos, property, if connected { "true" } else { "false" }, 3);
    }
}

fn vessel_at(host_ref: &dyn Host, level: LevelRef, at: [i32; 3]) -> Option<String> {
    let be = host_ref.block_entity(level, at)?;
    let id = be.kind(host_ref)?.id(host_ref);
    Vessel::of_id(&id).map(|_| id)
}

fn vessel_input(host_ref: &dyn Host, level: LevelRef, at: [i32; 3]) -> Option<ItemStack> {
    let be = host_ref.block_entity(level, at)?;
    be.item_slot(host_ref, 0)
        .ok()
        .flatten()
        .filter(|s| s.count() > 0)
}

/// `TagRecipeResult.getStack`: resolve first registered item in tag order, or a
/// labelled barrier when empty.
fn recipe_result(host_ref: &dyn Host, json: &Value) -> Option<ItemStack> {
    if json.get("type").and_then(Value::as_str) != Some("theurgy:tag") {
        return recipes::parse_result(host_ref, json);
    }
    let tag_name = json.get("tag").and_then(Value::as_str)?;
    let tag_clean = tag_name.strip_prefix('#').unwrap_or(tag_name);
    if let Ok(tag_id) = Identifier::parse(tag_clean)
        && let Some(tag_key) = host_ref.tags().key(&tag_id)
        && let Some(members) = host_ref.tags().items(tag_key)
        && let Some(&first_item) = members.first()
    {
        let mut resolved = json.clone();
        resolved["id"] = Value::String(first_item.id(host_ref));
        return recipes::parse_result(host_ref, &resolved);
    }

    let count = json.get("count").and_then(Value::as_u64).unwrap_or(1) as u32;
    let barrier = serde_json::json!({"id": "minecraft:barrier", "count": count.max(1)});
    let mut result = recipes::parse_result(host_ref, &barrier)?;
    let empty_name = format!("Empty Tag: {tag_name}");
    let mut patch = Compound::new();
    patch.insert(
        "minecraft:custom_name".into(),
        Tag::String(empty_name.as_str().into()),
    );
    let _ = result.apply_components(&patch);
    Some(result)
}

fn output_can_fit(existing: Option<&ItemStack>, result: &ItemStack, max_stack: u32) -> bool {
    if result.count() == 0 {
        return false;
    }
    match existing {
        None => result.count() <= max_stack,
        Some(old) if old.count() == 0 => result.count() <= max_stack,
        Some(old) => {
            old.is_stackable_with(result)
                && old.count().saturating_add(result.count()) <= max_stack
        }
    }
}

fn perform_craft(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    vessels: [[i32; 3]; 3],
    incubator: &mut ModBlockEntityView<'_>,
    result: &ItemStack,
) -> bool {
    let max_stack = host::item_max_stack(&*host_ref, result).min(MAX_STACK);
    if !output_can_fit(
        incubator.items.first().and_then(Option::as_ref),
        result,
        max_stack,
    ) {
        return false;
    }

    // Atomically decrement one item from each vessel in a single host transaction.
    let consumed = host_ref.transaction(|host_ref, tx| {
        for at in vessels {
            let be = host_ref.block_entity(level, at).ok_or(())?;
            let Some(mut stack) = be.item_slot(&*host_ref, 0).ok().flatten() else {
                return Err(());
            };
            if stack.count() < 1 {
                return Err(());
            }
            let next = if stack.count() > 1 {
                let left = stack.count() - 1;
                stack.set_count(left).map_err(|_| ())?;
                Some(stack)
            } else {
                None
            };
            be.set_item_slot(host_ref, tx, 0, next).map_err(|_| ())?;
        }
        Ok::<(), ()>(())
    });
    if consumed.is_err() {
        return false;
    }
    for at in vessels {
        host_ref.block_entity_sync(at);
    }

    if incubator.items.is_empty() {
        incubator.items.push(Some(result.clone()));
    } else if let Some(slot) = incubator.items[0].as_mut() {
        if slot.count() == 0 {
            *slot = result.clone();
        } else if slot.is_stackable_with(result) {
            let total = slot.count().saturating_add(result.count());
            let _ = slot.set_count(total);
        } else {
            return false;
        }
    } else {
        incubator.items[0] = Some(result.clone());
    }
    true
}

/// Compact non-zero hash of the three vessel stacks `(item_id, count)` stored in
/// `entity.data[9]` so external vessel edits (including `/data merge` or player
/// swaps) trigger `input_changed` just like Java's `onContentsChanged`, while
/// `0` means uninitialized after chunk/world load (never resets saved progress).
fn vessel_signature(
    host_ref: &dyn Host,
    m: &ItemStack,
    sa: &ItemStack,
    su: &ItemStack,
) -> i32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for stack in [m, sa, su] {
        for b in stack.item().id(host_ref).as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h ^= u64::from(stack.count());
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let folded = (h ^ (h >> 32)) as i32;
    if folded == 0 { 1 } else { folded }
}

fn tick_incubator(
    host_ref: &mut dyn ServerHost,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    let Some(level) = host::level_in_place(host_ref) else {
        return;
    };
    if entity.items.is_empty() {
        entity.items.push(None);
    }
    let mut neighbours: Vec<(String, [i32; 3])> = Vec::new();
    for at in horizontal_neighbours(pos) {
        if let Some(id) = vessel_at(&*host_ref, level, at) {
            neighbours.push((id, at));
        }
    }
    let vessels = classify_neighbours(neighbours.iter().map(|(id, at)| (id.as_str(), *at)));
    update_connections(host_ref, pos, &vessels);

    let mut state = host::crafting(&entity.nbt, &entity.data);
    let (Some(mercury), Some(salt), Some(sulfur)) = (vessels.mercury, vessels.salt, vessels.sulfur)
    else {
        if state.stop() > 0 {
            let fields = &mut **entity;
            host::save_crafting(&mut fields.nbt, &mut fields.data, &state);
            host_ref.block_entity_sync(pos);
        }
        return;
    };

    let mercury_stack = vessel_input(&*host_ref, level, mercury);
    let salt_stack = vessel_input(&*host_ref, level, salt);
    let sulfur_stack = vessel_input(&*host_ref, level, sulfur);

    let has_input = mercury_stack.is_some() && salt_stack.is_some() && sulfur_stack.is_some();
    let is_heated = heat::heated(host_ref, level, pos, entity);

    if !has_input {
        if entity.items.first().is_some_and(Option::is_some) {
            return;
        }
        if state.stop() > 0 {
            let fields = &mut **entity;
            host::save_crafting(&mut fields.nbt, &mut fields.data, &state);
            host_ref.block_entity_sync(pos);
        }
        return;
    }

    let (m_stack, sa_stack, su_stack) = (
        mercury_stack.as_ref().unwrap(),
        salt_stack.as_ref().unwrap(),
        sulfur_stack.as_ref().unwrap(),
    );

    let list = recipes::of_type(RECIPE_KIND);
    let mut matched: Option<RecipeShape> = None;
    for rec in list.iter() {
        let Some(shape) = RecipeShape::parse(&rec.json) else {
            continue;
        };
        if recipes::matches_value(&*host_ref, &shape.mercury, m_stack)
            && recipes::matches_value(&*host_ref, &shape.salt, sa_stack)
            && recipes::matches_value(&*host_ref, &shape.sulfur, su_stack)
        {
            matched = Some(shape);
            break;
        }
    }

    let Some(shape) = matched else {
        if state.stop() > 0 {
            let fields = &mut **entity;
            host::save_crafting(&mut fields.nbt, &mut fields.data, &state);
            host_ref.block_entity_sync(pos);
        }
        return;
    };

    let result = recipe_result(&*host_ref, &shape.result);
    let can_craft = result.as_ref().is_some_and(|out| {
        let max_stack = host::item_max_stack(&*host_ref, out).min(MAX_STACK);
        output_can_fit(entity.items.first().and_then(Option::as_ref), out, max_stack)
    });

    let time = shape.time.max(0);
    let sig = vessel_signature(&*host_ref, m_stack, sa_stack, su_stack);
    let prev_sig = entity.data.get(9).copied().unwrap_or(0);
    if state.total_time != time || (prev_sig != 0 && prev_sig != sig) {
        state.input_changed(time);
    }
    let effects = state.tick(is_heated, true, can_craft, time);

    let mut crafted = false;
    let mut post_sig = sig;
    if effects.craft_attempt
        && let Some(out) = result
        && perform_craft(host_ref, level, [mercury, salt, sulfur], entity, &out)
    {
        crafted = true;
        state.progress = 0;
        state.total_time = total_time_or_default(&state);
        post_sig = match (
            vessel_input(&*host_ref, level, mercury),
            vessel_input(&*host_ref, level, salt),
            vessel_input(&*host_ref, level, sulfur),
        ) {
            (Some(nm), Some(nsa), Some(nsu)) => vessel_signature(&*host_ref, &nm, &nsa, &nsu),
            _ => 0,
        };
    }
    let fields = &mut **entity;
    host::save_crafting(&mut fields.nbt, &mut fields.data, &state);
    if fields.data.len() <= 9 {
        fields.data.resize(10, 0);
    }
    fields.data[9] = post_sig;
    if effects.block_updates > 0 || crafted {
        host_ref.block_entity_sync(pos);
    }
}

/// Pure fit check used by unit tests without needing a live `ItemKey` registry.
#[cfg(test)]
fn slot_fits(
    existing: Option<(&str, u32)>,
    incoming: (&str, u32),
    max_stack: u32,
) -> bool {
    if incoming.1 == 0 {
        return false;
    }
    match existing {
        None | Some((_, 0)) => incoming.1 <= max_stack,
        Some((id, count)) => id == incoming.0 && count.saturating_add(incoming.1) <= max_stack,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::serde_json::json;

    #[test]
    fn horizontal_neighbours_match_java_order() {
        let got = horizontal_neighbours([10, 5, -3]);
        assert_eq!(got, [[10, 5, -4], [11, 5, -3], [10, 5, -2], [9, 5, -3]]);
    }

    #[test]
    fn classify_needs_all_three_kinds() {
        let only = classify_neighbours([(MERCURY_VESSEL, [1, 0, 0]), (SALT_VESSEL, [0, 0, 1])]);
        assert!(!only.is_valid());
        assert_eq!(only.mercury, Some([1, 0, 0]));
        assert_eq!(only.salt, Some([0, 0, 1]));
        assert_eq!(only.sulfur, None);

        let all = classify_neighbours([
            (SULFUR_VESSEL, [0, 0, -1]),
            (MERCURY_VESSEL, [1, 0, 0]),
            (SALT_VESSEL, [0, 0, 1]),
        ]);
        assert!(all.is_valid());
        assert_eq!(all.mercury, Some([1, 0, 0]));
        assert_eq!(all.salt, Some([0, 0, 1]));
        assert_eq!(all.sulfur, Some([0, 0, -1]));
    }

    #[test]
    fn classify_ignores_unknown_blocks_and_keeps_first() {
        let only = classify_neighbours([
            ("theurgy:calcination_oven", [1, 0, 0]),
            (MERCURY_VESSEL, [0, 0, 1]),
        ]);
        assert_eq!(only.mercury, Some([0, 0, 1]));
        assert!(only.salt.is_none());

        let v = classify_neighbours([(SALT_VESSEL, [1, 0, 0]), (SALT_VESSEL, [0, 0, 1])]);
        assert_eq!(v.salt, Some([1, 0, 0]));
    }

    #[test]
    fn recipe_shape_parses_defaults_and_explicit_time() {
        let recipe = json!({
            "mercury": "theurgy:alchemical_mercury",
            "salt": "#theurgy:alchemical_salts",
            "sulfur": "theurgy:alchemical_sulfur",
            "result": {"id": "minecraft:gold_nugget"}
        });
        assert_eq!(RecipeShape::parse(&recipe).unwrap().time, DEFAULT_TIME);

        let bad = json!({
            "mercury": "theurgy:alchemical_mercury",
            "salt": "theurgy:alchemical_salt",
            "result": {"id": "minecraft:gold_nugget"}
        });
        assert!(RecipeShape::parse(&bad).is_none());

        let timed = json!({
            "mercury": "a", "salt": "b", "sulfur": "c",
            "result": {"id": "minecraft:dirt"}, "time": 40
        });
        assert_eq!(RecipeShape::parse(&timed).unwrap().time, 40);
    }

    #[test]
    fn default_total_time_and_slot_fits() {
        assert_eq!(total_time_or_default(&Crafting::default()), DEFAULT_TIME);
        assert_eq!(
            total_time_or_default(&Crafting {
                total_time: 30,
                ..Default::default()
            }),
            30
        );
        assert!(slot_fits(None, ("theurgy:result", 1), 64));
        assert!(!slot_fits(Some(("minecraft:stone", 64)), ("minecraft:stone", 1), 64));
        assert!(!slot_fits(Some(("minecraft:stone", 1)), ("minecraft:dirt", 1), 64));
        assert!(slot_fits(Some(("minecraft:stone", 63)), ("minecraft:stone", 1), 64));
        assert!(slot_fits(Some(("minecraft:stone", 0)), ("minecraft:dirt", 1), 64));
    }

    #[test]
    fn state_round_trips_through_compound() {
        let state = Crafting {
            progress: 7,
            total_time: 42,
            is_processing: true,
            could_craft_last_tick: false,
        };
        let mut nbt = Compound::new();
        write_network(&mut nbt, &state);
        assert_eq!(read_network(&nbt), state);
    }
}
