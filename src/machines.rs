// SPDX-License-Identifier: MIT
//! The five core processing machines: calcination oven, distiller, liquefaction
//! cauldron, fermentation vat, and digestion vat.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use mod_api::serde_json::Value;
use mod_api::{
    AnalogOutputEvent, BlockSignalEvent, Compound, HasAnalogOutputEvent, Host,
    IsSignalSourceEvent, ItemStack, LevelHost, LevelRef, ModBlockEntityView, NeighbourUpdate,
    ParticleOptions, ServerHost, Tag,
};

use crate::engine::machines::{CraftPlan, MachineLayout, ProcessResult, commit_plan, layout};
use crate::engine::recipes::{Item, Recipe, Tank, fluid_part, ingredient_parts};
use crate::{heat, host, recipes, registry};

/// Machine kinds shared with `engine::machines::layout`.
const MACHINE_KINDS: [(&str, &str); 5] = [
    ("theurgy:calcination_oven", "theurgy:calcination"),
    ("theurgy:distiller", "theurgy:distillation"),
    ("theurgy:liquefaction_cauldron", "theurgy:liquefaction"),
    ("theurgy:fermentation_vat", "theurgy:fermentation"),
    ("theurgy:digestion_vat", "theurgy:digestion"),
];

const MACHINE_TANK_CAPACITY: i32 = 10_000;
const FALLBACK_MAX_STACK: u32 = 64;

/// Cached vat output signal state so `block_signal` and `analog_output` answer
/// immediately from memory without touching the world during redstone updates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct VatSignal {
    has_output: bool,
    analog: i32,
}

static VAT_SIGNALS: OnceLock<RwLock<HashMap<(String, [i32; 3]), VatSignal>>> = OnceLock::new();

fn vat_signals() -> &'static RwLock<HashMap<(String, [i32; 3]), VatSignal>> {
    VAT_SIGNALS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn clear_signals() {
    if let Ok(mut guard) = vat_signals().write() {
        guard.clear();
    }
}

pub fn forget_vat_signal(dim: &str, pos: [i32; 3]) {
    if let Ok(mut guard) = vat_signals().write() {
        guard.remove(&(dim.to_owned(), pos));
    }
}

pub fn update_vat_signal(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    output: Option<&ItemStack>,
) {
    let signal = compute_vat_signal(&*host_ref, output);
    let dim = host_ref.dimension().to_owned();
    let changed = if let Ok(mut guard) = vat_signals().write() {
        let prev = guard.insert((dim, pos), signal).unwrap_or_default();
        prev != signal
    } else {
        false
    };
    if changed && let Some(state) = level.block_state(&*host_ref, pos) {
        let _ = level.update_neighbors(host_ref, pos, state, NeighbourUpdate::Neighbours);
        let _ = level.update_neighbors(host_ref, pos, state, NeighbourUpdate::OutputSignal);
    }
}

fn compute_vat_signal(host_ref: &dyn Host, output: Option<&ItemStack>) -> VatSignal {
    match output {
        Some(stack) if stack.count() > 0 => {
            let max = host::item_max_stack(host_ref, stack);
            let analog = 1 + ((u64::from(stack.count()) * 14) / u64::from(max)) as i32;
            VatSignal {
                has_output: true,
                analog: analog.clamp(0, 15),
            }
        }
        _ => VatSignal::default(),
    }
}

pub fn kind_for_block(block: &str) -> Option<&'static str> {
    MACHINE_KINDS
        .iter()
        .find(|(id, _)| *id == block)
        .map(|(_, kind)| *kind)
}

fn slot_count(kind: &str) -> usize {
    match kind {
        "theurgy:calcination" | "theurgy:distillation" | "theurgy:liquefaction" => 2,
        "theurgy:fermentation" | "theurgy:digestion" => 4,
        _ => 0,
    }
}

fn is_heat_consumer(kind: &str) -> bool {
    matches!(
        kind,
        "theurgy:calcination" | "theurgy:distillation" | "theurgy:liquefaction"
    )
}

fn read_slot(items: &[Option<ItemStack>], index: usize) -> Option<&ItemStack> {
    items.get(index).and_then(Option::as_ref)
}

fn read_items(
    host_ref: &dyn Host,
    items: &[Option<ItemStack>],
    count: usize,
) -> Vec<Option<Item>> {
    (0..count)
        .map(|i| {
            read_slot(items, i).map(|stack| Item {
                id: stack.item().id(host_ref),
                count: stack.count(),
                max_stack: host::item_max_stack(host_ref, stack),
            })
        })
        .collect()
}

fn input_signature(
    host_ref: &dyn Host,
    items: &[Option<ItemStack>],
    kind: &str,
    recipe_id: &str,
) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in kind.bytes().chain(recipe_id.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let inputs = layout(kind).map(|value| value.inputs).unwrap_or(&[]);
    for slot in inputs
        .iter()
        .map(|index| items.get(*index).and_then(Option::as_ref))
    {
        if let Some(stack) = slot {
            let id = stack.item().id(host_ref);
            let desc = format!("{id}|{}|{:?}", stack.damage(), stack.components());
            for byte in desc.bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn apply_plan_to_items(
    host_ref: &dyn Host,
    entity_items: &mut [Option<ItemStack>],
    plan: &CraftPlan,
    assembled: Option<&ItemStack>,
) -> bool {
    if let Some((slot, result)) = &plan.output {
        let fits = match entity_items.get(*slot).and_then(Option::as_ref) {
            None => result.count <= result.max_stack,
            Some(old) => {
                old.item().id(host_ref) == result.id
                    && assembled.is_none_or(|result_stack| old.is_stackable_with(result_stack))
                    && u64::from(old.count()) + u64::from(result.count)
                        <= u64::from(host::item_max_stack(host_ref, old).min(result.max_stack))
            }
        };
        if !fits {
            return false;
        }
    }
    for &(slot, count) in &plan.inputs {
        match entity_items.get(slot).and_then(Option::as_ref) {
            Some(stack) if stack.count() >= count => {}
            _ => return false,
        }
    }
    for &(slot, count) in &plan.inputs {
        if let Some(Some(stack)) = entity_items.get_mut(slot) {
            let left = stack.count().saturating_sub(count);
            if left == 0 {
                entity_items[slot] = None;
            } else {
                let _ = stack.set_count(left);
            }
        }
    }
    if let Some((slot, result)) = &plan.output {
        if *slot >= entity_items.len() {
            return false;
        }
        match entity_items.get_mut(*slot) {
            Some(existing @ Some(_)) => {
                if let Some(stack) = existing.as_mut() {
                    let total = stack.count().saturating_add(result.count);
                    let _ = stack.set_count(total);
                }
            }
            Some(slot_ref) => {
                let built = assembled
                    .cloned()
                    .or_else(|| host::make_stack(host_ref, &result.id, result.count));
                let Some(stack) = built else {
                    return false;
                };
                *slot_ref = Some(stack);
            }
            None => return false,
        }
    }
    true
}

fn read_tank(entity_id: &str, nbt: &Compound, index: usize) -> Tank {
    let capacity = if entity_id == "theurgy:liquefaction_cauldron" {
        2_000
    } else {
        MACHINE_TANK_CAPACITY
    };
    host::tank_from_nbt(nbt, index, capacity)
}

fn write_tank(nbt: &mut Compound, index: usize, tank: &Tank) {
    let mut list = match nbt.get(&"fluids".into()) {
        Some(Tag::List(l)) => l.clone(),
        _ => Vec::new(),
    };
    while list.len() <= index {
        list.push(Tag::End);
    }
    let mut map = Compound::new();
    map.insert(
        "Fluid".into(),
        Tag::String(tank.fluid.as_deref().unwrap_or("minecraft:empty").into()),
    );
    map.insert("Amount".into(), Tag::Int(tank.amount));
    list[index] = Tag::Compound(map);
    nbt.insert("fluids".into(), Tag::List(list));
}

fn plan_for(
    kind: &str,
    recipe: &Recipe,
    items: &[Option<Item>],
    tanks: &[Tank],
    matches: &mut dyn FnMut(&Value, &Item) -> bool,
) -> Option<CraftPlan> {
    let layout = layout(kind)?;
    let parts = ingredient_parts(kind, recipe.json);
    if parts.is_empty() {
        return None;
    }
    let mut inputs: Vec<(usize, u32)> = Vec::new();
    let mut used: Vec<bool> = vec![false; items.len()];
    for (ingredient, count) in &parts {
        let mut chosen = None;
        for slot in layout.inputs {
            if used.get(*slot).copied().unwrap_or(true) {
                continue;
            }
            if let Some(item) = items.get(*slot).and_then(Option::as_ref)
                && item.count >= *count
                && matches(ingredient, item)
            {
                chosen = Some(*slot);
                break;
            }
        }
        let slot = chosen?;
        used[slot] = true;
        inputs.push((slot, *count));
    }
    let fluid = match fluid_part(kind, recipe.json) {
        Some((fluid_id, amount)) => {
            let tank_slot = layout.tank?;
            let tank = tanks.get(tank_slot)?;
            if tank.amount < amount || tank.fluid.as_deref() != Some(fluid_id) {
                return None;
            }
            Some((tank_slot, amount))
        }
        None => None,
    };
    let result = recipe.json.get("result")?;
    let id = if let Some(s) = result.get("id").and_then(Value::as_str) {
        s.to_owned()
    } else if let Some(s) = result.as_str() {
        s.to_owned()
    } else {
        return None;
    };
    let count = result
        .get("count")
        .and_then(Value::as_u64)
        .unwrap_or(1) as u32;
    if count == 0 {
        return None;
    }
    let max_stack = result
        .get("max_stack")
        .and_then(Value::as_u64)
        .map(|v| v.min(99) as u32)
        .unwrap_or(FALLBACK_MAX_STACK)
        .max(1);
    Some(CraftPlan {
        inputs,
        output: Some((
            layout.output,
            Item {
                id,
                count,
                max_stack,
            },
        )),
        fluid,
    })
}

fn recipe_time(json: &Value) -> i32 {
    json.get("time")
        .and_then(Value::as_i64)
        .unwrap_or(100)
        .clamp(0, i64::from(i32::MAX)) as i32
}

fn default_crafting_time(kind: &str) -> i32 {
    if matches!(kind, "theurgy:fermentation" | "theurgy:digestion") {
        200
    } else {
        100
    }
}

fn plan_for_cached(
    host_ref: &dyn Host,
    kind: &str,
    cached: &recipes::CachedRecipe,
    entity_items: &[Option<ItemStack>],
    items: &[Option<Item>],
    tanks: &[Tank],
) -> Option<CraftPlan> {
    let layout = layout(kind)?;
    if cached.inputs.is_empty() {
        return None;
    }
    let mut inputs = Vec::with_capacity(cached.inputs.len());
    let mut used = vec![false; items.len()];
    for ing in &cached.inputs {
        let mut chosen = None;
        for &slot in layout.inputs {
            if used.get(slot).copied().unwrap_or(true) {
                continue;
            }
            let Some(stack) = read_slot(entity_items, slot) else {
                continue;
            };
            if stack.count() >= ing.count && recipes::matches_cached(host_ref, ing, stack) {
                chosen = Some(slot);
                break;
            }
        }
        let slot = chosen?;
        used[slot] = true;
        inputs.push((slot, ing.count));
    }
    let fluid = match &cached.fluid {
        Some((fluid_id, amount)) => {
            let tank_slot = layout.tank?;
            let tank = tanks.get(tank_slot)?;
            if tank.amount < *amount || tank.fluid.as_deref() != Some(fluid_id.as_str()) {
                return None;
            }
            Some((tank_slot, *amount))
        }
        None => None,
    };
    let result_stack = cached.result.as_ref();
    let result_json = cached.json.get("result")?;
    let id = result_stack
        .map(|s| s.item().id(host_ref))
        .or_else(|| result_json.get("id").and_then(Value::as_str).map(str::to_owned))
        .or_else(|| result_json.as_str().map(str::to_owned))?;
    let count = result_stack
        .map(ItemStack::count)
        .unwrap_or_else(|| result_json.get("count").and_then(Value::as_u64).unwrap_or(1) as u32);
    if count == 0 {
        return None;
    }
    let max_stack = result_stack
        .map(|s| host::item_max_stack(host_ref, s))
        .unwrap_or_else(|| host::item_max_stack_id(host_ref, &id));
    Some(CraftPlan {
        inputs,
        output: Some((
            layout.output,
            Item {
                id,
                count,
                max_stack,
            },
        )),
        fluid,
    })
}

fn vat_is_open(host_ref: &dyn Host, level: LevelRef, pos: [i32; 3]) -> bool {
    level.block_property(host_ref, pos, "open").as_deref() == Some("true")
}

pub fn tick(
    host_ref: &mut dyn ServerHost,
    entity_id: &str,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    let Some(level) = host::level_in_place(host_ref) else {
        return;
    };
    let Some(kind) = kind_for_block(entity_id) else {
        return;
    };
    let Some(MachineLayout {
        tank: tank_slot, ..
    }) = layout(kind)
    else {
        return;
    };
    let input_slots: &[usize] = layout(kind).map(|l| l.inputs).unwrap_or(&[]);
    let slots = slot_count(kind);
    if entity.items.len() < slots {
        entity.items.resize(slots, None);
    }
    let mut items = read_items(&*host_ref, &entity.items, slots);
    let mut tanks: Vec<Tank> = match tank_slot {
        Some(i) => vec![read_tank(entity_id, &entity.nbt, i)],
        None => Vec::new(),
    };
    let mut state = host::crafting(&entity.nbt, &entity.data);
    let old_state = state.clone();
    let current_signature: u64;

    let can_process = if is_heat_consumer(kind) {
        heat::heated(host_ref, level, pos, entity)
    } else {
        !vat_is_open(&*host_ref, level, pos)
    };
    let was_open = !is_heat_consumer(kind) && vat_is_open(&*host_ref, level, pos);
    let mut selected_can_craft = false;

    let recipe_list = recipes::of_type(kind);
    let tank_for_match = tanks.first();
    let mut selected: Option<usize> = None;
    for (idx, cached) in recipe_list.iter().enumerate() {
        if cached.inputs.is_empty() {
            continue;
        }
        if matches!(kind, "theurgy:fermentation" | "theurgy:digestion") {
            let filled = input_slots
                .iter()
                .filter(|slot| items.get(**slot).is_some_and(Option::is_some))
                .count();
            if filled != cached.inputs.len() {
                continue;
            }
        }
        let mut ok = true;
        let mut used = vec![false; items.len()];
        for ing in &cached.inputs {
            let mut found = false;
            for &slot in input_slots {
                if used.get(slot).copied().unwrap_or(true) {
                    continue;
                }
                let Some(stack) = read_slot(&entity.items, slot) else {
                    continue;
                };
                if stack.count() >= ing.count && recipes::matches_cached(&*host_ref, ing, stack) {
                    used[slot] = true;
                    found = true;
                    break;
                }
            }
            if !found {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        if let Some((fluid_id, amount)) = &cached.fluid {
            let matches_fluid = tank_for_match.is_some_and(|t| {
                t.amount >= *amount && t.fluid.as_deref() == Some(fluid_id.as_str())
            });
            if !matches_fluid {
                continue;
            }
        }
        selected = Some(idx);
        break;
    }

    let result: ProcessResult = match selected {
        None => {
            let old = state.clone();
            let previous = ((entity.data.get(8).copied().unwrap_or(0) as u32 as u64) << 32)
                | entity.data.get(7).copied().unwrap_or(0) as u32 as u64;
            let signature = input_signature(&*host_ref, &entity.items, kind, "");
            current_signature = signature;
            let has_items = input_slots
                .iter()
                .any(|index| entity.items.get(*index).is_some_and(Option::is_some));
            if previous != signature && (previous != 0 || has_items) {
                state.total_time = default_crafting_time(kind);
                state.progress = 0;
            }
            let updates = state.stop();
            ProcessResult {
                selected_recipe: None,
                crafted: false,
                state_changed: old != state || updates != 0,
            }
        }
        Some(idx) => {
            let cached = &recipe_list[idx];
            let signature = input_signature(&*host_ref, &entity.items, kind, &cached.id);
            current_signature = signature;
            let previous = ((entity.data.get(8).copied().unwrap_or(0) as u32 as u64) << 32)
                | entity.data.get(7).copied().unwrap_or(0) as u32 as u64;
            if (previous != 0 && previous != signature) || state.total_time != cached.time {
                state.input_changed(cached.time);
            }
            let plan = plan_for_cached(&*host_ref, kind, cached, &entity.items, &items, &tanks);
            let mut can_craft = false;
            if let Some(plan_ref) = plan.as_ref() {
                let mut sim_items = items.clone();
                let mut sim_tanks = tanks.clone();
                can_craft = commit_plan(&mut sim_items, &mut sim_tanks, plan_ref);
                selected_can_craft = can_craft;
            }
            let assembled = cached.result.as_ref();
            let output_index = layout(kind).map(|value| value.output).unwrap_or(0);
            if let Some(result_stack) = assembled
                && let Some(old) = read_slot(&entity.items, output_index)
                && !old.is_stackable_with(result_stack)
            {
                can_craft = false;
                selected_can_craft = false;
            }
            let effects = state.tick(can_process, true, can_craft, cached.time);
            let did_craft = if effects.craft_attempt && can_craft {
                plan.as_ref().is_some_and(|plan| {
                    commit_plan(&mut items, &mut tanks, plan)
                        && apply_plan_to_items(&*host_ref, &mut entity.items, plan, assembled)
                })
            } else {
                false
            };
            ProcessResult {
                selected_recipe: Some(cached.id.clone()),
                crafted: did_craft,
                state_changed: old_state != state || effects.block_updates != 0,
            }
        }
    };

    for (i, tank) in tanks.iter().enumerate() {
        write_tank(&mut entity.nbt, i, tank);
    }
    let fields = &mut **entity;
    host::save_crafting(&mut fields.nbt, &mut fields.data, &state);
    let heat_cache = fields.data.get(6).copied().unwrap_or(0);
    fields.data.clear();
    fields.data.extend([
        state.progress.max(0),
        state.total_time.max(0),
        tanks.first().map_or(0, |tank| tank.amount.max(0)),
        tanks.first().map_or(0, |tank| tank.capacity.max(0)),
        i32::from(state.is_processing),
        i32::from(state.could_craft_last_tick),
        heat_cache,
        current_signature as u32 as i32,
        (current_signature >> 32) as u32 as i32,
    ]);

    if !is_heat_consumer(kind) {
        if !fields.nbt.contains_key(&"randomSlowTickOffset".into()) {
            let offset = (pos[0]
                .wrapping_mul(31)
                .wrapping_add(pos[1].wrapping_mul(17))
                .wrapping_add(pos[2]))
            .rem_euclid(20);
            fields
                .nbt
                .insert("randomSlowTickOffset".into(), Tag::Int(offset));
        }
        let offset = host::int(&fields.nbt, "randomSlowTickOffset", 0);
        let now = level.game_time(&*host_ref).unwrap_or(0);
        let redstone_signal = level.best_neighbor_signal(&*host_ref, pos).unwrap_or(0);
        if was_open
            && selected_can_craft
            && (now + i64::from(offset)) % 20 == 0
            && redstone_signal > 0
        {
            host_ref.set_block_property_with(pos, "open", "false", 2);
        }
        let previous_signal = host::int(&fields.nbt, "lastRedstoneSignal", redstone_signal);
        if previous_signal != redstone_signal {
            if redstone_signal == 0 && !was_open {
                host_ref.set_block_property_with(pos, "open", "true", 2);
            }
            host::set_int(&mut fields.nbt, "lastRedstoneSignal", redstone_signal);
        }
        let output_stack = read_slot(&fields.items, 3).cloned();
        if kind == "theurgy:fermentation" {
            let has_output = output_stack.as_ref().is_some_and(|stack| stack.count() > 0);
            let want = if has_output { "true" } else { "false" };
            if level.block_property(&*host_ref, pos, "has_output").as_deref() != Some(want) {
                host_ref.set_block_property_with(pos, "has_output", want, 3);
            }
        }
        update_vat_signal(host_ref, level, pos, output_stack.as_ref());
    }

    if result.crafted {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.5,
            f64::from(pos[2]) + 0.5,
        ];
        if let Some(dust_key) = registry::particles::dust() {
            let dust = ParticleOptions::dust(dust_key, [0.35, 0.8, 1.0], 0.75);
            let _ = level.send_particles(host_ref, &dust, None, at, [0.25, 0.35, 0.25], 0.02, 4);
        }
        if let Some(flame_key) = registry::particles::flame() {
            let flame = ParticleOptions::simple(flame_key);
            let _ = level.send_particles(host_ref, &flame, None, at, [0.15, 0.15, 0.15], 0.01, 2);
        }
    }

    if result.state_changed || result.crafted {
        host_ref.block_entity_sync(pos);
    }
}

/// `GenericVatInteractionBehaviour.useItemOn`: closing is allowed only when
/// the current items/fluid form a complete recipe whose whole output fits.
pub fn vat_can_craft(host_ref: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3]) -> bool {
    let Some(be) = host_ref.block_entity(level, pos) else {
        return false;
    };
    let Some(entity_id) = be.kind(&*host_ref).map(|k| k.id(&*host_ref)) else {
        return false;
    };
    let Some(kind) = kind_for_block(&entity_id)
        .filter(|kind| matches!(*kind, "theurgy:fermentation" | "theurgy:digestion"))
    else {
        return false;
    };
    let nbt = be.data(&*host_ref).unwrap_or_default();
    let entity_items: Vec<Option<ItemStack>> = (0..4)
        .map(|slot| be.item_slot(&*host_ref, slot).ok().flatten())
        .collect();
    let items = read_items(&*host_ref, &entity_items, 4);
    let tanks = vec![read_tank(&entity_id, &nbt, 0)];
    let list = recipes::of_type(kind);
    for cached in list.iter() {
        if cached.inputs.is_empty()
            || (0..3)
                .filter(|slot| items.get(*slot).is_some_and(Option::is_some))
                .count()
                != cached.inputs.len()
        {
            continue;
        }
        let Some(plan) = plan_for_cached(&*host_ref, kind, cached, &entity_items, &items, &tanks)
        else {
            continue;
        };
        let mut sim_items = items.clone();
        let mut sim_tanks = tanks.clone();
        if !commit_plan(&mut sim_items, &mut sim_tanks, &plan) {
            continue;
        }
        if let Some(result) = cached.result.as_ref()
            && read_slot(&entity_items, 3).is_some_and(|old| !old.is_stackable_with(result))
        {
            continue;
        }
        return true;
    }
    false
}

fn is_vat_block(level: &dyn LevelHost, state: mod_api::BlockState) -> bool {
    matches!(
        level.state_block(state),
        Some("theurgy:fermentation_vat" | "theurgy:digestion_vat")
    )
}

/// `VatRedstoneHasOutputBehaviour`: only the block's front emits 15.
pub fn block_signal(level: &dyn LevelHost, event: &mut BlockSignalEvent) {
    if !is_vat_block(level, event.state) {
        return;
    }
    let facing = host::level_state_property(level, event.state, "facing")
        .as_deref()
        .and_then(host::facing_name_to_dir)
        .unwrap_or(2);
    let face_id = host::direction_to_ordinal(event.face);
    if face_id != facing {
        event.signal = 0;
        return;
    }
    let dim = event.dimension.key().id();
    let pos = [event.pos.x, event.pos.y, event.pos.z];
    let has_output = vat_signals()
        .read()
        .ok()
        .and_then(|map| map.get(&(dim.to_owned(), pos)).map(|s| s.has_output))
        .unwrap_or_else(|| {
            host::level_state_property(level, event.state, "has_output").as_deref() == Some("true")
        });
    event.signal = if has_output { 15 } else { 0 };
}

pub fn is_signal_source(level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
    if is_vat_block(level, event.state) {
        event.source = true;
    }
}

pub fn has_analog_output(level: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
    if is_vat_block(level, event.state) {
        event.output = true;
    }
}

/// `VatAnalogSignalOutputBehaviour`: one output slot, vanilla container formula.
pub fn analog_output(level: &dyn LevelHost, event: &mut AnalogOutputEvent) {
    if !is_vat_block(level, event.state) {
        return;
    }
    let dim = event.dimension.key().id();
    let pos = [event.pos.x, event.pos.y, event.pos.z];
    event.output = vat_signals()
        .read()
        .ok()
        .and_then(|map| map.get(&(dim.to_owned(), pos)).map(|s| s.analog))
        .unwrap_or(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::serde_json::json;

    #[test]
    fn every_machine_block_maps_to_a_recipe_kind() {
        assert_eq!(
            kind_for_block("theurgy:calcination_oven"),
            Some("theurgy:calcination")
        );
        assert_eq!(
            kind_for_block("theurgy:distiller"),
            Some("theurgy:distillation")
        );
        assert_eq!(
            kind_for_block("theurgy:liquefaction_cauldron"),
            Some("theurgy:liquefaction")
        );
        assert_eq!(
            kind_for_block("theurgy:fermentation_vat"),
            Some("theurgy:fermentation")
        );
        assert_eq!(
            kind_for_block("theurgy:digestion_vat"),
            Some("theurgy:digestion")
        );
    }

    #[test]
    fn unrelated_blocks_are_ignored() {
        assert_eq!(kind_for_block("theurgy:pyromantic_brazier"), None);
        assert_eq!(kind_for_block("minecraft:furnace"), None);
    }

    #[test]
    fn slot_counts_match_container_shape() {
        assert_eq!(slot_count("theurgy:calcination"), 2);
        assert_eq!(slot_count("theurgy:distillation"), 2);
        assert_eq!(slot_count("theurgy:liquefaction"), 2);
        assert_eq!(slot_count("theurgy:fermentation"), 4);
        assert_eq!(slot_count("theurgy:digestion"), 4);
    }

    #[test]
    fn heat_consumer_classification_matches_source() {
        assert!(is_heat_consumer("theurgy:calcination"));
        assert!(is_heat_consumer("theurgy:distillation"));
        assert!(is_heat_consumer("theurgy:liquefaction"));
        assert!(!is_heat_consumer("theurgy:fermentation"));
        assert!(!is_heat_consumer("theurgy:digestion"));
    }

    #[test]
    fn recipe_time_defaults_and_clamps() {
        assert_eq!(recipe_time(&json!({})), 100);
        assert_eq!(recipe_time(&json!({"time": 50})), 50);
        assert_eq!(recipe_time(&json!({"time": -5})), 0);
    }

    #[test]
    fn plan_uses_declared_max_stack_from_result() {
        let json = json!({
            "ingredient": "minecraft:oak_log",
            "result": {"id": "theurgy:x", "count": 1, "max_stack": 16},
            "time": 100
        });
        let recipe = Recipe {
            id: "test",
            json: &json,
            time: 100,
        };
        let items = vec![
            Some(Item {
                id: "minecraft:oak_log".into(),
                count: 1,
                max_stack: 64,
            }),
            None,
        ];
        let mut matcher = |_: &Value, _item: &Item| true;
        let plan =
            plan_for("theurgy:distillation", &recipe, &items, &[], &mut matcher).expect("plan");
        assert_eq!(plan.output.as_ref().unwrap().1.max_stack, 16);
    }

    #[test]
    fn plan_rejects_shorted_fluid() {
        let json = json!({
            "ingredient": "minecraft:oak_log",
            "solvent": {"ingredient": "theurgy:sal_ammoniac", "amount": 10},
            "result": {"id": "theurgy:x", "count": 1},
            "time": 100
        });
        let recipe = Recipe {
            id: "test",
            json: &json,
            time: 100,
        };
        let items = vec![
            Some(Item {
                id: "minecraft:oak_log".into(),
                count: 1,
                max_stack: 64,
            }),
            None,
        ];
        let tanks = vec![Tank {
            fluid: Some("theurgy:sal_ammoniac".into()),
            amount: 9,
            capacity: MACHINE_TANK_CAPACITY,
        }];
        let mut matcher = |_: &Value, _item: &Item| true;
        assert!(plan_for("theurgy:liquefaction", &recipe, &items, &tanks, &mut matcher).is_none());
    }

    #[test]
    fn plan_uses_only_input_slots_and_distinct_slots() {
        let json = json!({
            "ingredients": [
                {"ingredient": "minecraft:a", "count": 1},
                {"ingredient": "minecraft:a", "count": 1}
            ],
            "fluid": {"ingredient": "minecraft:water", "amount": 100},
            "result": {"id": "theurgy:out", "count": 1},
            "time": 100
        });
        let recipe = Recipe {
            id: "t",
            json: &json,
            time: 100,
        };
        let items = vec![
            Some(Item {
                id: "minecraft:a".into(),
                count: 2,
                max_stack: 64,
            }),
            None,
            None,
            None,
        ];
        let tanks = vec![Tank {
            fluid: Some("minecraft:water".into()),
            amount: 100,
            capacity: MACHINE_TANK_CAPACITY,
        }];
        let mut matcher = |_: &Value, _item: &Item| true;
        assert!(plan_for("theurgy:fermentation", &recipe, &items, &tanks, &mut matcher).is_none());
    }

    #[test]
    fn tank_roundtrip_preserves_fluid_and_amount() {
        let mut nbt = Compound::new();
        let tank = Tank {
            fluid: Some("theurgy:sal_ammoniac".into()),
            amount: 500,
            capacity: 2_000,
        };
        write_tank(&mut nbt, 0, &tank);
        assert_eq!(read_tank("theurgy:liquefaction_cauldron", &nbt, 0), tank);
        assert_eq!(
            read_tank("theurgy:liquefaction_cauldron", &nbt, 1).fluid,
            None
        );
        assert_eq!(read_tank("theurgy:liquefaction_cauldron", &nbt, 1).amount, 0);
    }
}
