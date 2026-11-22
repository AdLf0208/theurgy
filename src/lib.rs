// SPDX-License-Identifier: MIT
//! Theurgy for Horizon — full SDK package (`api = "0.2"`).

pub mod commands;
pub mod config;
pub mod engine;
pub mod fluids;
pub mod heat;
pub mod host;
pub mod interactions;
pub mod machines;
pub mod recipes;
pub mod registry;

use mod_api::{
    self, ComponentTypesEvent, Flow, Hooks, HostSide, RegisterCommandsEvent,
    RegisterFluidModelsEvent, Registrar, RegistrationContext, RunModCommandEvent,
    ServerAboutToStartEvent, ServerHost, ServerStoppedEvent, TagsUpdatedEvent,
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
    }

    fn tags_updated(&self, host: HostSide<'_>, _: &mut TagsUpdatedEvent) {
        if let HostSide::Server(host) = host {
            recipes::publish(host);
        }
    }

    fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut ServerStoppedEvent) {
        recipes::clear();
        machines::clear_signals();
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        commands::declare(event.commands);
    }

    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut RunModCommandEvent<'_>,
    ) -> Flow {
        commands::run(host, event)
    }
}
