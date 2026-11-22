//! Theurgy for Horizon — the calcination slice as one `impl Hooks` and two block entity Parts.
//!
//! Ported from Theurgy 1.78.0 (`klikli-dev/theurgy`, MIT AND CC-BY-4.0); see `FINDINGS.md`
//! for the behaviour read from the jar and the upstream repository, `CONTENT.md` for the
//! carried content, and `notes.md` for the hook map, the gaps and the migration to the SDK
//! format.
//!
//! The two blocks are **block entities**, so their moments are Parts attached to the types
//! this mod declares — not global hooks: the pyromantic brazier burns furnace fuel and
//! publishes its `lit` state, and the calcination oven above it samples that heat once every
//! 20 ticks and advances one recipe tick per heated tick. A click on either block is a block
//! Part (`use_item_on` carries an item, `use_without_item` an empty hand), and the
//! `/theurgy` readouts are the harness surface.
//!
//! The numbers — the jar's 86 recipes, the oven's progress rule and the brazier's burn rule —
//! live in [`runtime`], which holds no `mod_api` type and is unit-tested by `cargo test` alone.

pub mod engine;
pub mod runtime;
pub mod support;

use mod_api::{
    self, BlockEntityEvent, BlockEntityPart, BlockEntityTickEvent, BlockEntityTicksEvent,
    BlockPart, Compound, Flow, Hooks, Host, InteractionResult, LevelSide, RegisterCommandsEvent,
    Registrar, RegistrationContext, RunModCommandEvent, ServerHost, UseItemOnEvent,
    UseWithoutItemEvent,
};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    /// Declare the two block entity types, their Parts and the item slots the world reaches
    /// (upstream's `EntityBlock` plus NeoForge's `RegisterCapabilitiesEvent`).
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        support::declare(r);
    }

    /// `/theurgy info|recipe|recipes`.
    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        support::declare_commands(event.commands);
    }

    /// Run one of the commands above and take it.
    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut RunModCommandEvent<'_>,
    ) -> Flow {
        support::run_command(host, event)
    }
}

/// The calcination oven: its block entity (input 0, output 1) and the click that feeds it and
/// takes its result.
pub(crate) struct Oven;

#[mod_api::horizon_part]
impl BlockPart for Oven {
    fn use_item_on(
        &self,
        host: &mut dyn ServerHost,
        event: &mut UseItemOnEvent<'_>,
    ) -> Option<InteractionResult> {
        support::oven_use_item_on(host, event)
    }

    fn use_without_item(
        &self,
        host: &mut dyn ServerHost,
        event: &mut UseWithoutItemEvent<'_>,
    ) -> Option<InteractionResult> {
        support::oven_use_without_item(host, event)
    }
}

#[mod_api::horizon_part]
impl BlockEntityPart for Oven {
    /// The oven starts with upstream's empty compound; a declared type gets one either way.
    fn block_entity(&self, _: LevelSide, _: &mut BlockEntityEvent<'_>) -> Option<Compound> {
        Some(Compound::new())
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        event.ticks = true;
    }

    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        support::oven_tick(host, event);
    }
}

/// The pyromantic brazier: one fuel slot and the burn that publishes `lit`.
pub(crate) struct Brazier;

#[mod_api::horizon_part]
impl BlockPart for Brazier {
    fn use_item_on(
        &self,
        host: &mut dyn ServerHost,
        event: &mut UseItemOnEvent<'_>,
    ) -> Option<InteractionResult> {
        support::brazier_use_item_on(host, event)
    }

    fn use_without_item(
        &self,
        host: &mut dyn ServerHost,
        event: &mut UseWithoutItemEvent<'_>,
    ) -> Option<InteractionResult> {
        support::brazier_use_without_item(host, event)
    }
}

#[mod_api::horizon_part]
impl BlockEntityPart for Brazier {
    fn block_entity(&self, _: LevelSide, _: &mut BlockEntityEvent<'_>) -> Option<Compound> {
        Some(Compound::new())
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        event.ticks = true;
    }

    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        support::brazier_tick(host, event);
    }
}
