// SPDX-License-Identifier: MIT
//! The upper half of a double-high apparatus has no inventory or tank of its own. Its items and
//! fluids are the lower half's: a hopper, pipe or inserter that reaches the upper half gets the
//! lower block entity's handler, as upstream's upper half delegates to the lower block entity.
//!
//! The upper half still carries a block entity (the game makes one for every state of a block
//! with a block entity type). Nothing of the mod reads or writes it, and whatever lands in it
//! anyway (a world saved before the halves delegated, a tool writing the block entity directly)
//! is handed down to the lower half on its next tick (`hand_down`).

use mod_api::{
    BlockEntityRef, BlockSide, BlockState, Capability, FLUIDS, FluidResource, Host,
    HostTransaction, ITEMS, ItemResource, ItemStack, ModBlockEntityView, ResourceHandler,
    ServerHost, Transaction,
};

use crate::{host, registry};

/// Forwards every call of a handler to another, the lower half's.
pub struct Forward<R: 'static>(pub Capability<dyn ResourceHandler<R>>);

impl<R: 'static> ResourceHandler<R> for Forward<R> {
    fn size(&self, host: &mut dyn ServerHost) -> usize {
        self.0.size(host)
    }

    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<R> {
        self.0.resource(host, index)
    }

    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.0.amount(host, index)
    }

    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: &R) -> u32 {
        self.0.capacity(host, index, resource)
    }

    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        self.0.insert(host, tx, index, resource, amount)
    }

    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        self.0.extract(host, tx, index, resource, amount)
    }
}

/// The lower half's position when the block at `at` is an upper half whose lower half is the same
/// block. None for a lower half, and for an upper half with no partner below it.
pub fn lower_position(host: &dyn Host, at: &BlockSide) -> Option<[i32; 3]> {
    if at.level.block_property(host, at.pos, "half").as_deref() != Some("upper") {
        return None;
    }
    let lower = [at.pos[0], at.pos[1] - 1, at.pos[2]];
    let here = at.level.block(host, at.pos)?;
    (at.level.block(host, lower)? == here).then_some(lower)
}

/// `ITEMS` of an upper half: the lower half's item handler on the same side.
pub fn items_of_upper_half(
    host: &mut dyn ServerHost,
    at: BlockSide,
    _state: BlockState,
    _entity: Option<BlockEntityRef>,
) -> Option<Box<dyn ResourceHandler<ItemResource>>> {
    let lower = lower_position(&*host, &at)?;
    let handler = at.level.capability(host, &ITEMS, lower, at.side)?;
    Some(Box::new(Forward(handler)) as Box<dyn ResourceHandler<ItemResource>>)
}

/// `FLUIDS` of an upper half: the lower half's tank on the same side.
pub fn fluids_of_upper_half(
    host: &mut dyn ServerHost,
    at: BlockSide,
    _state: BlockState,
    _entity: Option<BlockEntityRef>,
) -> Option<Box<dyn ResourceHandler<FluidResource>>> {
    let lower = lower_position(&*host, &at)?;
    let handler = at.level.capability(host, &FLUIDS, lower, at.side)?;
    Some(Box::new(Forward(handler)) as Box<dyn ResourceHandler<FluidResource>>)
}

/// How many of `incoming` join a slot holding `stored` (its count, or 0 when empty) under
/// `max_stack`: None when the slot holds something else, so nothing may join it.
pub fn joins(stored: Option<u32>, stackable: bool, incoming: u32, max_stack: u32) -> Option<u32> {
    match stored {
        Some(_) if !stackable => None,
        Some(count) => Some(incoming.min(max_stack.saturating_sub(count))),
        None => Some(incoming.min(max_stack)),
    }
}

/// The upper half's tick: hand whatever its own block entity holds down to the lower half at
/// `pos` minus one, slot for slot and tank for tank, as far as it fits. What does not fit stays
/// and is tried again.
pub fn hand_down(
    host_ref: &mut dyn ServerHost,
    block: &str,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    let holds_items = entity.items.iter().any(Option::is_some);
    let capacity = registry::fluids(block).and_then(|tanks| tanks.tanks.first().copied());
    let own_tank = capacity.map(|capacity| host::tank(entity, 0, capacity));
    let holds_fluid = own_tank.as_ref().is_some_and(|tank| tank.amount > 0);
    if !holds_items && !holds_fluid {
        return;
    }
    let Some(level) = host::level_in_place(host_ref) else {
        return;
    };
    let lower = [pos[0], pos[1] - 1, pos[2]];
    if host::block_id_at(&*host_ref, level, lower).as_deref() != Some(block) {
        return;
    }
    let Some(below) = host_ref.block_entity(level, lower) else {
        return;
    };
    let mut changed = false;
    for index in 0..entity.items.len() {
        let Some(stack) = entity.items[index].clone() else {
            continue;
        };
        let stored = below.item_slot(&*host_ref, index as u32).ok().flatten();
        let Some(moving) = joins(
            stored.as_ref().map(ItemStack::count),
            stored.as_ref().is_some_and(|s| s.is_stackable_with(&stack)),
            stack.count(),
            host::item_max_stack(&*host_ref, &stack),
        )
        .filter(|moving| *moving > 0) else {
            continue;
        };
        let mut next = stored.clone().unwrap_or_else(|| stack.clone());
        if next
            .set_count(stored.as_ref().map_or(0, ItemStack::count) + moving)
            .is_err()
        {
            continue;
        }
        let placed = host_ref.transaction(|host_ref, tx| {
            below
                .set_item_slot(host_ref, tx, index as u32, Some(next))
                .map_err(|_| ())
        });
        if placed.is_err() {
            continue;
        }
        let left = stack.count() - moving;
        entity.items[index] = if left == 0 {
            None
        } else {
            let mut rest = stack;
            rest.set_count(left).ok().map(|()| rest)
        };
        changed = true;
    }
    if let (Some(mut own), Some(capacity)) = (own_tank.filter(|tank| tank.amount > 0), capacity)
        && let Some(mut nbt) = below.data(&*host_ref)
    {
        let mut tank = host::tank_from_nbt(&nbt, 0, capacity);
        let same = tank.amount == 0 || tank.fluid == own.fluid;
        let moving = own.amount.min(tank.capacity - tank.amount);
        if same && moving > 0 {
            tank.fluid = own.fluid.clone();
            tank.amount += moving;
            host::save_tanks_nbt(&mut nbt, std::slice::from_ref(&tank));
            if below.merge_data(host_ref, nbt).is_ok() {
                own.amount -= moving;
                if own.amount == 0 {
                    own.fluid = None;
                }
                host::save_tanks(entity, std::slice::from_ref(&own));
                changed = true;
            }
        }
    }
    if changed {
        host_ref.block_entity_sync(lower);
    }
}

#[cfg(test)]
mod tests {
    use super::joins;

    #[test]
    fn handed_down_stacks_join_only_what_they_stack_with() {
        assert_eq!(joins(None, false, 2, 64), Some(2));
        assert_eq!(joins(None, false, 70, 64), Some(64));
        assert_eq!(joins(Some(60), true, 10, 64), Some(4));
        assert_eq!(joins(Some(64), true, 10, 64), Some(0));
        assert_eq!(joins(Some(1), false, 10, 64), None);
    }
}
