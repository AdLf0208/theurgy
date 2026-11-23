// SPDX-License-Identifier: MIT
//! Theurgy for Horizon — full SDK package (`api = "0.2"`).

pub mod book;
pub mod client;
pub mod commands;
pub mod config;
pub mod engine;
pub mod filter;
pub mod filter_client;
pub mod fluids;
pub mod flux;
pub mod halves;
pub mod heat;
pub mod host;
pub mod incubation;
pub mod interactions;
pub mod items;
pub mod logistics;
pub mod machines;
pub mod model_data;
pub mod probe;
pub mod recipe_client;
pub mod recipes;
pub mod reformation;
pub mod registry;
pub mod render;
pub mod selection;

use mod_api::{
    self, BlockChangedEvent, ClientHost, ClientTickPostEvent, ComponentTypesEvent, LootDropsEvent,
    EntityPlaceEvent,
    Flow, GuiLayerDrawEvent, Hooks, HostSide, InputKeyEvent, InteractionKeyMappingTriggeredEvent,
    ItemStack, LeftClickBlockEvent, LevelHost, LivingEntityUseItemFinishEvent, MouseScrollingEvent,
    PlayerLoadFromFileEvent, PlayerLoggedInEvent, PlayerSaveToFileEvent, PrepareItemCraftEvent,
    RegisterCommandsEvent, RegisterFluidModelsEvent, RegisterGuiLayersEvent, Registrar,
    RegistrationContext, RightClickBlockEvent, RunModCommandEvent, ScreenCharTypedPreEvent,
    ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenMouseDraggedPreEvent,
    ScreenMouseScrolledPreEvent, ScreenRenderPostEvent, ServerAboutToStartEvent, ServerHost,
    ServerStoppedEvent, ServerTickEvent, StandaloneModelsEvent, TagsUpdatedEvent,
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

    fn standalone_models(&self, _: &RegistrationContext, event: &mut StandaloneModelsEvent<'_>) {
        render::standalone_models(event);
    }

    fn register_gui_layers(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterGuiLayersEvent<'_>,
    ) {
        client::register_gui_layers(event);
    }

    fn server_about_to_start(&self, _: &mut dyn ServerHost, _: &mut ServerAboutToStartEvent) {
        recipes::clear();
        machines::clear_signals();
        selection::clear();
        logistics::clear();
        items::clear_removed_state();
    }

    fn tags_updated(&self, host: HostSide<'_>, _: &mut TagsUpdatedEvent) {
        if let HostSide::Server(host) = host {
            recipes::publish(host);
        }
    }

    fn server_tick(&self, host: &mut dyn ServerHost, _: &mut ServerTickEvent) {
        client::drain_local_actions(host);
        items::drain_support_checks(host);
    }

    fn block_changed(&self, level: &dyn LevelHost, event: &mut BlockChangedEvent) {
        items::on_neighbor_block_changed(level, event);
    }

    fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut ServerStoppedEvent) {
        recipes::clear();
        machines::clear_signals();
        selection::clear();
        logistics::clear();
        items::clear_removed_state();
    }

    fn player_load_from_file(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerLoadFromFileEvent<'_>,
    ) {
        client::book_load(event.player, event.directory);
    }

    fn player_save_to_file(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerSaveToFileEvent<'_>,
    ) {
        client::book_save(event.player, event.directory);
    }

    fn player_logged_in(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerLoggedInEvent<'_>,
    ) {
        client::book_logged_in(host, event.player);
    }

    fn right_click_block(
        &self,
        host: HostSide<'_>,
        event: &mut RightClickBlockEvent<'_>,
    ) -> Flow {
        match host {
            HostSide::Server(host) => {
                if matches!(items::right_click_block(host, event), Flow::Cancel) {
                    return Flow::Cancel;
                }
                selection::right_click_block(host, event)
            }
            HostSide::Client(host) => {
                let sneaking = host.is_key_down("key.keyboard.left.shift")
                    || host.is_key_down("key.keyboard.right.shift");
                if interactions::client_right_click(&*host, event, sneaking) {
                    event.cancelled = true;
                    return Flow::Cancel;
                }
                Flow::Continue
            }
        }
    }

    fn loot_drops(&self, host: &mut dyn ServerHost, event: &mut LootDropsEvent<'_>) {
        items::loot_drops(host, event);
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

    fn client_tick_post(&self, host: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
        client::client_tick_post(host);
    }

    fn gui_layer_draw(&self, host: &dyn ClientHost, event: &mut GuiLayerDrawEvent<'_>) {
        client::gui_layer_draw(host, event);
    }

    fn input_key(&self, host: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
        client::input_key(host, event);
    }

    fn mouse_scrolling(
        &self,
        host: &mut dyn ClientHost,
        event: &mut MouseScrollingEvent,
    ) -> Flow {
        client::mouse_scrolling(host, event)
    }

    fn interaction_key_mapping_triggered(
        &self,
        host: &mut dyn ClientHost,
        event: &mut InteractionKeyMappingTriggeredEvent,
    ) -> Flow {
        client::interaction_key(host, event)
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        client::screen_render_post(host, event);
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        client::screen_mouse_pressed(host, event)
    }

    fn screen_mouse_dragged_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseDraggedPreEvent<'_>,
    ) -> Flow {
        client::screen_mouse_dragged(host, event)
    }

    fn screen_mouse_scrolled_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseScrolledPreEvent<'_>,
    ) -> Flow {
        client::screen_mouse_scrolled(host, event)
    }

    fn screen_key_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        client::screen_key_pressed(host, event)
    }

    fn screen_char_typed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenCharTypedPreEvent<'_>,
    ) -> Flow {
        client::screen_char_typed(host, event)
    }
}
