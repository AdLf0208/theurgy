// SPDX-License-Identifier: MIT
//! Theurgy for Horizon — full SDK package (`api = "0.2"`).

pub mod commands;
pub mod config;
pub mod engine;
pub mod filter;
pub mod fluids;
pub mod flux;
pub mod heat;
pub mod host;
pub mod incubation;
pub mod interactions;
pub mod items;
pub mod logistics;
pub mod machines;
pub mod probe;
pub mod recipes;
pub mod reformation;
pub mod registry;
pub mod selection;

use mod_api::{
    self, ComponentTypesEvent, EntityPlaceEvent, Flow, Hooks, HostSide, ItemStack,
    LeftClickBlockEvent, LivingEntityUseItemFinishEvent, PrepareItemCraftEvent,
    RegisterCommandsEvent, RegisterFluidModelsEvent, Registrar, RegistrationContext,
    RightClickBlockEvent, RunModCommandEvent, ServerAboutToStartEvent, ServerHost,
    ServerStoppedEvent, TagsUpdatedEvent,
};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        registry::register(r);
    }

    fn component_types(&self, _: &RegistrationContext, event: &mut ComponentTypesEvent<'_>) {
        registry::component_types(event.types);
    }

    fn register_fluid_models(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterFluidModelsEvent<'_>,
    ) {
        fluids::register_fluid_models(event);
    }

    fn server_about_to_start(&self, _: &mut dyn ServerHost, _: &mut ServerAboutToStartEvent) {
        recipes::clear();
        machines::clear_signals();
        selection::clear();
        logistics::clear();
    }

    fn tags_updated(&self, host: HostSide<'_>, _: &mut TagsUpdatedEvent) {
        if let HostSide::Server(host) = host {
            recipes::publish(host);
        }
    }

    fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut ServerStoppedEvent) {
        recipes::clear();
        machines::clear_signals();
        selection::clear();
        logistics::clear();
    }

    fn right_click_block(
        &self,
        host: HostSide<'_>,
        event: &mut RightClickBlockEvent<'_>,
    ) -> Flow {
        if let HostSide::Server(host) = host {
            if matches!(items::right_click_block(host, event), Flow::Cancel) {
                return Flow::Cancel;
            }
            selection::right_click_block(host, event)
        } else {
            Flow::Continue
        }
    }

    fn left_click_block(
        &self,
        host: &mut dyn ServerHost,
        event: &mut LeftClickBlockEvent,
    ) -> Flow {
        selection::left_click_block(host, event)
    }

    fn living_entity_use_item_finish(
        &self,
        host: HostSide<'_>,
        event: &mut LivingEntityUseItemFinishEvent<'_>,
    ) {
        if let HostSide::Server(host) = host {
            items::finish_using(host, event);
        }
    }

    fn prepare_item_craft(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PrepareItemCraftEvent<'_>,
    ) -> Option<ItemStack> {
        items::prepare_item_craft(host, event)
    }

    fn entity_place(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityPlaceEvent,
    ) -> Flow {
        if let Some(placer) = event.placer
            && let Some(uuid) = placer.uuid(host)
            && let Some(level) = host::level_in_place(host)
            && let Some(block_id) =
                host::block_id_at(&*host, level, [event.pos.x, event.pos.y, event.pos.z])
        {
            selection::placed(
                host,
                level,
                uuid,
                &block_id,
                [event.pos.x, event.pos.y, event.pos.z],
            );
        }
        Flow::Continue
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        commands::declare(event.commands);
        probe::register(event.commands);
        filter::register(event.commands);
    }

    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut RunModCommandEvent<'_>,
    ) -> Flow {
        if matches!(commands::run(host, event), Flow::Cancel) {
            return Flow::Cancel;
        }
        if matches!(filter::run(host, event), Flow::Cancel) {
            return Flow::Cancel;
        }
        probe::run(host, event)
    }
}
