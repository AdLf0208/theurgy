// SPDX-License-Identifier: MIT
//! Reformation array: `SulfuricFluxEmitterBlockEntity`,
//! `ReformationArrayCraftingBehaviour` and `ReformationRecipe`.

use mod_api::serde_json::Value;
use mod_api::{
    Compound, Host, HostTransaction, ItemStack, LevelRef, ModBlockEntityView, ServerHost, Tag,
};

use crate::{host, recipes, selection};

const EMITTER: &str = "theurgy:sulfuric_flux_emitter";
const SOURCE: &str = "theurgy:reformation_source_pedestal";
const TARGET: &str = "theurgy:reformation_target_pedestal";
const RESULT: &str = "theurgy:reformation_result_pedestal";
pub const FLUX_CAPACITY: i32 = 1_000;
const DEFAULT_TIME: i32 = 100;

fn flux(nbt: &Compound) -> i32 {
    let Some(Tag::Compound(handler)) = nbt.get(&"mercuryFluxHandler".into()) else {
        return 0;
    };
    host::int(handler, "energy", 0).clamp(0, FLUX_CAPACITY)
}

fn set_flux(nbt: &mut Compound, value: i32) {
    let clamped = value.clamp(0, FLUX_CAPACITY);
    let mut handler = match nbt.get(&"mercuryFluxHandler".into()) {
        Some(Tag::Compound(existing)) => existing.clone(),
        _ => Compound::new(),
    };
    host::set_int(&mut handler, "energy", clamped);
    nbt.insert("mercuryFluxHandler".into(), Tag::Compound(handler));
}

fn pedestal_item(
    host_ref: &dyn Host,
    level: LevelRef,
    at: [i32; 3],
    expected: &str,
) -> Option<ItemStack> {
    let be = host_ref.block_entity(level, at)?;
    if be.kind(host_ref)?.id(host_ref) != expected {
        return None;
    }
    be.item_slot(host_ref, 0)
        .ok()
        .flatten()
        .filter(|s| s.count() > 0)
}

fn pedestal_exists(host_ref: &dyn Host, level: LevelRef, at: [i32; 3], expected: &str) -> bool {
    host_ref
        .block_entity(level, at)
        .and_then(|be| be.kind(host_ref))
        .is_some_and(|k| k.id(host_ref) == expected)
}

fn as_i32(value: Option<&Value>, fallback: i32) -> i32 {
    value
        .and_then(Value::as_i64)
        .unwrap_or(i64::from(fallback))
        .clamp(0, i64::from(i32::MAX)) as i32
}

fn as_u32(value: Option<&Value>, fallback: u32) -> u32 {
    value
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(fallback))
        .min(u64::from(u32::MAX)) as u32
}

#[derive(Clone)]
struct SourceUse {
    pos: [i32; 3],
    count: u32,
    expected: ItemStack,
}

struct RecipeChoice {
    cost: i32,
    time: i32,
    sources: Vec<SourceUse>,
    result: ItemStack,
}

fn output_fits(existing: Option<&ItemStack>, result: &ItemStack, max_stack: u32) -> bool {
    match existing {
        None => result.count() <= max_stack,
        Some(old) => {
            old.is_stackable_with(result)
                && u64::from(old.count()) + u64::from(result.count()) <= u64::from(max_stack)
        }
    }
}

fn read_selection(nbt: &Compound) -> (Vec<[i32; 3]>, Option<[i32; 3]>, Option<[i32; 3]>) {
    let mut sources = Vec::new();
    for pos in selection::read_points(nbt, "sourcePedestals") {
        if !sources.contains(&pos) {
            sources.push(pos);
        }
    }
    (
        sources,
        selection::read_point(nbt, "targetPedestal"),
        selection::read_point(nbt, "resultPedestal"),
    )
}

pub fn trail_color(start: [f32; 3], end: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|axis| start[axis] + (end[axis] - start[axis]) * t)
}

pub fn tick(
    host_ref: &mut dyn ServerHost,
    entity_id: &str,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    if entity_id != EMITTER {
        return;
    }
    let Some(level) = host::level_in_place(host_ref) else {
        return;
    };
    let (selected_sources, selected_target, selected_result) = read_selection(&entity.nbt);
    let (Some(target_pos), Some(result_pos)) = (selected_target, selected_result) else {
        stop_idle(entity);
        return;
    };
    if !pedestal_exists(&*host_ref, level, result_pos, RESULT) {
        stop_idle(entity);
        return;
    }
    let Some(target_stack) = pedestal_item(&*host_ref, level, target_pos, TARGET) else {
        stop_idle(entity);
        return;
    };
    let sources: Vec<[i32; 3]> = selected_sources
        .into_iter()
        .filter(|source| pedestal_item(&*host_ref, level, *source, SOURCE).is_some())
        .collect();
    if sources.is_empty() {
        stop_idle(entity);
        return;
    }

    let available_flux = flux(&entity.nbt);
    let list = recipes::of_type("theurgy:reformation");
    let mut chosen: Option<RecipeChoice> = None;
    for cached in list.iter() {
        let json = &cached.json;
        let cost = as_i32(json.get("mercuryFlux"), 0);
        if available_flux < cost {
            continue;
        }
        let Some(target_ingredient) = json.get("target") else {
            continue;
        };
        if !recipes::matches_value(&*host_ref, target_ingredient, &target_stack) {
            continue;
        }
        let Some(required) = json.get("sources").and_then(Value::as_array) else {
            continue;
        };
        let mut used: Vec<SourceUse> = Vec::new();
        let mut complete = true;
        for entry in required {
            let ingredient = entry.get("ingredient").unwrap_or(entry);
            let count = as_u32(entry.get("count"), 1);
            let found = sources.iter().find_map(|source| {
                if used.iter().any(|previous| previous.pos == *source) {
                    return None;
                }
                let stack = pedestal_item(&*host_ref, level, *source, SOURCE)?;
                (stack.count() >= count && recipes::matches_value(&*host_ref, ingredient, &stack))
                    .then(|| SourceUse {
                        pos: *source,
                        count,
                        expected: stack,
                    })
            });
            if let Some(found) = found {
                used.push(found);
            } else {
                complete = false;
                break;
            }
        }
        if !complete {
            continue;
        }
        let Some(mut result_stack) = cached
            .result
            .clone()
            .or_else(|| json.get("result").and_then(|r| recipes::parse_result(&*host_ref, r)))
        else {
            continue;
        };
        // ReformationRecipe.assemble copies target components to the result.
        if target_stack.damage() > 0 {
            let _ = result_stack.set_damage(target_stack.damage());
        }
        if let Some(comps) = target_stack.components() {
            let _ = result_stack.apply_components(comps.as_nbt());
        }
        let max_stack = host::item_max_stack(&*host_ref, &result_stack);
        let existing = pedestal_item(&*host_ref, level, result_pos, RESULT);
        if !output_fits(existing.as_ref(), &result_stack, max_stack) {
            continue;
        }
        chosen = Some(RecipeChoice {
            cost,
            time: as_i32(json.get("time"), DEFAULT_TIME),
            sources: used,
            result: result_stack,
        });
        break;
    }

    let mut state = host::crafting(&entity.nbt, &entity.data);
    let total = chosen.as_ref().map_or(DEFAULT_TIME, |choice| choice.time);
    if state.total_time != total {
        state.input_changed(total);
    }
    let effects = state.tick(true, true, chosen.is_some(), total);
    let mut current_flux = available_flux;
    let mut crafted = false;

    if effects.craft_attempt
        && let Some(choice) = chosen
        && current_flux >= choice.cost
    {
        let max_stack = host::item_max_stack(&*host_ref, &choice.result);
        let committed = host_ref.transaction(|host_ref, tx| {
            // Revalidate and consume sources.
            for source in &choice.sources {
                let be = host_ref.block_entity(level, source.pos).ok_or(())?;
                let Some(mut stack) = be.item_slot(&*host_ref, 0).ok().flatten() else {
                    return Err(());
                };
                if !stack.is_stackable_with(&source.expected) || stack.count() < source.count {
                    return Err(());
                }
                let left = stack.count() - source.count;
                let next = if left == 0 {
                    None
                } else {
                    stack.set_count(left).map_err(|_| ())?;
                    Some(stack)
                };
                be.set_item_slot(host_ref, tx, 0, next).map_err(|_| ())?;
            }
            // Revalidate and write result pedestal.
            let res_be = host_ref.block_entity(level, result_pos).ok_or(())?;
            let existing = res_be.item_slot(&*host_ref, 0).ok().flatten();
            if !output_fits(existing.as_ref(), &choice.result, max_stack) {
                return Err(());
            }
            let next_out = match existing {
                Some(mut old) => {
                    let total = old.count().saturating_add(choice.result.count());
                    old.set_count(total).map_err(|_| ())?;
                    old
                }
                None => choice.result.clone(),
            };
            res_be
                .set_item_slot(host_ref, tx, 0, Some(next_out))
                .map_err(|_| ())?;
            Ok::<(), ()>(())
        });
        if committed.is_ok() {
            for source in &choice.sources {
                host_ref.block_entity_sync(source.pos);
            }
            host_ref.block_entity_sync(result_pos);
            current_flux -= choice.cost;
            crafted = true;
            state.progress = 0;
        }
    }

    let fields = &mut **entity;
    set_flux(&mut fields.nbt, current_flux);
    host::save_crafting(&mut fields.nbt, &mut fields.data, &state);
    fields.data.clear();
    fields.data.extend([
        state.progress,
        state.total_time,
        current_flux,
        FLUX_CAPACITY,
        i32::from(state.is_processing),
        i32::from(state.could_craft_last_tick),
    ]);
    if effects.block_updates > 0 || crafted {
        host_ref.block_entity_sync(pos);
    }
}

fn stop_idle(entity: &mut ModBlockEntityView<'_>) {
    let fields = &mut **entity;
    let mut state = host::crafting(&fields.nbt, &fields.data);
    let _ = state.tick(true, false, false, DEFAULT_TIME);
    host::save_crafting(&mut fields.nbt, &mut fields.data, &state);
    let current_flux = flux(&fields.nbt);
    fields.data.clear();
    fields.data.extend([
        state.progress,
        state.total_time,
        current_flux,
        FLUX_CAPACITY,
        i32::from(state.is_processing),
        i32::from(state.could_craft_last_tick),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projectile_color_lerp_matches_java_endpoints() {
        let start = [2.0 / 3.0, 8.0 / 255.0, 2.0 / 3.0];
        let end = [0.0, 1.0, 0.0];
        for (step, expected) in [(0.0, start), (1.0, end)] {
            assert_eq!(trail_color(start, end, step), expected);
        }
    }

    #[test]
    fn flux_never_wraps_below_zero_or_above_capacity() {
        let mut nbt = Compound::new();
        set_flux(&mut nbt, -5);
        assert_eq!(flux(&nbt), 0);
        set_flux(&mut nbt, 5_000);
        assert_eq!(flux(&nbt), FLUX_CAPACITY);
    }

    #[test]
    fn malformed_handler_is_safe() {
        let mut nbt = Compound::new();
        nbt.insert("mercuryFluxHandler".into(), Tag::Int(7));
        assert_eq!(flux(&nbt), 0);
        set_flux(&mut nbt, 42);
        assert_eq!(flux(&nbt), 42);
    }

    #[test]
    fn no_selection_means_no_sources_or_pedestals() {
        let nbt = Compound::new();
        assert_eq!(read_selection(&nbt), (Vec::new(), None, None));
    }
}
