// SPDX-License-Identifier: MIT
//! Central registration of Theurgy's 29 block entities, 26 data components,
//! container layouts, fluid tank layouts, recipe types, and shared Part dispatch.

use std::sync::OnceLock;

use mod_api::{
    AnalogOutputEvent, BlockEntityEvent, BlockEntityPart, BlockEntityTickEvent,
    BlockEntityTicksEvent, BlockEntityUpdateTagEvent, BlockPart, BlockSignalEvent, BlockState,
    CanSurviveEvent, Compound, HasAnalogOutputEvent, Host, InteractionResult, IsSignalSourceEvent,
    LevelHost, LevelSide, LevelTicksHost, ModContainer, ModEnergyBuffer, ModFluidTanks,
    OnDestroyedByPlayerEvent, ParticleKey, Registrar, ServerHost, StateForPlacementEvent, Tag,
    UpdateShapeEvent, UseItemOnEvent, UseWithoutItemEvent,
};

use crate::{fluids, flux, heat, incubation, interactions, items, machines, recipes, reformation};

/// All Theurgy blocks that carry a block entity, mirroring `BlockEntityRegistry`.
pub const BLOCK_ENTITIES: &[&str] = &[
    "theurgy:calcination_oven",
    "theurgy:pyromantic_brazier",
    "theurgy:liquefaction_cauldron",
    "theurgy:distiller",
    "theurgy:incubator",
    "theurgy:incubator_mercury_vessel",
    "theurgy:incubator_salt_vessel",
    "theurgy:incubator_sulfur_vessel",
    "theurgy:sal_ammoniac_tank",
    "theurgy:sal_ammoniac_accumulator",
    "theurgy:logistics_nexus",
    "theurgy:mercury_catalyst",
    "theurgy:mercury_capacitor",
    "theurgy:caloric_flux_emitter",
    "theurgy:reformation_source_pedestal",
    "theurgy:reformation_target_pedestal",
    "theurgy:reformation_result_pedestal",
    "theurgy:sulfuric_flux_emitter",
    "theurgy:mercury_flux_emitter",
    "theurgy:fermentation_vat",
    "theurgy:digestion_vat",
    "theurgy:logistics_item_inserter",
    "theurgy:logistics_item_extractor",
    "theurgy:logistics_fluid_inserter",
    "theurgy:logistics_fluid_extractor",
    "theurgy:logistics_connector_node",
    "theurgy:logistics_capability_probe",
    "theurgy:logistics_capability_proxy",
    "theurgy:logistics_mercury_flux_connector",
];

/// All data component types registered by `DataComponentRegistry`.
pub const COMPONENT_TYPES: &[&str] = &[
    "mercury_flux_storage",
    "mercury_flux_to_convert",
    "current_mercury_flux_per_tick",
    "divination_settings_allowed_blocks_tag",
    "divination_settings_disallowed_blocks_tag",
    "divination_settings_duration",
    "divination_settings_max_damage",
    "divination_settings_range",
    "divination_settings_tier",
    "divination_settings_allow_attuning",
    "divination_distance",
    "divination_pos",
    "divination_linked_block",
    "divination_linked_tag",
    "wire_end_point",
    "mercurial_wand_item_mode",
    "source_item",
    "source_tag",
    "mercury_catalyst_inventory",
    "filter_items",
    "filter_is_deny_list",
    "filter_respects_data_components",
    "filter_mode",
    "filter_attributes",
    "selected_frequency",
    "logistics_nexus_id",
];

pub mod particles {
    use super::*;

    static DUST: OnceLock<ParticleKey> = OnceLock::new();
    static FLAME: OnceLock<ParticleKey> = OnceLock::new();
    static BUBBLE: OnceLock<ParticleKey> = OnceLock::new();
    static ENCHANT: OnceLock<ParticleKey> = OnceLock::new();
    static END_ROD: OnceLock<ParticleKey> = OnceLock::new();

    pub(crate) fn init(r: &mut dyn Registrar) {
        let _ = DUST.set(r.particle_key("minecraft:dust"));
        let _ = FLAME.set(r.particle_key("minecraft:flame"));
        let _ = BUBBLE.set(r.particle_key("minecraft:bubble"));
        let _ = ENCHANT.set(r.particle_key("minecraft:enchant"));
        let _ = END_ROD.set(r.particle_key("minecraft:end_rod"));
    }

    pub fn dust() -> Option<ParticleKey> {
        DUST.get().copied()
    }
    pub fn flame() -> Option<ParticleKey> {
        FLAME.get().copied()
    }
    pub fn bubble() -> Option<ParticleKey> {
        BUBBLE.get().copied()
    }
    pub fn enchant() -> Option<ParticleKey> {
        ENCHANT.get().copied()
    }
    pub fn end_rod() -> Option<ParticleKey> {
        END_ROD.get().copied()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Layout {
    Stack { slots: usize, inputs: usize },
    Split {
        slots: usize,
        inputs: Vec<usize>,
        outputs: Vec<usize>,
    },
}

fn layout(block: &str) -> Option<Layout> {
    Some(match block {
        "theurgy:calcination_oven"
        | "theurgy:distiller"
        | "theurgy:liquefaction_cauldron" => Layout::Stack { slots: 2, inputs: 1 },
        "theurgy:fermentation_vat" | "theurgy:digestion_vat" => Layout::Stack { slots: 4, inputs: 3 },
        "theurgy:incubator" | "theurgy:reformation_result_pedestal" => Layout::Split {
            slots: 1,
            inputs: vec![],
            outputs: vec![0],
        },
        "theurgy:pyromantic_brazier"
        | "theurgy:incubator_mercury_vessel"
        | "theurgy:incubator_salt_vessel"
        | "theurgy:incubator_sulfur_vessel"
        | "theurgy:reformation_source_pedestal"
        | "theurgy:reformation_target_pedestal"
        | "theurgy:mercury_catalyst"
        | "theurgy:sal_ammoniac_accumulator" => Layout::Split {
            slots: 1,
            inputs: vec![0],
            outputs: vec![],
        },
        _ => return None,
    })
}

pub fn is_be(block: &str) -> bool {
    BLOCK_ENTITIES.contains(&block)
}

pub fn initial_nbt(block: &str) -> Option<Compound> {
    if !is_be(block) {
        return None;
    }
    let mut nbt = Compound::new();
    if matches!(
        block,
        "theurgy:calcination_oven"
            | "theurgy:distiller"
            | "theurgy:liquefaction_cauldron"
            | "theurgy:fermentation_vat"
            | "theurgy:digestion_vat"
            | "theurgy:incubator"
            | "theurgy:sal_ammoniac_accumulator"
            | "theurgy:sulfuric_flux_emitter"
    ) {
        nbt.insert("progress".into(), Tag::Short(0));
        nbt.insert("totalTime".into(), Tag::Short(0));
    }
    if block == "theurgy:pyromantic_brazier" {
        nbt.insert("remainingLitTime".into(), Tag::Short(0));
    }
    Some(nbt)
}

pub fn container(block: &str) -> Option<ModContainer> {
    let layout = layout(block)?;
    let (slots, inputs, outputs) = match layout {
        Layout::Stack { slots, inputs } => {
            let outputs = if inputs < slots {
                vec![slots - 1]
            } else {
                vec![]
            };
            (slots, (0..inputs).collect::<Vec<_>>(), outputs)
        }
        Layout::Split {
            slots,
            inputs,
            outputs,
        } => (slots, inputs, outputs),
    };
    let input_count = inputs.len();
    let mut faces: [Vec<usize>; 6] = std::array::from_fn(|_| (0..slots).collect());
    if matches!(
        block,
        "theurgy:calcination_oven"
            | "theurgy:distiller"
            | "theurgy:liquefaction_cauldron"
            | "theurgy:fermentation_vat"
            | "theurgy:digestion_vat"
    ) {
        // UP exposes only inputs, DOWN only output, horizontal faces expose combined inventory.
        faces[0] = (input_count..slots).collect();
        faces[1] = (0..input_count).collect();
    }
    let outputs = match block {
        "theurgy:incubator" | "theurgy:reformation_result_pedestal" => outputs,
        _ => (0..slots)
            .filter(|slot| inputs.contains(slot) || outputs.contains(slot))
            .collect(),
    };
    Some(ModContainer {
        slots,
        faces,
        inputs,
        outputs,
        menu: None,
    })
}

const TANK_CAPACITY: i32 = 10_000;
const SAL_AMMONIAC_TANK_CAPACITY: i32 = 2_000;

pub fn fluids(block: &str) -> Option<ModFluidTanks> {
    let capacity = match block {
        "theurgy:sal_ammoniac_tank" | "theurgy:liquefaction_cauldron" => SAL_AMMONIAC_TANK_CAPACITY,
        "theurgy:fermentation_vat"
        | "theurgy:digestion_vat"
        | "theurgy:sal_ammoniac_accumulator" => TANK_CAPACITY,
        _ => return None,
    };
    let mut tanks = ModFluidTanks::default();
    tanks.tanks = vec![capacity];
    tanks.faces = std::array::from_fn(|_| vec![0]);
    tanks.inputs = vec![0];
    tanks.outputs = vec![0];
    Some(tanks)
}

pub fn component_types(types: &mut Vec<String>) {
    for name in COMPONENT_TYPES {
        types.push(format!("theurgy:{name}"));
    }
}

/// Shared Part attached once to every declared Theurgy block entity type.
/// Because `BlockEntityPart: BlockPart`, this also handles all block hooks
/// for those 29 blocks without attaching twice.
pub struct TheurgyBlockPart;

#[mod_api::horizon_part]
impl BlockPart for TheurgyBlockPart {
    fn state_for_placement(
        &self,
        level: &dyn LevelHost,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockState>> {
        items::state_for_placement(level, event)
    }

    fn update_shape(
        &self,
        level: &mut dyn LevelTicksHost,
        event: &mut UpdateShapeEvent,
    ) -> Option<BlockState> {
        items::update_shape(level, event)
    }

    fn can_survive(&self, level: &dyn LevelHost, event: &mut CanSurviveEvent) -> Option<bool> {
        items::can_survive(level, event)
    }

    fn on_destroyed_by_player(
        &self,
        host: &mut dyn ServerHost,
        event: &mut OnDestroyedByPlayerEvent<'_>,
    ) -> Option<bool> {
        items::on_destroyed_by_player(host, event)
    }

    fn use_item_on(
        &self,
        host: &mut dyn ServerHost,
        event: &mut UseItemOnEvent<'_>,
    ) -> Option<InteractionResult> {
        interactions::use_item_on(host, event)
    }

    fn use_without_item(
        &self,
        host: &mut dyn ServerHost,
        event: &mut UseWithoutItemEvent<'_>,
    ) -> Option<InteractionResult> {
        interactions::use_without_item(host, event)
    }

    fn is_signal_source(&self, level: &dyn LevelHost, event: &mut IsSignalSourceEvent) {
        machines::is_signal_source(level, event);
    }

    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        machines::block_signal(level, event);
    }

    fn has_analog_output(&self, level: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        machines::has_analog_output(level, event);
    }

    fn analog_output(&self, level: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        machines::analog_output(level, event);
    }
}

#[mod_api::horizon_part]
impl BlockEntityPart for TheurgyBlockPart {
    fn block_entity(&self, _: LevelSide, event: &mut BlockEntityEvent<'_>) -> Option<Compound> {
        initial_nbt(event.block)
    }

    fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if is_be(event.block) {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host_ref: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        let id = event.entity.id().to_owned();
        let pos = event.pos;
        match id.as_str() {
            "theurgy:calcination_oven"
            | "theurgy:distiller"
            | "theurgy:liquefaction_cauldron"
            | "theurgy:fermentation_vat"
            | "theurgy:digestion_vat" => {
                machines::tick(host_ref, &id, pos, &mut event.entity);
            }
            "theurgy:pyromantic_brazier" => {
                heat::tick(host_ref, pos, &mut event.entity);
            }
            "theurgy:sal_ammoniac_accumulator" | "theurgy:sal_ammoniac_tank" => {
                fluids::tick(host_ref, &id, pos, &mut event.entity);
            }
            "theurgy:incubator"
            | "theurgy:incubator_mercury_vessel"
            | "theurgy:incubator_salt_vessel"
            | "theurgy:incubator_sulfur_vessel" => {
                incubation::tick(host_ref, &id, pos, &mut event.entity);
            }
            "theurgy:sulfuric_flux_emitter"
            | "theurgy:reformation_source_pedestal"
            | "theurgy:reformation_target_pedestal"
            | "theurgy:reformation_result_pedestal" => {
                reformation::tick(host_ref, &id, pos, &mut event.entity);
            }
            "theurgy:mercury_catalyst"
            | "theurgy:mercury_capacitor"
            | "theurgy:caloric_flux_emitter"
            | "theurgy:mercury_flux_emitter" => {
                flux::tick(host_ref, &id, pos, &mut event.entity);
            }
            _ => {}
        }
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        Some(event.entity.nbt.clone())
    }
}

/// Declare all block entity types, container/fluid capabilities, recipe types, and particle keys.
pub fn register(r: &mut dyn Registrar) {
    particles::init(r);
    let _ = r.fluid_key(fluids::SAL_AMMONIAC);

    for recipe_type in recipes::RECIPE_TYPES {
        r.register_recipe_type(recipe_type);
    }

    for &full_id in BLOCK_ENTITIES {
        let path = full_id.strip_prefix("theurgy:").unwrap_or(full_id);
        let block = r.block_key(full_id);
        let be_type = r.block_entity_type(path).blocks(&[path]).build();
        r.block_entity_part(&be_type, TheurgyBlockPart)
            .expect("theurgy block entity Part attaches to its declared type");
        if let Some(c) = container(full_id) {
            r.block_container(&block, c);
        }
        if let Some(f) = fluids(full_id) {
            r.block_fluids(&block, f);
        }
        if full_id == "theurgy:mercury_flux_emitter" {
            r.block_energy(
                &block,
                ModEnergyBuffer {
                    capacity: flux::MERCURY_EMITTER_CAPACITY,
                    max_insert: flux::MERCURY_EMITTER_CAPACITY,
                    max_extract: flux::MERCURY_EMITTER_CAPACITY,
                    insert_faces: [true; 6],
                    extract_faces: [true; 6],
                },
            );
        }
        let item = r.item_key(full_id);
        r.item_part(item, items::TheurgyItemPart);
    }

    for &item_id in items::STANDALONE_ITEMS {
        let item = r.item_key(item_id);
        r.item_part(item, items::TheurgyItemPart);
    }
    for tag in [
        "theurgy:alchemical_sulfurs",
        "theurgy:alchemical_niters",
        "theurgy:alchemical_salts",
        "theurgy:alchemical_mercuries",
    ] {
        r.item_part_tag(tag, items::TheurgyItemPart);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_block_has_a_block_entity() {
        for block in BLOCK_ENTITIES {
            assert!(is_be(block), "{block} should be a block entity");
        }
        assert!(!is_be("theurgy:not_a_block"));
        assert!(!is_be("minecraft:chest"));
    }

    #[test]
    fn initial_nbt_only_for_known_blocks() {
        assert!(initial_nbt("minecraft:stone").is_none());
        let nbt = initial_nbt("theurgy:distiller").expect("distiller nbt");
        assert_eq!(nbt.get(&"progress".into()), Some(&Tag::Short(0)));
        assert_eq!(nbt.get(&"totalTime".into()), Some(&Tag::Short(0)));
        let brazier = initial_nbt("theurgy:pyromantic_brazier").expect("brazier nbt");
        assert_eq!(brazier.get(&"remainingLitTime".into()), Some(&Tag::Short(0)));
    }

    #[test]
    fn machine_containers_expose_input_and_output() {
        let c = container("theurgy:distiller").expect("distiller container");
        assert_eq!(c.slots, 2);
        assert_eq!(c.inputs, vec![0]);
        assert_eq!(c.outputs, vec![0, 1]);
        assert_eq!(c.menu, None);
        assert_eq!(c.faces[0], vec![1], "bottom is output-only");
        assert_eq!(c.faces[1], vec![0], "top is input-only");
        for face in 2..6 {
            assert_eq!(c.faces[face], vec![0, 1], "horizontal combined inventory");
        }

        let vat = container("theurgy:fermentation_vat").expect("vat container");
        assert_eq!(vat.slots, 4);
        assert_eq!(vat.inputs, vec![0, 1, 2]);
        assert_eq!(vat.outputs, vec![0, 1, 2, 3]);
        assert_eq!(vat.faces[0], vec![3]);
        assert_eq!(vat.faces[1], vec![0, 1, 2]);
    }

    #[test]
    fn single_slot_blocks_route_the_right_slot() {
        let catalyst = container("theurgy:mercury_catalyst").expect("catalyst");
        assert_eq!(catalyst.inputs, vec![0]);
        assert_eq!(catalyst.outputs, vec![0]);

        let result = container("theurgy:reformation_result_pedestal").expect("result pedestal");
        assert!(result.inputs.is_empty());
        assert_eq!(result.outputs, vec![0]);

        assert!(container("theurgy:logistics_nexus").is_none());
    }

    #[test]
    fn fluid_tanks_only_where_expected() {
        let tank = fluids("theurgy:sal_ammoniac_tank").expect("tank");
        assert_eq!(tank.tanks, vec![SAL_AMMONIAC_TANK_CAPACITY]);
        assert_eq!(tank.inputs, vec![0]);
        assert_eq!(tank.outputs, vec![0]);
        assert!(fluids("theurgy:distiller").is_none());
        assert_eq!(
            fluids("theurgy:liquefaction_cauldron")
                .expect("cauldron")
                .tanks,
            vec![2_000]
        );
    }

    #[test]
    fn all_component_types_are_theurgy_namespaced() {
        let mut types = Vec::new();
        component_types(&mut types);
        assert_eq!(types.len(), COMPONENT_TYPES.len());
        assert!(types.iter().all(|t| t.starts_with("theurgy:")));
        assert!(types.contains(&"theurgy:mercury_flux_storage".to_owned()));
        assert!(types.contains(&"theurgy:logistics_nexus_id".to_owned()));
    }
}
