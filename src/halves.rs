// SPDX-License-Identifier: MIT
//! The upper half of a double-high apparatus has no inventory or tank of its own. Its items and
//! fluids are the lower half's: a hopper, pipe or inserter that reaches the upper half gets the
//! lower block entity's handler, as upstream's upper half delegates to the lower block entity.
//!
//! The upper half still carries a block entity (the game makes one for every state of a block
//! with a block entity type), but nothing reads or writes it.

use mod_api::{
    BlockEntityRef, BlockSide, BlockState, Capability, FLUIDS, FluidResource, Host, ITEMS,
    ItemResource, ResourceHandler, ServerHost, Transaction,
};

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
