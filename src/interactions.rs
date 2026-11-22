// SPDX-License-Identifier: MIT
//! Direct item and fluid interactions for Theurgy apparatus blocks
//! (`BlockPart::use_item_on` and `BlockPart::use_without_item`).
//!
//! Mirrors Theurgy 26.3's `ItemHandlerBehaviour` and fluid-handler clicks:
//! - fluid containers fill/drain tanks first (`fluid_util::interact_with_fluid_handler`),
//! - sneaking with an empty hand toggles vat lids (`fermentation_vat`, `digestion_vat`),
//! - holding a valid input inserts into the input slot(s),
//! - an empty hand takes output first, then inputs.

use std::sync::atomic::{AtomicI64, Ordering};

use mod_api::{
    BlockClick, Host, HostTransaction, InteractionResult, ItemPlace, ItemResource, ItemStack,
    LevelRef, ServerHost, Text, UseItemOnEvent, UseWithoutItemEvent, fluid_util,
};

use crate::{host, machines, recipes, registry};

/// Guard against a programmatic click delivering both insert and empty-hand take
/// within the same game tick.
static LAST_INSERT_TICK: AtomicI64 = AtomicI64::new(i64::MIN);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interaction {
    TwoSlot,
    DynamicOutput,
    OneInput,
    OneOutput,
}

fn interaction(block: &str) -> Option<Interaction> {
    Some(match block {
        "theurgy:calcination_oven" | "theurgy:distiller" | "theurgy:liquefaction_cauldron" => {
            Interaction::TwoSlot
        }
        "theurgy:fermentation_vat" | "theurgy:digestion_vat" => Interaction::DynamicOutput,
        "theurgy:incubator" | "theurgy:reformation_result_pedestal" => Interaction::OneOutput,
        "theurgy:pyromantic_brazier"
        | "theurgy:incubator_mercury_vessel"
        | "theurgy:incubator_salt_vessel"
        | "theurgy:incubator_sulfur_vessel"
        | "theurgy:reformation_source_pedestal"
        | "theurgy:reformation_target_pedestal"
        | "theurgy:mercury_catalyst"
        | "theurgy:sal_ammoniac_accumulator" => Interaction::OneInput,
        _ => return None,
    })
}

fn slots(kind: Interaction) -> usize {
    match kind {
        Interaction::TwoSlot => 2,
        Interaction::DynamicOutput => 4,
        Interaction::OneInput | Interaction::OneOutput => 1,
    }
}

fn recipe_kind(block: &str) -> Option<&'static str> {
    Some(match block {
        "theurgy:calcination_oven" => "theurgy:calcination",
        "theurgy:distiller" => "theurgy:distillation",
        "theurgy:liquefaction_cauldron" => "theurgy:liquefaction",
        "theurgy:fermentation_vat" => "theurgy:fermentation",
        "theurgy:digestion_vat" => "theurgy:digestion",
        "theurgy:mercury_catalyst" => "theurgy:catalysation",
        "theurgy:sal_ammoniac_accumulator" => "theurgy:accumulation",
        _ => return None,
    })
}

/// `MonitoredItemStackHandler.isValid` for direct insertion.
pub fn accepts_input(host_ref: &dyn Host, block: &str, stack: &ItemStack) -> bool {
    let tag = match block {
        "theurgy:incubator_mercury_vessel" => Some("theurgy:alchemical_mercuries"),
        "theurgy:incubator_salt_vessel" => Some("theurgy:alchemical_salts"),
        "theurgy:incubator_sulfur_vessel" => Some("theurgy:alchemical_sulfurs"),
        "theurgy:reformation_source_pedestal" | "theurgy:reformation_target_pedestal" => {
            Some("theurgy:alchemical_sulfurs_and_niters")
        }
        _ => None,
    };
    if let Some(tag) = tag {
        return host::in_item_tag(host_ref, stack.item(), tag);
    }
    if block == "theurgy:pyromantic_brazier" {
        return host_ref.item_burn_time(stack.item()).unwrap_or(0) > 0;
    }
    let Some(kind) = recipe_kind(block) else {
        return false;
    };
    if block == "theurgy:sal_ammoniac_accumulator" {
        return recipes::of_type(kind).iter().any(|recipe| {
            recipe
                .json
                .get("solute")
                .is_some_and(|solute| recipes::matches_value(host_ref, solute, stack))
        });
    }
    recipes::of_type(kind).iter().any(|recipe| {
        recipe
            .inputs
            .iter()
            .any(|ing| recipes::matches_cached(host_ref, ing, stack))
    })
}

fn resolve_entity_pos(
    host_ref: &dyn Host,
    level: LevelRef,
    pos: [i32; 3],
) -> Option<([i32; 3], String)> {
    let mut entity_pos = pos;
    if level.block_property(host_ref, pos, "half").as_deref() == Some("upper") {
        entity_pos[1] -= 1;
    }
    let block = host::block_id_at(host_ref, level, entity_pos)?;
    Some((entity_pos, block))
}

fn vat_message(host_ref: &mut dyn ServerHost, player_id: u128, block: &str, suffix: &str) {
    let kind = if block == "theurgy:fermentation_vat" {
        "fermentation_vat"
    } else {
        "digestion_vat"
    };
    if let Some(player) = host_ref.player(player_id) {
        let _ = player.action_bar(
            host_ref,
            Text::translate(
                format!("theurgy.behaviour.interaction.{kind}.{suffix}"),
                Vec::new(),
            ),
        );
    }
}

fn remember_insert(host_ref: &dyn Host, level: LevelRef) {
    LAST_INSERT_TICK.store(level.game_time(host_ref).unwrap_or(0), Ordering::Relaxed);
}

fn has_fluid_tank(block: &str) -> bool {
    registry::fluids(block).is_some()
}

/// `BlockPart::use_item_on` for apparatus blocks holding an item in hand.
pub fn use_item_on(
    host_ref: &mut dyn ServerHost,
    event: &mut UseItemOnEvent<'_>,
) -> Option<InteractionResult> {
    let click = event.click;
    let level = host_ref.level(click.dimension.key().id())?;
    let (entity_pos, block) = resolve_entity_pos(&*host_ref, level, click.pos)?;

    // Closed vats block all non-empty-hand clicks.
    let kind = interaction(&block);
    if kind == Some(Interaction::DynamicOutput) {
        let is_open = level.block_property(&*host_ref, entity_pos, "open").as_deref() != Some("false");
        if !is_open {
            vat_message(host_ref, click.player, &block, "closed");
            return Some(InteractionResult::Success);
        }
    }

    // Try fluid tank interaction first (buckets / fluid containers).
    if has_fluid_tank(&block)
        && let Some(player) = host_ref.player(click.player)
    {
        let side = host::face_to_direction(click.hit.face);
        if fluid_util::interact_with_fluid_handler(
            host_ref,
            player,
            click.hand,
            level,
            entity_pos,
            side,
        ) {
            host_ref.block_entity_sync(entity_pos);
            remember_insert(&*host_ref, level);
            return Some(InteractionResult::Success);
        }
    }

    let kind = kind?;
    let held = click.stack.filter(|s| s.count() > 0)?;

    if kind == Interaction::OneOutput || !accepts_input(&*host_ref, &block, held) {
        return None;
    }

    let slot_count = slots(kind);
    let candidates: &[usize] = match kind {
        Interaction::TwoSlot => &[0],
        Interaction::DynamicOutput => &[0, 1, 2],
        Interaction::OneInput => &[0],
        Interaction::OneOutput => &[],
    };
    let max_stack = host::item_max_stack(&*host_ref, held);
    let entity = host_ref.block_entity(level, entity_pos)?;

    for &index in candidates {
        // Vats enforce distinct item types across their 3 input slots.
        if kind == Interaction::DynamicOutput {
            let dup = (0..slot_count - 1).any(|other| {
                other != index
                    && entity
                        .item_slot(&*host_ref, other as u32)
                        .ok()
                        .flatten()
                        .is_some_and(|s| s.is_stackable_with(held))
            });
            if dup {
                continue;
            }
        }

        let stored = entity.item_slot(&*host_ref, index as u32).ok().flatten();
        let filling = match stored.as_ref() {
            Some(existing) if !existing.is_stackable_with(held) => continue,
            Some(existing) => existing.count(),
            None => 0,
        };
        let room = max_stack.saturating_sub(filling);
        let want = held.count().min(room);
        if want == 0 {
            continue;
        }

        let hand = ItemPlace::PlayerHand {
            player: click.player,
            hand: click.hand,
        };
        let resource = ItemResource::of(held);
        let placed = host_ref.transaction(|host_ref, tx| {
            let moved = hand.extract(host_ref, tx, &resource, want);
            if moved == 0 {
                return Err(());
            }
            let entity = host_ref.block_entity(level, entity_pos).ok_or(())?;
            let total = filling.saturating_add(moved);
            let mut next = stored.clone().unwrap_or_else(|| held.clone());
            next.set_count(total).map_err(|_| ())?;
            entity
                .set_item_slot(host_ref, tx, index as u32, Some(next))
                .map_err(|_| ())?;
            Ok(())
        });
        if placed.is_ok() {
            host_ref.block_entity_sync(entity_pos);
            remember_insert(&*host_ref, level);
            return Some(InteractionResult::Success);
        }
    }
    None
}

/// `BlockPart::use_without_item`: vat lid toggle on sneak, or take output-first
/// on empty hand.
pub fn use_without_item(
    host_ref: &mut dyn ServerHost,
    event: &mut UseWithoutItemEvent<'_>,
) -> Option<InteractionResult> {
    let click = event.click;
    if click.stack.is_some_and(|s| s.count() > 0) {
        return None;
    }
    let level = host_ref.level(click.dimension.key().id())?;
    let (entity_pos, block) = resolve_entity_pos(&*host_ref, level, click.pos)?;
    let kind = interaction(&block)?;

    if kind == Interaction::DynamicOutput {
        let is_open =
            level.block_property(&*host_ref, entity_pos, "open").as_deref() != Some("false");
        let sneaking = host_ref
            .player(click.player)
            .is_some_and(|p| p.is_secondary_use_active(&*host_ref).unwrap_or(false));
        if sneaking {
            if is_open {
                if machines::vat_can_craft(host_ref, level, entity_pos) {
                    host_ref.set_block_property_with(entity_pos, "open", "false", 2);
                } else {
                    vat_message(host_ref, click.player, &block, "no_recipe");
                }
            } else {
                host_ref.set_block_property_with(entity_pos, "open", "true", 2);
                if let Some(be) = host_ref.block_entity(level, entity_pos) {
                    let mut nbt = be.data(&*host_ref).unwrap_or_default();
                    let mut data = be.data_slots(&*host_ref).unwrap_or_default();
                    let mut state = host::crafting(&nbt, &data);
                    let _ = state.stop();
                    host::save_crafting(&mut nbt, &mut data, &state);
                    let _ = be.merge_data(host_ref, nbt);
                    let _ = be.set_data_slots(host_ref, data);
                    host_ref.block_entity_sync(entity_pos);
                }
            }
            return Some(InteractionResult::Success);
        } else if !is_open {
            vat_message(host_ref, click.player, &block, "closed");
            return Some(InteractionResult::Success);
        }
    }

    take_output_first(host_ref, level, entity_pos, click, kind)
}

fn take_output_first(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    entity_pos: [i32; 3],
    click: &BlockClick<'_>,
    kind: Interaction,
) -> Option<InteractionResult> {
    if LAST_INSERT_TICK.load(Ordering::Relaxed) == level.game_time(&*host_ref).unwrap_or(0) {
        return None;
    }
    let output_first: &[usize] = match kind {
        Interaction::TwoSlot => &[1, 0],
        Interaction::DynamicOutput => &[3, 0, 1, 2],
        Interaction::OneInput | Interaction::OneOutput => &[0],
    };
    let entity = host_ref.block_entity(level, entity_pos)?;
    for &index in output_first {
        let Some(stack) = entity.item_slot(&*host_ref, index as u32).ok().flatten() else {
            continue;
        };
        if stack.count() == 0 {
            continue;
        }
        let taken = host_ref.transaction(|host_ref, tx| {
            let entity = host_ref.block_entity(level, entity_pos).ok_or(())?;
            let player = host_ref.player(click.player).ok_or(())?;
            player.give_stack(host_ref, stack.clone()).map_err(|_| ())?;
            entity
                .set_item_slot(host_ref, tx, index as u32, None)
                .map_err(|_| ())?;
            Ok::<(), ()>(())
        });
        if taken.is_ok() {
            if kind == Interaction::DynamicOutput && index == 3 {
                machines::update_vat_signal(host_ref, level, entity_pos, None);
            }
            host_ref.block_entity_sync(entity_pos);
            return Some(InteractionResult::Success);
        }
    }
    None
}

/// Pure helper mirroring `ItemUtil.insertItemReturnRemaining` for one slot.
pub fn insert_count(existing_count: u32, held_count: u32, same_resource: bool, max_stack: u32) -> u32 {
    if held_count == 0 {
        return 0;
    }
    let max_stack = max_stack.max(1);
    if existing_count > 0 && (!same_resource || existing_count >= max_stack) {
        return 0;
    }
    held_count.min(max_stack.saturating_sub(existing_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_count_respects_max_stack_and_resource_match() {
        assert_eq!(insert_count(0, 20, true, 16), 16);
        assert_eq!(insert_count(16, 20, true, 16), 0);
        assert_eq!(insert_count(5, 10, false, 64), 0);
        assert_eq!(insert_count(5, 10, true, 64), 10);
        assert_eq!(insert_count(60, 10, true, 64), 4);
    }

    #[test]
    fn output_only_never_accepts_a_held_stack() {
        assert_eq!(interaction("theurgy:incubator"), Some(Interaction::OneOutput));
        assert_eq!(
            interaction("theurgy:reformation_result_pedestal"),
            Some(Interaction::OneOutput)
        );
        assert_eq!(interaction("theurgy:distiller"), Some(Interaction::TwoSlot));
        assert_eq!(interaction("minecraft:chest"), None);
    }
}
