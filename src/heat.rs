// SPDX-License-Identifier: MIT
//! Heat provider (`pyromantic_brazier`) and heat consumer check (`HeatConsumerBehaviour`).

use mod_api::{Host, LevelRef, ModBlockEntityView, ServerHost, Tag};

use crate::engine::crafting::Crafting;
use crate::host;

pub const BRAZIER: &str = "theurgy:pyromantic_brazier";
const HEAT_CHECK_INTERVAL: i64 = 20;
const HEAT_TARGET_TICKS: i32 = 20 * 6;

#[cfg(test)]
fn key(pos: [i32; 3]) -> (i32, i32, i32) {
    (pos[0], pos[1], pos[2])
}

fn stack_burn_time(host_ref: &dyn Host, stack: &mod_api::ItemStack) -> i32 {
    host_ref.item_burn_time(stack.item()).unwrap_or(0).max(0)
}

fn consume_fuel(host_ref: &dyn Host, entity: &mut ModBlockEntityView<'_>) -> bool {
    let Some(Some(stack)) = entity.items.first_mut() else {
        return false;
    };
    if stack.count() > 1 {
        let left = stack.count() - 1;
        let _ = stack.set_count(left);
        return true;
    }
    let remainder = host_ref.item_crafting_remainder(stack);
    entity.items[0] = remainder;
    true
}

fn decrement_lit_time(current: i32) -> i32 {
    // PyromanticBrazierBlockEntity.tickServer decrements only positive values.
    if current > 0 { current - 1 } else { current }
}

/// The pyromantic brazier's server tick: refuel from slot 0, decrement the
/// remaining lit time, and mirror `lit` into the block state.
pub fn tick(host_ref: &mut dyn ServerHost, pos: [i32; 3], entity: &mut ModBlockEntityView<'_>) {
    if entity.items.is_empty() {
        entity.items.push(None);
    }
    let mut remaining = host::int(&entity.nbt, "remainingLitTime", 0);
    let was_lit = remaining > 0;
    remaining = decrement_lit_time(remaining);
    let has_fuel = entity.items.first().and_then(Option::as_ref).is_some();
    let mut turned_on = false;
    if remaining <= 0 && has_fuel {
        let burn = entity
            .items
            .first()
            .and_then(Option::as_ref)
            .map_or(0, |stack| stack_burn_time(&*host_ref, stack));
        if burn > 0 {
            remaining = burn;
            if consume_fuel(&*host_ref, entity) {
                turned_on = true;
            } else {
                remaining = 0;
            }
        }
    }
    let lit = remaining > 0;
    entity
        .nbt
        .insert("remainingLitTime".into(), Tag::Short(remaining as i16));
    if was_lit != lit || turned_on {
        host_ref.set_block_property_with(pos, "lit", if lit { "true" } else { "false" }, 3);
        host_ref.block_entity_sync(pos);
    }
}

/// Whether a heat consumer at `pos` is heated, as Java's `HeatConsumerBehaviour.isHeated`:
/// - the block directly below is a lit pyromantic brazier, or
/// - its own heat receiver was filled by a caloric flux emitter and has not expired.
///
/// Re-checks only when `gameTime % 20 == 0` and otherwise returns the cached value (`data[6]`).
pub fn heated(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) -> bool {
    let now = level.game_time(&*host_ref).unwrap_or(0);
    if entity.data.len() < 7 {
        entity.data.resize(7, 0);
    }
    if now % HEAT_CHECK_INTERVAL != 0 {
        return entity.data[6] != 0;
    }

    let below = [pos[0], pos[1] - 1, pos[2]];
    let below_hot = host::block_id_at(&*host_ref, level, below).as_deref() == Some(BRAZIER)
        && (level.block_property(&*host_ref, below, "lit").as_deref() == Some("true")
            || host_ref
                .block_entity(level, below)
                .and_then(|be| be.data(&*host_ref))
                .is_some_and(|nbt| host::int(&nbt, "remainingLitTime", 0) > 0));
    let hot = below_hot || i64::from(host::int(&entity.nbt, "hotUntil", 0)) > now;
    let was_hot = entity.data[6] != 0;
    entity.data[6] = i32::from(hot);
    if was_hot != hot && level.block_property(&*host_ref, pos, "lit").is_some() {
        host_ref.set_block_property_with(pos, "lit", if hot { "true" } else { "false" }, 3);
    }
    hot
}

/// The value a caloric flux emitter should write after a successful transfer.
pub fn hot_until_for(host_ref: &dyn Host, level: LevelRef) -> i32 {
    let now = level.game_time(host_ref).unwrap_or(0);
    now.saturating_add(i64::from(HEAT_TARGET_TICKS))
        .min(i64::from(i32::MAX)) as i32
}

/// Mirror crafting `is_processing` into a machine's `lit` block property when present.
pub fn mirror_lit(host_ref: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3], state: &Crafting) {
    if level.block_property(&*host_ref, pos, "lit").is_some() {
        host_ref.set_block_property_with(
            pos,
            "lit",
            if state.is_processing { "true" } else { "false" },
            3,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_java_short_is_preserved_without_fuel() {
        assert_eq!(decrement_lit_time(-123), -123);
        assert_eq!(decrement_lit_time(0), 0);
        assert_eq!(decrement_lit_time(1), 0);
        assert_eq!(decrement_lit_time(32_767), 32_766);
    }

    #[test]
    fn hot_until_is_six_seconds_ahead() {
        assert_eq!(HEAT_TARGET_TICKS, 120);
    }

    #[test]
    fn brazier_key_is_position() {
        assert_eq!(key([1, 2, 3]), (1, 2, 3));
    }
}
