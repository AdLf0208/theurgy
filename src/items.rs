// SPDX-License-Identifier: MIT
//! Item & block-item behaviours: double-high apparatus placement/break,
//! directional attachment survival, divination rods, mercurial wand, copper
//! wire helpers, nexus UUIDs, crafting attunement, and full tooltip parity.

use std::sync::atomic::{AtomicU64, Ordering};

use mod_api::text::TextStyle;
use mod_api::{
    BlockState, BlockUpdateFlags, CanSurviveEvent, ClientHost, Compound, Direction, Flow, Hand,
    Host, HostSide, HostTransaction, Identifier, InteractionResult, InventoryTickEvent, ItemKey,
    ItemPart, ItemPlace, ItemStack, LevelHost, LevelRef, LevelTicksHost,
    LivingEntityUseItemFinishEvent, OnBlockPlacedByEvent, OnDestroyedByPlayerEvent,
    PrepareItemCraftEvent, ReleaseUsingEvent, RightClickBlockEvent, ServerHost, SoundSource,
    StateForPlacementEvent, Tag, Text, TooltipEvent, UpdateShapeEvent, UseAnimation,
    UseAnimationEvent, UseDurationEvent, UseItemEvent, UseOnEvent,
};

use crate::{config, host, selection};

/// Blocks whose Java class is a double-high apparatus.
pub const DOUBLE_HIGH: [&str; 4] = [
    "theurgy:calcination_oven",
    "theurgy:liquefaction_cauldron",
    "theurgy:distiller",
    "theurgy:incubator",
];

pub const WIRE_ITEM: &str = "theurgy:copper_wire";
pub const WIRE_END_POINT: &str = "theurgy:wire_end_point";
pub const WIRE_MAX_RANGE: i64 = 32;

pub const DIVINATION_LINKED_BLOCK: &str = "theurgy:divination_linked_block";
pub const DIVINATION_LINKED_TAG: &str = "theurgy:divination_linked_tag";
pub const DIVINATION_POS: &str = "theurgy:divination_pos";
pub const DIVINATION_DISTANCE: &str = "theurgy:divination_distance";
pub const DIVINATION_RANGE: &str = "theurgy:divination_settings_range";
pub const DIVINATION_MAX_RANGE: i32 = 96;
pub const DIVINATION_TIER: &str = "theurgy:divination_settings_tier";
pub const DIVINATION_DURATION: &str = "theurgy:divination_settings_duration";
pub const DIVINATION_NOT_FOUND: f32 = 7.0;
pub const DIVINATION_SEARCHING: f32 = 8.0;

pub const MERCURIAL_WAND: &str = "theurgy:mercurial_wand";
pub const MERCURIAL_WAND_MODE: &str = "theurgy:mercurial_wand_item_mode";
pub const SELECTED_FREQUENCY: &str = "theurgy:selected_frequency";

pub const WIRE_ENDPOINTS: [&str; 8] = [
    "theurgy:logistics_connector_node",
    "theurgy:logistics_item_inserter",
    "theurgy:logistics_item_extractor",
    "theurgy:logistics_fluid_inserter",
    "theurgy:logistics_fluid_extractor",
    "theurgy:logistics_capability_probe",
    "theurgy:logistics_capability_proxy",
    "theurgy:logistics_mercury_flux_connector",
];

const WAND_CONNECTORS: [&str; 5] = [
    "theurgy:logistics_item_inserter",
    "theurgy:logistics_item_extractor",
    "theurgy:logistics_fluid_inserter",
    "theurgy:logistics_fluid_extractor",
    "theurgy:logistics_mercury_flux_connector",
];

pub const WAND_MODE_NAMES: [&str; 5] = [
    "cycle_direction",
    "switch_logistics_enabled",
    "select_frequency",
    "set_selected_frequency",
    "cycle_capacitor_side",
];

/// Non-block, non-derivative Theurgy items needing an `ItemPart`.
pub const STANDALONE_ITEMS: &[&str] = &[
    "theurgy:divination_rod_t1",
    "theurgy:divination_rod_t2",
    "theurgy:divination_rod_t3",
    "theurgy:divination_rod_t4",
    "theurgy:sulfur_attuned_divination_rod_abundant",
    "theurgy:sulfur_attuned_divination_rod_common",
    "theurgy:sulfur_attuned_divination_rod_rare",
    "theurgy:sulfur_attuned_divination_rod_precious",
    "theurgy:amethyst_divination_rod",
    "theurgy:mercurial_wand",
    "theurgy:copper_wire",
    "theurgy:the_hermetica",
    "theurgy:list_filter",
    "theurgy:attribute_filter",
    "theurgy:sal_ammoniac_crystal",
    "theurgy:purified_gold",
    "theurgy:fermentation_starter",
    "theurgy:crystallized_water",
    "theurgy:crystallized_lava",
];

static UUID_SEQ: AtomicU64 = AtomicU64::new(0x5468_6575_7267_7901);

pub fn is_double_high(block: &str) -> bool {
    DOUBLE_HIGH.contains(&block)
}

pub fn is_wire_endpoint(block: &str) -> bool {
    WIRE_ENDPOINTS.contains(&block)
}

pub fn is_air_name(name: &str) -> bool {
    matches!(
        name,
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    )
}

fn facing_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Down => "down",
        Direction::Up => "up",
        Direction::North => "north",
        Direction::South => "south",
        Direction::West => "west",
        Direction::East => "east",
    }
}

fn horizontal_yaw(yaw: f32) -> &'static str {
    match (yaw.div_euclid(90.0).round() as i32).rem_euclid(4) {
        0 => "south",
        1 => "west",
        2 => "north",
        _ => "east",
    }
}

fn opposite_direction(d: Direction) -> Direction {
    match d {
        Direction::Down => Direction::Up,
        Direction::Up => Direction::Down,
        Direction::North => Direction::South,
        Direction::South => Direction::North,
        Direction::West => Direction::East,
        Direction::East => Direction::West,
    }
}

fn direction_normal(d: Direction) -> [i32; 3] {
    match d {
        Direction::Down => [0, -1, 0],
        Direction::Up => [0, 1, 0],
        Direction::North => [0, 0, -1],
        Direction::South => [0, 0, 1],
        Direction::West => [-1, 0, 0],
        Direction::East => [1, 0, 0],
    }
}

// ---------------------------------------------------------------------------
// Component helpers
// ---------------------------------------------------------------------------

pub fn component_tag(host_ref: &dyn Host, stack: &ItemStack, id: &str) -> Option<Tag> {
    if let Some(patch) = stack.components().map(|c| c.as_nbt()) {
        if let Some(value) = patch.get(&id.into()) {
            return Some(value.clone());
        }
        let removed_key = format!("!{id}");
        if patch.contains_key(&removed_key.as_str().into()) {
            return None;
        }
    }
    host_ref.item_default_component(stack.item(), id)
}

pub fn component_string(host_ref: &dyn Host, stack: &ItemStack, id: &str) -> Option<String> {
    match component_tag(host_ref, stack, id)? {
        Tag::String(v) => Some(v.to_string_lossy()),
        _ => None,
    }
}

pub fn component_bool(host_ref: &dyn Host, stack: &ItemStack, id: &str) -> bool {
    match component_tag(host_ref, stack, id) {
        Some(Tag::Byte(v)) => v != 0,
        Some(Tag::Int(v)) => v != 0,
        Some(Tag::Short(v)) => v != 0,
        _ => false,
    }
}

pub fn component_i32(host_ref: &dyn Host, stack: &ItemStack, id: &str) -> Option<i32> {
    match component_tag(host_ref, stack, id)? {
        Tag::Int(v) => Some(v),
        Tag::Short(v) => Some(i32::from(v)),
        Tag::Byte(v) => Some(i32::from(v)),
        _ => None,
    }
}

pub fn set_stack_component(stack: &mut ItemStack, id: &str, value: Tag) {
    let mut patch = Compound::new();
    patch.insert(id.into(), value);
    let _ = stack.apply_components(&patch);
}

pub fn remove_stack_component(stack: &mut ItemStack, id: &str) {
    let mut patch = Compound::new();
    patch.insert(format!("!{id}").as_str().into(), Tag::End);
    let _ = stack.apply_components(&patch);
}

pub fn update_player_hand(
    host_ref: &mut dyn ServerHost,
    player_id: u128,
    hand: Hand,
    update: impl FnOnce(&mut ItemStack),
) -> bool {
    let place = ItemPlace::PlayerHand {
        player: player_id,
        hand,
    };
    let Some(mut stack) = place.stack(host_ref) else {
        return false;
    };
    update(&mut stack);
    let Some(player) = host_ref.player(player_id) else {
        return false;
    };
    let slot = match hand {
        Hand::Main => player.selected_slot(&*host_ref).unwrap_or(0) as u32,
        Hand::Off => 40,
    };
    host_ref
        .transaction(|host_ref, _tx| {
            player
                .set_inventory_slot(host_ref, slot, Some(stack))
                .map_err(|_| ())
        })
        .is_ok()
}

pub fn set_held_component(
    host_ref: &mut dyn ServerHost,
    player_id: u128,
    hand: Hand,
    id: &str,
    value: Tag,
) {
    let _ = update_player_hand(host_ref, player_id, hand, |stack| {
        set_stack_component(stack, id, value);
    });
}

pub fn clear_held_component(
    host_ref: &mut dyn ServerHost,
    player_id: u128,
    hand: Hand,
    id: &str,
) {
    let _ = update_player_hand(host_ref, player_id, hand, |stack| {
        remove_stack_component(stack, id);
    });
}

/// Java `BlockPos.asLong`: x in high 26 bits, z in middle 26 bits, y in low 12 bits.
pub fn block_pos_long(pos: [i32; 3]) -> i64 {
    ((i64::from(pos[0]) & 0x3ff_ffff) << 38)
        | ((i64::from(pos[2]) & 0x3ff_ffff) << 12)
        | (i64::from(pos[1]) & 0xfff)
}

pub fn block_pos_from_long(value: i64) -> [i32; 3] {
    let x = (value >> 38) as i32;
    let y = ((value << 52) >> 52) as i32;
    let z = ((value << 26) >> 38) as i32;
    [x, y, z]
}

pub fn manhattan(a: [i32; 3], b: [i32; 3]) -> i64 {
    (i64::from(a[0]) - i64::from(b[0])).abs()
        + (i64::from(a[1]) - i64::from(b[1])).abs()
        + (i64::from(a[2]) - i64::from(b[2])).abs()
}

pub fn uuid_text(host_ref: &dyn Host, stack: &ItemStack) -> Option<String> {
    match component_tag(host_ref, stack, "theurgy:logistics_nexus_id")? {
        Tag::String(value) => Some(value.to_string_lossy()),
        Tag::IntArray(values) if values.len() == 4 => {
            let most = ((values[0] as u32 as u64) << 32) | (values[1] as u32 as u64);
            let least = ((values[2] as u32 as u64) << 32) | (values[3] as u32 as u64);
            Some(format!(
                "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                (most >> 32) as u32,
                ((most >> 16) & 0xffff) as u16,
                (most & 0xffff) as u16,
                (least >> 48) as u16,
                least & 0xffff_ffff_ffff
            ))
        }
        _ => None,
    }
}

fn next_uuid_tag(seed_extra: u64) -> Tag {
    let s1 = UUID_SEQ
        .fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed)
        ^ seed_extra;
    let mut most = s1.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let mut least = most.wrapping_mul(0x94d0_49bb_1331_11eb) ^ 0xd1b5_4a32_d192_ed03;
    most = (most & 0xffff_ffff_ffff_0fff) | 0x0000_0000_0000_4000;
    least = (least & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
    Tag::IntArray(vec![
        (most >> 32) as i32,
        most as i32,
        (least >> 32) as i32,
        least as i32,
    ])
}

// ---------------------------------------------------------------------------
// Block placement, survival & double-high apparatus
// ---------------------------------------------------------------------------

pub fn state_for_placement(
    level: &dyn LevelHost,
    event: &mut StateForPlacementEvent<'_>,
) -> Option<Option<BlockState>> {
    let block = level.state_block(event.state)?.to_owned();
    if !block.starts_with("theurgy:") {
        return None;
    }
    let mut state = event.state;
    let connector = matches!(
        block.as_str(),
        "theurgy:logistics_item_inserter"
            | "theurgy:logistics_item_extractor"
            | "theurgy:logistics_fluid_inserter"
            | "theurgy:logistics_fluid_extractor"
            | "theurgy:logistics_mercury_flux_connector"
            | "theurgy:logistics_capability_proxy"
            | "theurgy:logistics_capability_probe"
            | "theurgy:logistics_connector_node"
            | "theurgy:caloric_flux_emitter"
            | "theurgy:mercury_flux_emitter"
            | "theurgy:sulfuric_flux_emitter"
    );
    if connector {
        let requested = host::level_state_property(level, state, "facing");
        let mut facing = event.context.face;
        let hit = [
            event.context.hit_pos.x,
            event.context.hit_pos.y,
            event.context.hit_pos.z,
        ];
        if let Some(other) = level.block_state(hit)
            && level.state_block(other) == Some(block.as_str())
            && host::level_state_property(level, other, "facing").as_deref()
                == Some(facing_name(facing))
        {
            facing = opposite_direction(facing);
        }
        let chosen = requested
            .filter(|v| v != "north")
            .unwrap_or_else(|| facing_name(facing).to_owned());
        if let Some(next) = level.state_with_property(state, "facing", &chosen) {
            state = next;
        }
    } else if matches!(
        block.as_str(),
        "theurgy:fermentation_vat" | "theurgy:digestion_vat"
    ) {
        let facing = host::level_state_property(level, state, "facing")
            .filter(|v| v != "north")
            .unwrap_or_else(|| horizontal_yaw(event.context.yaw).to_owned());
        if let Some(next) = level.state_with_property(state, "facing", &facing) {
            state = next;
        }
        let open = host::level_state_property(level, state, "open")
            .filter(|v| v == "false")
            .unwrap_or_else(|| "true".to_owned());
        if let Some(next) = level.state_with_property(state, "open", &open) {
            state = next;
        }
    }

    if host::level_state_property(level, state, "waterlogged").is_some() {
        let wet = !event.fluid.is_empty() && event.fluid.source;
        if let Some(next) =
            level.state_with_property(state, "waterlogged", if wet { "true" } else { "false" })
        {
            state = next;
        }
    }

    if block == "theurgy:logistics_nexus" {
        for dir in [
            Direction::Down,
            Direction::Up,
            Direction::North,
            Direction::South,
            Direction::West,
            Direction::East,
        ] {
            let n = direction_normal(dir);
            let at = [
                event.context.pos.x + n[0],
                event.context.pos.y + n[1],
                event.context.pos.z + n[2],
            ];
            let present = level
                .block_state(at)
                .and_then(|s| level.state_block(s))
                .is_some_and(|name| !is_air_name(name));
            if let Some(next) = level.state_with_property(
                state,
                facing_name(dir),
                if present { "true" } else { "false" },
            ) {
                state = next;
            }
        }
    }

    Some(Some(state))
}

/// Java `canSurvive` for directional attachments.
pub fn attachment_survives(block: &str, support_air: bool, support_sturdy: bool) -> Option<bool> {
    if WIRE_ENDPOINTS.contains(&block) {
        Some(!support_air)
    } else if matches!(
        block,
        "theurgy:caloric_flux_emitter"
            | "theurgy:mercury_flux_emitter"
            | "theurgy:sulfuric_flux_emitter"
    ) {
        Some(support_sturdy)
    } else {
        None
    }
}

pub fn update_shape(
    level: &mut dyn LevelTicksHost,
    event: &mut UpdateShapeEvent,
) -> Option<BlockState> {
    let block = level.state_block(event.state)?.to_owned();
    if block == "theurgy:logistics_nexus" {
        let present = level
            .state_block(event.neighbour)
            .is_some_and(|other| !is_air_name(other));
        let next = level.state_with_property(
            event.state,
            facing_name(event.direction),
            if present { "true" } else { "false" },
        )?;
        return (next != event.state).then_some(next);
    }
    let facing = host::level_state_property(level, event.state, "facing")?;
    if facing != facing_name(opposite_direction(event.direction)) {
        return None;
    }
    let support_air = level
        .state_block(event.neighbour)
        .is_none_or(is_air_name);
    // Conservative sturdy proxy when only BlockState is available on LevelHost:
    // non-air full blocks satisfy support check.
    let support_sturdy = !support_air;
    if attachment_survives(&block, support_air, support_sturdy) == Some(false) {
        return level.block_state_id("minecraft:air");
    }
    None
}

pub fn can_survive(level: &dyn LevelHost, event: &mut CanSurviveEvent) -> Option<bool> {
    let block = level.state_block(event.state)?.to_owned();
    let facing = host::level_state_property(level, event.state, "facing")?;
    let direction = [
        Direction::Down,
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ]
    .into_iter()
    .find(|d| facing_name(*d) == facing)?;
    let n = direction_normal(opposite_direction(direction));
    let support_pos = [
        event.pos.x + n[0],
        event.pos.y + n[1],
        event.pos.z + n[2],
    ];
    let support_air = level
        .block_state(support_pos)
        .and_then(|s| level.state_block(s))
        .is_none_or(is_air_name);
    let support_sturdy = !support_air;
    attachment_survives(&block, support_air, support_sturdy)
}

fn apply_block_item_components(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    stack: &ItemStack,
    pos: [i32; 3],
) {
    let item_id = stack.item().id(&*host_ref);
    let flux = component_i32(&*host_ref, stack, "theurgy:mercury_flux_storage");
    let to_convert = component_i32(&*host_ref, stack, "theurgy:mercury_flux_to_convert");
    let per_tick = component_i32(&*host_ref, stack, "theurgy:current_mercury_flux_per_tick");
    let nexus = (item_id == "theurgy:logistics_nexus")
        .then(|| uuid_text(&*host_ref, stack))
        .flatten();
    if flux.is_none() && to_convert.is_none() && per_tick.is_none() && nexus.is_none() {
        return;
    }
    let Some(be) = host_ref.block_entity(level, pos) else {
        return;
    };
    let mut nbt = be.data(&*host_ref).unwrap_or_default();
    if let Some(amount) = flux {
        let mut handler = Compound::new();
        handler.insert("energy".into(), Tag::Int(amount.max(0)));
        nbt.insert("mercuryFluxHandler".into(), Tag::Compound(handler));
    }
    if let Some(amount) = to_convert {
        nbt.insert("mercuryFluxToConvert".into(), Tag::Int(amount));
    }
    if let Some(amount) = per_tick {
        nbt.insert("currentMercuryFluxPerTick".into(), Tag::Int(amount));
    }
    if let Some(id) = nexus {
        nbt.insert("nexusId".into(), Tag::String(id.as_str().into()));
    }
    let _ = be.merge_data(host_ref, nbt);
    host_ref.block_entity_sync(pos);
}

pub fn on_block_placed_by(host_ref: &mut dyn ServerHost, event: &mut OnBlockPlacedByEvent<'_>) {
    let Some(level) = host::level_in_place(host_ref) else {
        return;
    };
    let item_id = event.stack.item().id(&*host_ref);
    if let Some(placer) = event.placer
        && let Some(uuid) = placer.entity().uuid(host_ref)
    {
        selection::placed(host_ref, level, uuid, &item_id, event.pos);
    }
    apply_block_item_components(host_ref, level, event.stack, event.pos);
    if !is_double_high(&item_id) {
        return;
    }
    let upper_str = format!("{item_id}[half=upper]");
    if let Some(upper) = host_ref.block_state_id(&upper_str) {
        let above = [event.pos[0], event.pos[1] + 1, event.pos[2]];
        let _ = level.set_block(
            host_ref,
            above,
            upper,
            BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
        );
    }
}

pub fn on_destroyed_by_player(
    host_ref: &mut dyn ServerHost,
    event: &mut OnDestroyedByPlayerEvent<'_>,
) -> Option<bool> {
    let level = host::level_in_place(host_ref)?;
    let serialized = host_ref.block_state_string(event.state)?;
    let block = serialized.split('[').next()?;
    if !is_double_high(block) {
        return None;
    }
    let half = host::parse_state_property(&serialized, "half")?;
    let (other, expected) = match half {
        "lower" => ([event.pos[0], event.pos[1] + 1, event.pos[2]], "upper"),
        "upper" => ([event.pos[0], event.pos[1] - 1, event.pos[2]], "lower"),
        _ => return None,
    };
    if host::block_id_at(&*host_ref, level, other).as_deref() == Some(block)
        && level.block_property(&*host_ref, other, "half").as_deref() == Some(expected)
        && let Some(air) = host_ref.block_state_id("minecraft:air")
    {
        let _ = level.set_block(
            host_ref,
            other,
            air,
            BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
        );
    }
    None
}

// ---------------------------------------------------------------------------
// Divination rods
// ---------------------------------------------------------------------------

pub fn distance_bucket(distance: f32) -> f32 {
    if distance < 6.0 {
        0.0
    } else if distance < 15.0 {
        1.0
    } else if distance < 25.0 {
        2.0
    } else if distance < 35.0 {
        3.0
    } else if distance < 45.0 {
        4.0
    } else if distance < 65.0 {
        5.0
    } else {
        6.0
    }
}

pub fn incorrect_blocks_tag(tier: &str) -> &'static str {
    match tier {
        "wood" | "wooden" => "minecraft:incorrect_for_wooden_tool",
        "gold" => "minecraft:incorrect_for_gold_tool",
        "iron" => "minecraft:incorrect_for_iron_tool",
        "diamond" => "minecraft:incorrect_for_diamond_tool",
        "netherite" => "minecraft:incorrect_for_netherite_tool",
        _ => "minecraft:incorrect_for_stone_tool",
    }
}

pub fn ore_tag_candidates(block: &str) -> Vec<String> {
    let path = block.split_once(':').map_or(block, |(_, path)| path);
    let Some(stem) = path.strip_suffix("_ore") else {
        return vec![format!("c:ores/{path}")];
    };
    let words: Vec<&str> = stem.split('_').collect();
    let mut candidates = Vec::with_capacity(2);
    if words.len() >= 2 {
        candidates.push(format!(
            "c:ores/{}_{}",
            words[words.len() - 2],
            words[words.len() - 1]
        ));
    }
    if let Some(word) = words.last() {
        candidates.push(format!("c:ores/{word}"));
    }
    candidates
}

pub fn deepslate_variant(block: &str) -> Option<String> {
    let (namespace, path) = block.split_once(':').unwrap_or(("minecraft", block));
    (path.contains("_ore") && !path.contains("deepslate_"))
        .then(|| format!("{namespace}:deepslate_{path}"))
}

fn held_divination_pos(host_ref: &dyn Host, stack: &ItemStack) -> Option<[i32; 3]> {
    match component_tag(host_ref, stack, DIVINATION_POS)? {
        Tag::IntArray(values) if values.len() == 3 => Some([values[0], values[1], values[2]]),
        Tag::List(values) if values.len() == 3 => {
            let mut pos = [0; 3];
            for (i, v) in values.iter().enumerate() {
                let Tag::Int(n) = v else {
                    return None;
                };
                pos[i] = *n;
            }
            Some(pos)
        }
        _ => None,
    }
}

fn translated_source_block(host_ref: &dyn Host, source: &str) -> Option<String> {
    if let Some(mapped) = config::source_override(source) {
        return Some(mapped.to_owned());
    }
    if host_ref.block_state_id(source).is_some() {
        return Some(source.to_owned());
    }
    let (namespace, path) = source.split_once(':')?;
    let mut translated = path.to_owned();
    for (needle, replacement) in [
        ("raw_", ""),
        ("_nugget", ""),
        ("_ingot", ""),
        ("_dust", ""),
        ("_block", ""),
    ] {
        if translated.contains(needle) {
            translated = translated.replace(needle, replacement);
            break;
        }
    }
    if translated.contains("deepslate_") {
        translated = translated.replace("deepslate_", "");
    } else if translated.contains("_ore") {
        translated = translated.replace("_ore", "");
    }
    translated.push_str("_ore");
    let candidate = format!("{namespace}:{translated}");
    host_ref.block_state_id(&candidate).map(|_| candidate)
}

fn block_tag_nonempty(host_ref: &dyn Host, tag: &str) -> bool {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    let Ok(id) = Identifier::parse(tag) else {
        return false;
    };
    let Some(key) = host_ref.tags().key(&id) else {
        return false;
    };
    host_ref
        .tags()
        .blocks(key)
        .is_some_and(|blocks| !blocks.is_empty())
}

fn translated_source_tag(host_ref: &dyn Host, source: &str) -> Option<String> {
    if let Some(mapped) = config::source_override(source) {
        return Some(mapped.to_owned());
    }
    if block_tag_nonempty(host_ref, source) {
        return Some(source.to_owned());
    }
    if source == "minecraft:coals" {
        return Some("c:ores/coal".to_owned());
    }
    let (namespace, path) = source.split_once(':')?;
    let mut translated = path.to_owned();
    for prefix in [
        "ingots/",
        "nuggets/",
        "raw_materials/",
        "dusts/",
        "storage_blocks/",
        "gems/",
    ] {
        if translated.contains(prefix) {
            translated = translated.replace(prefix, "ores/");
            break;
        }
    }
    if translated.ends_with("/netherite") {
        translated = translated.replace("/netherite", "/netherite_scrap");
    }
    let candidate = format!("{namespace}:{translated}");
    block_tag_nonempty(host_ref, &candidate).then_some(candidate)
}

fn attune_rod(
    host_ref: &mut dyn ServerHost,
    stack: &ItemStack,
    player_id: u128,
    hand: Hand,
    pos: [i32; 3],
) -> Option<InteractionResult> {
    let player = host_ref.player(player_id)?;
    if !player.is_secondary_use_active(&*host_ref).unwrap_or(false) {
        return None;
    }
    if !component_bool(&*host_ref, stack, "theurgy:divination_settings_allow_attuning") {
        return Some(InteractionResult::Success);
    }
    let level = player.level(&*host_ref)?;
    let block_key = level.block(&*host_ref, pos)?;
    let block = block_key.id(&*host_ref);
    if is_air_name(&block) {
        return Some(InteractionResult::Success);
    }
    let tier = component_string(&*host_ref, stack, DIVINATION_TIER)
        .unwrap_or_else(|| "stone".to_owned());
    if host::in_block_tag(&*host_ref, block_key, incorrect_blocks_tag(&tier)) {
        return Some(InteractionResult::Success);
    }
    let allowed = component_string(&*host_ref, stack, "theurgy:divination_settings_allowed_blocks_tag");
    let disallowed =
        component_string(&*host_ref, stack, "theurgy:divination_settings_disallowed_blocks_tag");
    if let Some(tag) = allowed.as_deref()
        && !tag.is_empty()
        && !host::in_block_tag(&*host_ref, block_key, tag)
    {
        return Some(InteractionResult::Success);
    }
    if let Some(tag) = disallowed.as_deref()
        && !tag.is_empty()
        && host::in_block_tag(&*host_ref, block_key, tag)
    {
        return Some(InteractionResult::Success);
    }
    let linked = translated_source_block(&*host_ref, &block).unwrap_or(block);
    set_held_component(
        host_ref,
        player_id,
        hand,
        DIVINATION_LINKED_BLOCK,
        Tag::String(linked.as_str().into()),
    );
    Some(InteractionResult::Success)
}

fn scan_rod(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    origin: [f64; 3],
    stack: &ItemStack,
) -> (Option<[i32; 3]>, f32) {
    let linked_block =
        component_string(&*host_ref, stack, DIVINATION_LINKED_BLOCK).filter(|v| !v.is_empty());
    let linked_tag =
        component_string(&*host_ref, stack, DIVINATION_LINKED_TAG).filter(|v| !v.is_empty());
    if linked_block.is_none() && linked_tag.is_none() {
        return (None, DIVINATION_NOT_FOUND);
    }
    let radius = component_i32(&*host_ref, stack, DIVINATION_RANGE)
        .unwrap_or(DIVINATION_MAX_RANGE)
        .clamp(0, DIVINATION_MAX_RANGE);
    let [ox, oy, oz] = origin;
    let origin_block = [ox.floor() as i32, oy.floor() as i32, oz.floor() as i32];

    let linked_ore_tag = linked_block
        .as_deref()
        .and_then(|block| ore_tag_candidates(block).into_iter().find(|t| block_tag_nonempty(&*host_ref, t)));
    let linked_deepslate = linked_block.as_deref().and_then(|block| {
        deepslate_variant(block).filter(|c| host_ref.block_state_id(c).is_some())
    });

    let mut best: Option<([i32; 3], f32)> = None;
    let mut best_distance2 = f64::INFINITY;
    let r2 = f64::from(radius * radius);

    for dx in -radius..=radius {
        let x = origin_block[0] + dx;
        for dy in -radius..=radius {
            let y = origin_block[1] + dy;
            for dz in -radius..=radius {
                let z = origin_block[2] + dz;
                let pos = [x, y, z];
                let Some(block_key) = level.block(&*host_ref, pos) else {
                    continue;
                };
                let block = block_key.id(&*host_ref);
                if block.is_empty() || is_air_name(&block) {
                    continue;
                }
                let matches = if let Some(tag) = linked_tag.as_deref() {
                    host::in_block_tag(&*host_ref, block_key, tag)
                } else if let Some(tag) = linked_ore_tag.as_deref() {
                    host::in_block_tag(&*host_ref, block_key, tag)
                } else if let Some(id) = linked_block.as_deref() {
                    block == id || linked_deepslate.as_deref() == Some(block.as_str())
                } else {
                    false
                };
                if !matches {
                    continue;
                }
                let ddx = f64::from(x) + 0.5 - ox;
                let ddy = f64::from(y) + 0.5 - oy;
                let ddz = f64::from(z) + 0.5 - oz;
                let d2 = ddx * ddx + ddy * ddy + ddz * ddz;
                if d2 > r2 || d2 >= best_distance2 {
                    continue;
                }
                best_distance2 = d2;
                best = Some((pos, (ddx * ddx + ddz * ddz).sqrt() as f32));
            }
        }
    }

    match best {
        Some((pos, dist)) => (Some(pos), distance_bucket(dist)),
        None => (None, DIVINATION_NOT_FOUND),
    }
}

pub fn finish_using(
    host_ref: &mut dyn ServerHost,
    event: &mut LivingEntityUseItemFinishEvent<'_>,
) {
    let item_id = event.stack.item().id(&*host_ref);
    if !item_id.contains("divination_rod") {
        return;
    }
    let mut result_stack = event.stack.clone();
    let max_damage = component_i32(&*host_ref, event.stack, "theurgy:divination_settings_max_damage")
        .unwrap_or(1)
        .max(1) as u32;
    if result_stack.damage() >= max_damage {
        if result_stack.count() > 1 {
            let left = result_stack.count() - 1;
            let _ = result_stack.set_count(left);
            let _ = result_stack.clear_damage();
            *event.result = Some(result_stack);
        } else {
            *event.result = None;
        }
        return;
    }
    if let Some(player) = host_ref.players().acting_player()
        && let Some(level) = player.level(&*host_ref)
    {
        let origin = player.entity().position(&*host_ref).unwrap_or([0.0; 3]);
        let (found_pos, bucket) = scan_rod(host_ref, level, origin, event.stack);
        if let Some(pos) = found_pos {
            set_stack_component(&mut result_stack, DIVINATION_POS, Tag::IntArray(pos.to_vec()));
        }
        set_stack_component(
            &mut result_stack,
            DIVINATION_DISTANCE,
            Tag::Float(bucket.to_bits()),
        );
        let duration = component_i32(&*host_ref, event.stack, DIVINATION_DURATION)
            .unwrap_or(40)
            .max(0);
        let group = host_ref.item_cooldown_group(event.stack);
        let _ = player.set_item_cooldown(host_ref, &group, duration);
        if !player.is_creative(&*host_ref).unwrap_or(false) {
            let next_dmg = result_stack.damage().saturating_add(1);
            let _ = result_stack.set_damage(next_dmg);
        }
    }
    *event.result = Some(result_stack);
}

// ---------------------------------------------------------------------------
// Mercurial Wand
// ---------------------------------------------------------------------------

pub fn wand_mode(host_ref: &dyn Host, stack: &ItemStack) -> i32 {
    if let Some(mode) = component_i32(host_ref, stack, MERCURIAL_WAND_MODE) {
        return mode.clamp(0, 4);
    }
    component_string(host_ref, stack, MERCURIAL_WAND_MODE)
        .and_then(|name| WAND_MODE_NAMES.iter().position(|c| *c == name.as_str()))
        .unwrap_or(0) as i32
}

fn next_direction(ordinal: i32) -> i32 {
    match ordinal {
        1 => 2,
        2 => 4,
        4 => 3,
        3 => 5,
        5 => 0,
        0 => 1,
        _ => 2,
    }
}

fn face_direction_upper(face: [i32; 3]) -> &'static str {
    match face {
        [0, -1, 0] => "DOWN",
        [0, 1, 0] => "UP",
        [0, 0, -1] => "NORTH",
        [0, 0, 1] => "SOUTH",
        [-1, 0, 0] => "WEST",
        [1, 0, 0] => "EAST",
        _ => "NORTH",
    }
}

fn wand_use_on(
    host_ref: &mut dyn ServerHost,
    stack: &ItemStack,
    player_id: u128,
    hand: Hand,
    level: LevelRef,
    pos: [i32; 3],
    face: [i32; 3],
) -> bool {
    if stack.item().id(&*host_ref) != MERCURIAL_WAND {
        return false;
    }
    let Some(block) = host::block_id_at(&*host_ref, level, pos) else {
        return false;
    };
    let connector = WAND_CONNECTORS.contains(&block.as_str());
    match wand_mode(&*host_ref, stack) {
        0 if connector => {
            let facing = level
                .block_property(&*host_ref, pos, "facing")
                .as_deref()
                .and_then(host::facing_name_to_dir)
                .map_or(2, i32::from);
            if let Some(be) = host_ref.block_entity(level, pos) {
                let mut nbt = be.data(&*host_ref).unwrap_or_default();
                let current = host::int(&nbt, "directionOverride", facing);
                host::set_int(&mut nbt, "directionOverride", next_direction(current));
                let _ = be.merge_data(host_ref, nbt);
                host_ref.block_entity_sync(pos);
            }
            true
        }
        1 if connector => {
            if let Some(be) = host_ref.block_entity(level, pos) {
                let mut nbt = be.data(&*host_ref).unwrap_or_default();
                let current = nbt.get(&"enabled".into()).is_none() || host::bool(&nbt, "enabled");
                nbt.insert("enabled".into(), Tag::Byte(i8::from(!current)));
                let _ = be.merge_data(host_ref, nbt);
                host_ref.block_entity_sync(pos);
            }
            true
        }
        2 => {
            let frequency = component_i32(&*host_ref, stack, SELECTED_FREQUENCY).unwrap_or(0) + 1;
            set_held_component(host_ref, player_id, hand, SELECTED_FREQUENCY, Tag::Int(frequency));
            true
        }
        3 if connector => {
            let frequency = component_i32(&*host_ref, stack, SELECTED_FREQUENCY).unwrap_or(0);
            if let Some(be) = host_ref.block_entity(level, pos) {
                let mut nbt = be.data(&*host_ref).unwrap_or_default();
                host::set_int(&mut nbt, "frequency", frequency);
                let _ = be.merge_data(host_ref, nbt);
                host_ref.block_entity_sync(pos);
            }
            true
        }
        4 if block == "theurgy:mercury_capacitor" => {
            let dir_name = face_direction_upper(face);
            if let Some(be) = host_ref.block_entity(level, pos) {
                let mut nbt = be.data(&*host_ref).unwrap_or_default();
                let mut modes = match nbt.get(&"sideModes".into()) {
                    Some(Tag::Compound(map)) => map.clone(),
                    _ => Compound::new(),
                };
                let fallback = if matches!(dir_name, "UP" | "DOWN") { 2 } else { 1 };
                let current = host::int(&modes, dir_name, fallback);
                host::set_int(&mut modes, dir_name, (current + 1).rem_euclid(4));
                nbt.insert("sideModes".into(), Tag::Compound(modes));
                let _ = be.merge_data(host_ref, nbt);
                host_ref.block_entity_sync(pos);
            }
            true
        }
        _ => false,
    }
}

pub fn change_wand_mode(host_ref: &mut dyn ServerHost, player_id: u128, delta: i32) -> bool {
    let place = ItemPlace::PlayerHand {
        player: player_id,
        hand: Hand::Main,
    };
    let Some(held) = place.stack(host_ref) else {
        return false;
    };
    if held.item().id(&*host_ref) != MERCURIAL_WAND {
        return false;
    }
    let current = wand_mode(&*host_ref, &held);
    let next = (current + delta).rem_euclid(5);
    set_held_component(
        host_ref,
        player_id,
        Hand::Main,
        MERCURIAL_WAND_MODE,
        Tag::String(WAND_MODE_NAMES[next as usize].into()),
    );
    let freq = Text::literal(
        component_i32(&*host_ref, &held, SELECTED_FREQUENCY)
            .unwrap_or(0)
            .to_string(),
    );
    let msg = match next {
        0 => Text::translate(
            "item.mode.theurgy.mercurial_wand.rotate_selected_direction",
            Vec::new(),
        ),
        1 => Text::translate(
            "item.mode.theurgy.mercurial_wand.switch_logistics_enabled",
            Vec::new(),
        ),
        2 => Text::translate(
            "item.mode.theurgy.mercurial_wand.select_frequency",
            vec![freq],
        ),
        3 => Text::translate(
            "item.mode.theurgy.mercurial_wand.set_selected_frequency",
            vec![freq],
        ),
        _ => Text::translate(
            "item.mode.theurgy.mercurial_wand.cycle_capacitor_side",
            Vec::new(),
        ),
    };
    if let Some(player) = host_ref.player(player_id) {
        let _ = player.action_bar(host_ref, msg);
    }
    true
}

/// Early `right_click_block` for Mercurial Wand (and copper wire in Step 7).
pub fn right_click_block(
    host_ref: &mut dyn ServerHost,
    event: &mut RightClickBlockEvent<'_>,
) -> Flow {
    if event.cancelled {
        return Flow::Continue;
    }
    let click = event.click;
    let Some(stack) = click.stack else {
        return Flow::Continue;
    };
    let Some(level) = host_ref.level(click.dimension.key().id()) else {
        return Flow::Continue;
    };
    let item_id = stack.item().id(&*host_ref);
    if item_id == MERCURIAL_WAND
        && wand_use_on(
            host_ref,
            stack,
            click.player,
            click.hand,
            level,
            click.pos,
            click.hit.face,
        )
    {
        event.cancelled = true;
        return Flow::Cancel;
    }
    Flow::Continue
}

/// `DivinationRodRecipe.assemble`: copy sulfur source into newly crafted rod.
pub fn prepare_item_craft(
    host_ref: &mut dyn ServerHost,
    event: &mut PrepareItemCraftEvent<'_>,
) -> Option<ItemStack> {
    if !event.name.contains("sulfur_attuned_divination_rod") {
        return None;
    }
    // Check player's inventory/crafting grid for sulfur source components if available.
    let player = host_ref.player(event.player)?;
    let result_id = if event.name.contains("abundant") {
        "theurgy:sulfur_attuned_divination_rod_abundant"
    } else if event.name.contains("common") {
        "theurgy:sulfur_attuned_divination_rod_common"
    } else if event.name.contains("rare") {
        "theurgy:sulfur_attuned_divination_rod_rare"
    } else {
        "theurgy:sulfur_attuned_divination_rod_precious"
    };
    let mut rod = host::make_stack(&*host_ref, result_id, 1)?;
    if let Some(slots) = player.inventory_slots(&*host_ref) {
        for (_, stack) in slots {
            if let Some(source) = component_string(&*host_ref, &stack, "theurgy:source_item")
                && let Some(block) = translated_source_block(&*host_ref, &source)
            {
                set_stack_component(
                    &mut rod,
                    DIVINATION_LINKED_BLOCK,
                    Tag::String(block.as_str().into()),
                );
                return Some(rod);
            }
            if let Some(source) = component_string(&*host_ref, &stack, "theurgy:source_tag")
                && let Some(tag) = translated_source_tag(&*host_ref, &source)
            {
                set_stack_component(
                    &mut rod,
                    DIVINATION_LINKED_TAG,
                    Tag::String(tag.as_str().into()),
                );
                return Some(rod);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tooltips (client side)
// ---------------------------------------------------------------------------

const TOOLTIP_GRAY: u32 = 0xFFAA_AAAA;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TooltipParts {
    tooltip: bool,
    extended: bool,
    usage: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HandlerLine {
    Blank,
    Heading(String),
    Body(String),
}

fn lang_has(host_ref: &dyn ClientHost, key: &str) -> bool {
    let text = Text::translate(key, Vec::new());
    host_ref
        .text_string(&text)
        .is_ok_and(|resolved| !resolved.is_empty() && resolved != key)
}

fn description_id(host_ref: &dyn ClientHost, item: &str) -> String {
    let (namespace, path) = item.split_once(':').unwrap_or(("minecraft", item));
    let block = format!("block.{namespace}.{path}");
    if lang_has(host_ref, &block) {
        block
    } else {
        format!("item.{namespace}.{path}")
    }
}

fn tooltip_handler_lines(
    description: &str,
    parts: TooltipParts,
    shift: bool,
    control: bool,
) -> Vec<HandlerLine> {
    let mut out = Vec::new();
    if parts.tooltip {
        out.push(HandlerLine::Body(format!("{description}.tooltip")));
    }
    if parts.extended {
        out.push(HandlerLine::Blank);
        if shift {
            out.push(HandlerLine::Heading(
                "tooltip.theurgy.extended_heading".into(),
            ));
            out.push(HandlerLine::Body(format!("{description}.tooltip.extended")));
        } else {
            out.push(HandlerLine::Heading("tooltip.theurgy.show_extended".into()));
        }
    }
    if parts.usage {
        if !parts.extended || shift {
            out.push(HandlerLine::Blank);
        }
        if control {
            out.push(HandlerLine::Heading("tooltip.theurgy.usage_heading".into()));
            out.push(HandlerLine::Body(format!("{description}.tooltip.usage")));
        } else {
            out.push(HandlerLine::Heading("tooltip.theurgy.show_usage".into()));
        }
    }
    out
}

fn wrap_in_square_brackets(text: Text) -> Text {
    Text::literal("[").append(text).append(Text::literal("]"))
}

fn italic_style(color: &str) -> TextStyle {
    let mut style = TextStyle::default();
    style.color = Some(color.to_owned());
    style.italic = Some(true);
    style
}

fn tooltip_data(host_ref: &dyn ClientHost, item_id: &str, item_key: ItemKey) -> Vec<Text> {
    let sulfur = item_id.starts_with("theurgy:alchemical_sulfur_");
    let niter = item_id.starts_with("theurgy:alchemical_niter_");
    let salt = item_id.starts_with("theurgy:alchemical_salt_");
    let source_key = format!("item.{}.source", item_id.replace(':', "."));
    if salt {
        return vec![Text::translate(source_key, Vec::new()).with_style(italic_style("green"))];
    }
    if !sulfur && !niter {
        return Vec::new();
    }
    let tier = if niter {
        ["abundant", "common", "commond", "rare", "precious"]
            .into_iter()
            .find(|tier| item_id.ends_with(&format!("_{tier}")))
            .map(|tier| if tier == "commond" { "common" } else { tier })
    } else {
        ["abundant", "common", "rare", "precious"]
            .into_iter()
            .find(|tier| {
                host::in_item_tag(host_ref, item_key, &format!("theurgy:alchemical_sulfurs/{tier}"))
            })
    }
    .unwrap_or("abundant");
    let color = match tier {
        "common" => "#41b335",
        "rare" => "#413fa3",
        "precious" => "#d20dd4",
        _ => "#cfcfcf",
    };
    let mut out = vec![
        Text::translate(source_key, Vec::new()).with_style(italic_style(color)),
        wrap_in_square_brackets(
            Text::translate(format!("theurgytheurgy.alchemical_tier.{tier}"), Vec::new())
                .with_style(italic_style(color)),
        ),
    ];
    if sulfur {
        let kind = [
            "animals",
            "crops",
            "earthen_matters",
            "gems",
            "logs",
            "metals",
            "mobs",
            "other_minerals",
        ]
        .into_iter()
        .find(|kind| {
            host::in_item_tag(host_ref, item_key, &format!("theurgy:alchemical_sulfurs/{kind}"))
        })
        .unwrap_or("misc");
        out.push(wrap_in_square_brackets(
            Text::translate(format!("theurgytheurgy.sulfur_type.{kind}"), Vec::new())
                .with_style(italic_style("dark_gray")),
        ));
    }
    out
}

fn divination_rod_tooltip(
    host_ref: &dyn ClientHost,
    stack: &ItemStack,
    lines: &mut Vec<(Text, u32)>,
) {
    let linked = component_string(host_ref, stack, DIVINATION_LINKED_BLOCK)
        .or_else(|| component_string(host_ref, stack, DIVINATION_LINKED_TAG));
    let Some(linked) = linked else {
        lines.push((
            Text::translate("tooltip.theurgy..divination_rod.no_link", Vec::new()),
            u32::MAX,
        ));
        return;
    };
    let target = if let Some((namespace, path)) =
        linked.strip_prefix('#').unwrap_or(&linked).split_once(':')
    {
        if linked.starts_with('#') {
            Text::literal(linked.clone())
        } else {
            Text::translate(format!("block.{namespace}.{path}"), Vec::new())
        }
    } else {
        Text::literal(linked.clone())
    };
    lines.push((
        Text::translate(
            "tooltip.theurgy..divination_rod.linked_to",
            vec![target.clone()],
        ),
        u32::MAX,
    ));
    if let Some(pos) = held_divination_pos(host_ref, stack) {
        let coords = Text::literal(format!("{}, {}, {}", pos[0], pos[1], pos[2]));
        lines.push((
            Text::translate(
                "tooltip.theurgy..divination_rod.last_result",
                vec![target, coords],
            ),
            u32::MAX,
        ));
    }
}

pub fn tooltip(host_ref: &dyn ClientHost, event: &mut TooltipEvent<'_>) {
    let stack = event.stack;
    let item_id = stack.item().id(host_ref);
    if !item_id.starts_with("theurgy:") {
        return;
    }
    if let Some(id) = uuid_text(host_ref, stack) {
        event
            .lines
            .push((Text::literal(format!("Nexus ID: {id}")), u32::MAX));
    }
    if let Some(stored) = component_i32(host_ref, stack, "theurgy:mercury_flux_storage") {
        let capacity = match item_id.as_str() {
            "theurgy:mercury_catalyst" => 50_000,
            "theurgy:mercury_capacitor" => 500_000,
            _ => 1_000,
        };
        event.lines.push((
            Text::literal(format!("Mercury Flux: {stored} / {capacity}")),
            u32::MAX,
        ));
    }
    if item_id.contains("divination_rod") {
        divination_rod_tooltip(host_ref, stack, event.lines);
    }
    let data = tooltip_data(host_ref, &item_id, stack.item());
    let description = description_id(host_ref, &item_id);
    let parts = TooltipParts {
        tooltip: lang_has(host_ref, &format!("{description}.tooltip")),
        extended: lang_has(host_ref, &format!("{description}.tooltip.extended")),
        usage: lang_has(host_ref, &format!("{description}.tooltip.usage")),
    };
    let mods = host_ref.modifiers();
    for line in tooltip_handler_lines(&description, parts, mods.shift(), mods.control()) {
        match line {
            HandlerLine::Blank => event.lines.push((Text::literal(" "), u32::MAX)),
            HandlerLine::Heading(key) => {
                event
                    .lines
                    .push((Text::translate(key, data.clone()), u32::MAX));
            }
            HandlerLine::Body(key) => {
                let text = Text::translate(key, data.clone());
                match host_ref.text_split(&text, 240) {
                    Ok(split) if !split.is_empty() => {
                        for part in split {
                            event.lines.push((part, TOOLTIP_GRAY));
                        }
                    }
                    _ => event.lines.push((text, TOOLTIP_GRAY)),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared ItemPart for Theurgy items
// ---------------------------------------------------------------------------

pub struct TheurgyItemPart;

#[mod_api::horizon_part]
impl ItemPart for TheurgyItemPart {
    fn tooltip(&self, host: &dyn ClientHost, event: &mut TooltipEvent<'_>) {
        tooltip(host, event);
    }

    fn on_block_placed_by(
        &self,
        host: &mut dyn ServerHost,
        event: &mut OnBlockPlacedByEvent<'_>,
    ) {
        on_block_placed_by(host, event);
    }

    fn inventory_tick(&self, host_ref: &mut dyn ServerHost, event: &mut InventoryTickEvent<'_>) {
        selection::inventory_tick(host_ref, event.stack, event.selected);
        let item_id = event.stack.item().id(&*host_ref);
        if (item_id == "theurgy:logistics_nexus_id" || item_id == "theurgy:logistics_nexus")
            && uuid_text(&*host_ref, event.stack).is_none()
            && let Some(player) = host_ref.players().acting_player()
            && let Some(slot) = u32::try_from(event.slot).ok()
        {
            let mut next = event.stack.clone();
            let seed = player.entity().uuid(host_ref).unwrap_or(0) as u64;
            set_stack_component(&mut next, "theurgy:logistics_nexus_id", next_uuid_tag(seed));
            let _ = host_ref.transaction(|host_ref, _tx| {
                player
                    .set_inventory_slot(host_ref, slot, Some(next))
                    .map_err(|_| ())
            });
        }
    }

    fn use_duration(&self, host_ref: &dyn Host, event: &mut UseDurationEvent<'_>) {
        if event.stack.item().id(host_ref).contains("divination_rod") {
            event.duration = component_i32(host_ref, event.stack, DIVINATION_DURATION)
                .unwrap_or(40)
                .max(0);
        }
    }

    fn use_animation(&self, host_ref: &dyn ClientHost, event: &mut UseAnimationEvent<'_>) {
        if event.stack.item().id(host_ref).contains("divination_rod") {
            *event.animation = UseAnimation::None;
        }
    }

    fn release_using(&self, host_side: HostSide<'_>, event: &mut ReleaseUsingEvent<'_>) {
        let HostSide::Server(host_ref) = host_side else {
            return;
        };
        if !event.stack.item().id(&*host_ref).contains("divination_rod") {
            return;
        }
        let Some(player) = host_ref.players().acting_player() else {
            return;
        };
        let Some(player_id) = player.entity().uuid(host_ref) else {
            return;
        };
        let origin = player.entity().position(&*host_ref).unwrap_or([0.0; 3]);
        let distance = held_divination_pos(&*host_ref, event.stack).map_or(
            DIVINATION_NOT_FOUND,
            |pos| {
                let dx = f64::from(pos[0]) + 0.5 - origin[0];
                let dz = f64::from(pos[2]) + 0.5 - origin[2];
                distance_bucket((dx * dx + dz * dz).sqrt() as f32)
            },
        );
        set_held_component(
            host_ref,
            player_id,
            Hand::Main,
            DIVINATION_DISTANCE,
            Tag::Float(distance.to_bits()),
        );
        event.used = false;
    }

    fn use_item(
        &self,
        host_side: HostSide<'_>,
        event: &mut UseItemEvent<'_>,
    ) -> Option<InteractionResult> {
        let HostSide::Server(host_ref) = host_side else {
            return None;
        };
        let item_id = event.stack.item().id(&*host_ref);
        let sneaking = host_ref
            .player(event.player)
            .is_some_and(|p| p.is_secondary_use_active(&*host_ref).unwrap_or(false));
        if item_id == MERCURIAL_WAND && wand_mode(&*host_ref, event.stack) == 2 {
            let freq = component_i32(&*host_ref, event.stack, SELECTED_FREQUENCY).unwrap_or(0) + 1;
            set_held_component(
                host_ref,
                event.player,
                event.hand,
                SELECTED_FREQUENCY,
                Tag::Int(freq),
            );
            return Some(InteractionResult::Success);
        }
        if item_id == WIRE_ITEM {
            if sneaking {
                clear_held_component(host_ref, event.player, event.hand, WIRE_END_POINT);
                return Some(InteractionResult::Success);
            }
            return None;
        }
        if !sneaking && item_id.contains("divination_rod") {
            let linked = component_string(&*host_ref, event.stack, DIVINATION_LINKED_BLOCK)
                .is_some()
                || component_string(&*host_ref, event.stack, DIVINATION_LINKED_TAG).is_some();
            if !linked {
                if let Some(player) = host_ref.player(event.player) {
                    let _ = player.action_bar(
                        host_ref,
                        Text::translate("message.theurgy.divination_rod.no_link", Vec::new()),
                    );
                }
                return Some(InteractionResult::Success);
            }
            set_held_component(
                host_ref,
                event.player,
                event.hand,
                DIVINATION_DISTANCE,
                Tag::Float(DIVINATION_SEARCHING.to_bits()),
            );
            if let Some(player) = host_ref.player(event.player)
                && let Some(level) = player.level(&*host_ref)
            {
                let at = player.entity().position(&*host_ref).unwrap_or([0.0; 3]);
                let _ = level.play_sound(
                    host_ref,
                    "theurgy:tuning_fork",
                    at,
                    SoundSource::Players,
                    1.0,
                    1.0,
                );
            }
            // Leave unclaimed so the timed use starts and invokes finish_using.
            return None;
        }
        None
    }

    fn use_on(
        &self,
        host_side: HostSide<'_>,
        event: &mut UseOnEvent<'_>,
    ) -> Option<InteractionResult> {
        let HostSide::Server(host_ref) = host_side else {
            return None;
        };
        let item_id = event.stack.item().id(&*host_ref);
        let level = host_ref.player(event.player)?.level(&*host_ref)?;
        if item_id == MERCURIAL_WAND
            && wand_use_on(
                host_ref,
                event.stack,
                event.player,
                event.hand,
                level,
                event.pos,
                event.face,
            )
        {
            return Some(InteractionResult::Success);
        }
        if item_id.contains("divination_rod")
            && let Some(res) = attune_rod(host_ref, event.stack, event.player, event.hand, event.pos)
        {
            return Some(res);
        }
        if is_double_high(&item_id) {
            let placement = [
                event.pos[0] + event.face[0],
                event.pos[1] + event.face[1],
                event.pos[2] + event.face[2],
            ];
            let above = [placement[0], placement[1] + 1, placement[2]];
            if let Some(above_block) = host::block_id_at(&*host_ref, level, above)
                && !is_air_name(&above_block)
            {
                return Some(InteractionResult::Fail);
            }
        }
        None
    }
}

pub fn bucket_remainder(item: &str) -> Option<&'static str> {
    (item == "theurgy:sal_ammoniac_bucket").then_some("minecraft:bucket")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachments_follow_java_can_survive() {
        assert_eq!(
            attachment_survives("theurgy:logistics_fluid_inserter", true, false),
            Some(false)
        );
        assert_eq!(
            attachment_survives("theurgy:logistics_fluid_inserter", false, false),
            Some(true)
        );
        assert_eq!(
            attachment_survives("theurgy:logistics_mercury_flux_connector", true, true),
            Some(false)
        );
        assert_eq!(
            attachment_survives("theurgy:caloric_flux_emitter", false, false),
            Some(false)
        );
        assert_eq!(
            attachment_survives("theurgy:sulfuric_flux_emitter", false, true),
            Some(true)
        );
        assert_eq!(
            attachment_survives("theurgy:calcination_oven", true, false),
            None
        );
        assert!(is_air_name("minecraft:cave_air") && !is_air_name("minecraft:stone"));
    }

    #[test]
    fn double_high_set_and_bucket_remainder() {
        for id in DOUBLE_HIGH {
            assert!(is_double_high(id), "{id}");
        }
        assert!(!is_double_high("minecraft:furnace"));
        assert_eq!(
            bucket_remainder("theurgy:sal_ammoniac_bucket"),
            Some("minecraft:bucket")
        );
        assert_eq!(bucket_remainder("minecraft:water_bucket"), None);
    }

    #[test]
    fn block_pos_packing_round_trips_negative_and_positive() {
        for pos in [[0, 0, 0], [123, -45, -678], [-100_000, 319, 100_000]] {
            assert_eq!(block_pos_from_long(block_pos_long(pos)), pos);
        }
        assert_eq!(manhattan([1, 2, 3], [4, -1, 3]), 6);
    }

    #[test]
    fn divination_ore_tags_and_tiers() {
        assert_eq!(
            ore_tag_candidates("theurgy:sal_ammoniac_ore"),
            vec!["c:ores/sal_ammoniac", "c:ores/ammoniac"]
        );
        assert_eq!(
            ore_tag_candidates("minecraft:deepslate_iron_ore"),
            vec!["c:ores/deepslate_iron", "c:ores/iron"]
        );
        assert_eq!(ore_tag_candidates("minecraft:stone"), vec!["c:ores/stone"]);
        assert_eq!(
            deepslate_variant("minecraft:iron_ore").as_deref(),
            Some("minecraft:deepslate_iron_ore")
        );
        assert_eq!(deepslate_variant("minecraft:deepslate_iron_ore"), None);
        assert_eq!(
            incorrect_blocks_tag("diamond"),
            "minecraft:incorrect_for_diamond_tool"
        );
        assert_eq!(distance_bucket(5.0), 0.0);
        assert_eq!(distance_bucket(10.0), 1.0);
        assert_eq!(distance_bucket(70.0), 6.0);
    }

    #[test]
    fn tooltip_handler_sequence_matches_java() {
        let parts = TooltipParts {
            tooltip: true,
            extended: true,
            usage: true,
        };
        let collapsed = tooltip_handler_lines("block.theurgy.oven", parts, false, false);
        assert_eq!(
            collapsed,
            vec![
                HandlerLine::Body("block.theurgy.oven.tooltip".into()),
                HandlerLine::Blank,
                HandlerLine::Heading("tooltip.theurgy.show_extended".into()),
                HandlerLine::Heading("tooltip.theurgy.show_usage".into()),
            ]
        );
        let shifted = tooltip_handler_lines("block.theurgy.oven", parts, true, true);
        assert_eq!(shifted.len(), 7);
    }
}
