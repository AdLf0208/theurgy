// SPDX-License-Identifier: MIT
//! Logistics networks: wire endpoints, nexus pairing, item/fluid/mercury-flux
//! connectors, capability probes & proxies, and transactional filtered transfers
//! built on `mod_api::ITEMS`, `mod_api::FLUIDS`, and `mod_api::move_resources`.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock, RwLock};

use mod_api::{
    Compound, FLUIDS, FluidResource, FluidStack, Hand, Host, HostTransaction, ITEMS, ItemPlace,
    ItemResource, ItemStack, LevelRef, ModBlockEntityView, ServerHost, Tag, move_resources,
};

use crate::{filter, flux, host, items};

pub type Pos = [i32; 3];

pub const ITEM_EXTRACTION_EVERY_N_TICKS: i32 = 20;
pub const FLUID_EXTRACTION_EVERY_N_TICKS: i32 = 20;
pub const FLUX_TRANSFER_EVERY_N_TICKS: i32 = 20;
pub const MAX_ITEM_EXTRACTION_AMOUNT: i32 = 64;
pub const MAX_FLUID_EXTRACTION_AMOUNT: i32 = 5_000;
pub const FLUX_BUFFER_CAPACITY: i32 = 1_000;
pub const FLUX_DEFAULT_TRANSFER_RATE: i32 = 100;
pub const FLUX_TRANSFER_RATE: i32 = FLUX_DEFAULT_TRANSFER_RATE;

pub const WIRE_CONNECTIONS: &str = "wireConnections";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafNodeMode {
    Insert,
    Extract,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityKind {
    Item,
    Fluid,
    MercuryFlux,
}

impl CapabilityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CapabilityKind::Item => "item",
            CapabilityKind::Fluid => "fluid",
            CapabilityKind::MercuryFlux => "mercury_flux",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeafNode {
    pub pos: Pos,
    pub mode: LeafNodeMode,
    pub kind: CapabilityKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogisticsState {
    pub enabled: bool,
    pub direction_override: Option<u8>,
    pub extraction_amount: i32,
    pub flux_buffer: i32,
}

impl LogisticsState {
    pub const DEFAULT_ENABLED: bool = true;

    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            direction_override: None,
            extraction_amount: 0,
            flux_buffer: 0,
        }
    }
}

/// Live server-side index of loaded logistics block entities per dimension.
/// Updated on tick/load/wire/remove and cleared on server start/stop so
/// connectors never need raw world-map scans.
#[derive(Clone, Debug)]
struct IndexedNode {
    block_id: String,
    nexus_id: Option<String>,
    frequency: i32,
    enabled: bool,
    wires: Vec<(Pos, Pos)>,
}

static NODES: OnceLock<RwLock<HashMap<String, HashMap<Pos, IndexedNode>>>> = OnceLock::new();
static LAST_FILTER_ACTION: Mutex<Option<(u128, Pos, i64)>> = Mutex::new(None);
static LAST_WIRE_ACTION: Mutex<Option<(u128, Pos, i64)>> = Mutex::new(None);

fn nodes_map() -> &'static RwLock<HashMap<String, HashMap<Pos, IndexedNode>>> {
    NODES.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn clear() {
    if let Ok(mut guard) = nodes_map().write() {
        guard.clear();
    }
}

pub fn forget_node(dim: &str, pos: Pos) {
    if let Ok(mut guard) = nodes_map().write()
        && let Some(dim_map) = guard.get_mut(dim)
    {
        dim_map.remove(&pos);
    }
}

fn record_node(dim: &str, pos: Pos, block_id: &str, nbt: &Compound) {
    let state = read_state(nbt);
    let node = IndexedNode {
        block_id: block_id.to_owned(),
        nexus_id: java_nexus_id(nbt),
        frequency: host::int(nbt, "frequency", 0),
        enabled: state.enabled,
        wires: wire_records(nbt),
    };
    if let Ok(mut guard) = nodes_map().write() {
        guard.entry(dim.to_owned()).or_default().insert(pos, node);
    }
}

// ---------------------------------------------------------------------------
// Pure NBT & network math
// ---------------------------------------------------------------------------

pub fn read_state(nbt: &Compound) -> LogisticsState {
    let enabled = nbt
        .get(&"enabled".into())
        .map(|_| host::bool(nbt, "enabled"))
        .unwrap_or(LogisticsState::DEFAULT_ENABLED);
    let direction_override = match nbt.get(&"directionOverride".into()) {
        Some(Tag::Int(v)) if (0..6).contains(v) => Some(*v as u8),
        Some(Tag::Byte(v)) if (0..6).contains(v) => Some(*v as u8),
        Some(Tag::Short(v)) if (0..6).contains(v) => Some(*v as u8),
        _ => None,
    };
    let from_handler = match nbt.get(&"mercuryFluxHandler".into()) {
        Some(Tag::Compound(child)) => Some(host::int(child, "energy", 0)),
        _ => None,
    };
    let from_buffer = match nbt.get(&"buffer".into()) {
        Some(Tag::Compound(child)) => Some(host::int(child, "energy", 0)),
        _ => None,
    };
    let flux_buffer = match (from_handler, from_buffer) {
        (Some(h), Some(b)) => h.max(b),
        (Some(h), None) => h,
        (None, Some(b)) => b,
        (None, None) => host::int(nbt, "fluxBuffer", 0),
    };
    LogisticsState {
        enabled,
        direction_override,
        extraction_amount: host::int(nbt, "extractionAmount", 0),
        flux_buffer,
    }
}

pub fn write_state(nbt: &mut Compound, state: &LogisticsState) {
    host::set_int(nbt, "enabled", i32::from(state.enabled));
    match state.direction_override {
        Some(v) => host::set_int(nbt, "directionOverride", i32::from(v)),
        None => {
            nbt.remove(&"directionOverride".into());
        }
    }
    host::set_int(nbt, "extractionAmount", state.extraction_amount);
    let mut buffer = Compound::new();
    host::set_int(
        &mut buffer,
        "energy",
        state.flux_buffer.clamp(0, FLUX_BUFFER_CAPACITY),
    );
    nbt.insert("buffer".into(), Tag::Compound(buffer.clone()));
    nbt.insert("mercuryFluxHandler".into(), Tag::Compound(buffer));
    nbt.remove(&"fluxBuffer".into());
}

pub fn leaf_nodes_of(
    nexus_id: &str,
    candidates: &[(Pos, &str, LeafNodeMode, CapabilityKind)],
) -> Vec<LeafNode> {
    candidates
        .iter()
        .filter(|(_, candidate_nexus, _, _)| *candidate_nexus == nexus_id)
        .map(|(pos, _, mode, kind)| LeafNode {
            pos: *pos,
            mode: *mode,
            kind: *kind,
        })
        .collect()
}

pub fn insert_targets<'a>(
    nodes: &'a [LeafNode],
    extractor: Pos,
    kind: CapabilityKind,
) -> Vec<&'a LeafNode> {
    nodes
        .iter()
        .filter(|n| n.kind == kind && n.mode == LeafNodeMode::Insert && n.pos != extractor)
        .collect()
}

pub fn on_slow_tick(offset: i32, game_time: i64, period: i32) -> bool {
    if period <= 0 {
        return false;
    }
    let shifted = game_time.rem_euclid(i64::from(period)) as i32;
    (offset + shifted).rem_euclid(period) == 0
}

pub fn slow_tick_offset(pos: Pos, period: i32) -> i32 {
    if period <= 0 {
        return 0;
    }
    let h = i64::from(pos[0]).wrapping_mul(73_856_093)
        ^ i64::from(pos[1]).wrapping_mul(19_349_663)
        ^ i64::from(pos[2]).wrapping_mul(83_492_791);
    h.rem_euclid(i64::from(period)) as i32
}

pub fn distribute_evenly(total: i32, sink_count: usize) -> Vec<i32> {
    if sink_count == 0 || total <= 0 {
        return Vec::new();
    }
    let n = sink_count as i32;
    let per = total / n;
    let rem = total % n;
    (0..sink_count)
        .map(|i| per + i32::from((i as i32) < rem))
        .collect()
}

pub fn flux_pushes_for(buffer: i32, sink_count: usize) -> i32 {
    if buffer <= 0 || sink_count == 0 {
        return 0;
    }
    buffer.min(FLUX_TRANSFER_RATE.saturating_mul(sink_count as i32))
}

pub fn connector_direction(override_dir: Option<u8>, facing: u8) -> u8 {
    override_dir.unwrap_or(facing)
}

pub fn wire_pos_long(pos: Pos) -> i64 {
    items::block_pos_long(pos)
}

pub fn wire_pos_from_long(value: i64) -> Pos {
    items::block_pos_from_long(value)
}

pub fn wire_records(nbt: &Compound) -> Vec<(Pos, Pos)> {
    let Some(Tag::List(list)) = nbt.get(&WIRE_CONNECTIONS.into()) else {
        return Vec::new();
    };
    list.iter()
        .take(256)
        .filter_map(|tag| {
            let Tag::Compound(record) = tag else {
                return None;
            };
            let from = match record.get(&"from".into()) {
                Some(Tag::Long(v)) => wire_pos_from_long(*v),
                _ => return None,
            };
            let to = match record.get(&"to".into()) {
                Some(Tag::Long(v)) => wire_pos_from_long(*v),
                _ => return None,
            };
            Some((from, to))
        })
        .collect()
}

pub fn same_wire(edge: (Pos, Pos), a: Pos, b: Pos) -> bool {
    (edge.0 == a && edge.1 == b) || (edge.0 == b && edge.1 == a)
}

pub fn reachable_nodes(start: Pos, edges: &[(Pos, Pos)]) -> Vec<Pos> {
    let mut reached = BTreeSet::from([start]);
    for _ in 0..edges.len() {
        let before = reached.len();
        for (a, b) in edges {
            if reached.contains(a) {
                reached.insert(*b);
            }
            if reached.contains(b) {
                reached.insert(*a);
            }
        }
        if reached.len() == before {
            break;
        }
    }
    reached.into_iter().collect()
}

pub fn next_wire_count(count: u32, delta: i32, creative: bool) -> u32 {
    if creative {
        return count;
    }
    if delta < 0 {
        count.saturating_sub(1)
    } else {
        count.saturating_add(1).min(64)
    }
}

pub fn repeated_wire_action(last: Option<(u128, Pos, i64)>, action: (u128, Pos, i64)) -> bool {
    last.is_some_and(|(player, pos, tick)| {
        player == action.0 && pos == action.1 && matches!(action.2.checked_sub(tick), Some(0..=4))
    })
}

pub fn block_kind(block: &str) -> Option<(LeafNodeMode, CapabilityKind)> {
    match block {
        "theurgy:logistics_item_inserter" => Some((LeafNodeMode::Insert, CapabilityKind::Item)),
        "theurgy:logistics_item_extractor" => Some((LeafNodeMode::Extract, CapabilityKind::Item)),
        "theurgy:logistics_fluid_inserter" => Some((LeafNodeMode::Insert, CapabilityKind::Fluid)),
        "theurgy:logistics_fluid_extractor" => Some((LeafNodeMode::Extract, CapabilityKind::Fluid)),
        "theurgy:logistics_mercury_flux_connector" => {
            Some((LeafNodeMode::Insert, CapabilityKind::MercuryFlux))
        }
        "theurgy:logistics_capability_proxy" => Some((LeafNodeMode::Insert, CapabilityKind::Item)),
        _ => None,
    }
}

fn is_connector(block: &str) -> bool {
    matches!(
        block,
        "theurgy:logistics_item_inserter"
            | "theurgy:logistics_item_extractor"
            | "theurgy:logistics_fluid_inserter"
            | "theurgy:logistics_fluid_extractor"
            | "theurgy:logistics_mercury_flux_connector"
            | "theurgy:logistics_capability_proxy"
    )
}

fn is_nexus(block: &str) -> bool {
    block == "theurgy:logistics_nexus"
}

fn java_nexus_id(nbt: &Compound) -> Option<String> {
    host::string(nbt, "nexusId")
}

fn fresh_nexus_id(pos: Pos, id: &str) -> String {
    let mut hasher = 0u64;
    for b in id.bytes() {
        hasher = hasher.wrapping_mul(1_000_003).wrapping_add(u64::from(b));
    }
    hasher ^= (i64::from(pos[0]) as u64).rotate_left(11);
    hasher ^= (i64::from(pos[1]) as u64).rotate_left(27);
    hasher ^= (i64::from(pos[2]) as u64).rotate_left(43);
    format!("{hasher:016x}")
}

fn attached(pos: Pos, facing: u8) -> Pos {
    host::offset_pos(pos, host::opposite_dir(facing))
}

fn facing_of(host_ref: &dyn Host, level: LevelRef, pos: Pos) -> u8 {
    level
        .block_property(host_ref, pos, "facing")
        .as_deref()
        .and_then(host::facing_name_to_dir)
        .unwrap_or(0)
}

fn saved_filter(host_ref: &dyn Host, nbt: &Compound) -> Option<ItemStack> {
    let stack = filter::stack_from_template(host_ref, nbt.get(&"filter".into())?)?;
    let id = stack.item().id(host_ref);
    matches!(
        id.as_str(),
        "theurgy:list_filter" | "theurgy:attribute_filter"
    )
    .then_some(stack)
}

fn filter_block(block: &str) -> bool {
    matches!(
        block,
        "theurgy:logistics_item_inserter"
            | "theurgy:logistics_item_extractor"
            | "theurgy:logistics_fluid_inserter"
            | "theurgy:logistics_fluid_extractor"
    )
}

// ---------------------------------------------------------------------------
// Wire & Filter interactions
// ---------------------------------------------------------------------------

fn load_wire_endpoint(host_ref: &dyn Host, stack: &ItemStack) -> Option<(Pos, String)> {
    let Tag::Compound(entries) = items::component_tag(host_ref, stack, items::WIRE_END_POINT)?
    else {
        return None;
    };
    let pos = match entries.get(&"pos".into()) {
        Some(Tag::Long(v)) => wire_pos_from_long(*v),
        _ => return None,
    };
    let level = host::string(&entries, "level")?;
    Some((pos, level))
}

fn find_wire(dim: &str, a: Pos, b: Pos) -> Option<Pos> {
    let guard = nodes_map().read().ok()?;
    let dim_map = guard.get(dim)?;
    for (&owner, node) in dim_map {
        if node.wires.iter().any(|edge| same_wire(*edge, a, b)) {
            return Some(owner);
        }
    }
    None
}

fn add_wire(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    dim: &str,
    owner: Pos,
    from: Pos,
    to: Pos,
) -> bool {
    let Some(be) = host_ref.block_entity(level, owner) else {
        return false;
    };
    let mut nbt = be.data(&*host_ref).unwrap_or_default();
    let mut records = match nbt.get(&WIRE_CONNECTIONS.into()) {
        Some(Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    };
    let mut record = Compound::new();
    record.insert("from".into(), Tag::Long(wire_pos_long(from)));
    record.insert("to".into(), Tag::Long(wire_pos_long(to)));
    records.push(Tag::Compound(record));
    nbt.insert(WIRE_CONNECTIONS.into(), Tag::List(records));
    let ok = be.merge_data(host_ref, nbt.clone()).unwrap_or(false);
    if ok {
        let block_id = host::block_id_at(&*host_ref, level, owner).unwrap_or_default();
        record_node(dim, owner, &block_id, &nbt);
        host_ref.block_entity_sync(owner);
    }
    ok
}

fn remove_wire(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    dim: &str,
    owner: Pos,
    a: Pos,
    b: Pos,
) -> bool {
    let Some(be) = host_ref.block_entity(level, owner) else {
        return false;
    };
    let mut nbt = be.data(&*host_ref).unwrap_or_default();
    let mut records = match nbt.get(&WIRE_CONNECTIONS.into()) {
        Some(Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    };
    let before = records.len();
    records.retain(|tag| {
        let Tag::Compound(record) = tag else {
            return true;
        };
        let from = match record.get(&"from".into()) {
            Some(Tag::Long(v)) => wire_pos_from_long(*v),
            _ => return true,
        };
        let to = match record.get(&"to".into()) {
            Some(Tag::Long(v)) => wire_pos_from_long(*v),
            _ => return true,
        };
        !same_wire((from, to), a, b)
    });
    if records.len() == before {
        return false;
    }
    if records.is_empty() {
        nbt.remove(&WIRE_CONNECTIONS.into());
    } else {
        nbt.insert(WIRE_CONNECTIONS.into(), Tag::List(records));
    }
    let ok = be.replace_data(host_ref, nbt.clone()).unwrap_or(false);
    if ok {
        let block_id = host::block_id_at(&*host_ref, level, owner).unwrap_or_default();
        record_node(dim, owner, &block_id, &nbt);
        host_ref.block_entity_sync(owner);
    }
    ok
}

fn adjust_wire_count(host_ref: &mut dyn ServerHost, player_id: u128, hand: Hand, delta: i32) {
    let creative = host_ref
        .player(player_id)
        .is_some_and(|p| p.is_creative(&*host_ref).unwrap_or(false));
    if creative {
        return;
    }
    let place = ItemPlace::PlayerHand {
        player: player_id,
        hand,
    };
    let Some(mut held) = place.stack(host_ref) else {
        return;
    };
    if held.item().id(&*host_ref) != items::WIRE_ITEM {
        return;
    }
    let next = next_wire_count(held.count(), delta, false);
    let Some(player) = host_ref.player(player_id) else {
        return;
    };
    let slot = match hand {
        Hand::Main => player.selected_slot(&*host_ref).unwrap_or(0) as u32,
        Hand::Off => 40,
    };
    let replacement = if next == 0 {
        None
    } else {
        let _ = held.set_count(next);
        Some(held)
    };
    let _ = host_ref.transaction(|host_ref, _tx| {
        player
            .set_inventory_slot(host_ref, slot, replacement)
            .map_err(|_| ())
    });
}

/// `WireItem.useOn`: select, connect, or disconnect an unordered copper wire edge.
pub fn wire_use_on(
    host_ref: &mut dyn ServerHost,
    player_id: u128,
    hand: Hand,
    level: LevelRef,
    dim: &str,
    stack: &ItemStack,
    pos: Pos,
) -> bool {
    let now = level.game_time(&*host_ref).unwrap_or(0);
    let action = (player_id, pos, now);
    if let Ok(mut last) = LAST_WIRE_ACTION.lock() {
        if repeated_wire_action(*last, action) {
            return true;
        }
        *last = Some(action);
    }
    let sneaking = host_ref
        .player(player_id)
        .is_some_and(|p| p.is_secondary_use_active(&*host_ref).unwrap_or(false));
    let block = host::block_id_at(&*host_ref, level, pos).unwrap_or_default();
    if !items::is_wire_endpoint(&block) || sneaking {
        if sneaking {
            items::clear_held_component(host_ref, player_id, hand, items::WIRE_END_POINT);
        }
        return true;
    }
    let Some((stored, stored_dim)) = load_wire_endpoint(&*host_ref, stack) else {
        let mut point = Compound::new();
        point.insert("pos".into(), Tag::Long(wire_pos_long(pos)));
        point.insert("level".into(), Tag::String(dim.into()));
        items::set_held_component(
            host_ref,
            player_id,
            hand,
            items::WIRE_END_POINT,
            Tag::Compound(point),
        );
        return true;
    };
    if stored_dim != dim || stored == pos || items::manhattan(stored, pos) > items::WIRE_MAX_RANGE
    {
        return true;
    }
    if let Some(owner) = find_wire(dim, stored, pos) {
        if remove_wire(host_ref, level, dim, owner, stored, pos) {
            items::clear_held_component(host_ref, player_id, hand, items::WIRE_END_POINT);
            adjust_wire_count(host_ref, player_id, hand, 1);
        }
    } else if add_wire(host_ref, level, dim, pos, stored, pos) {
        items::clear_held_component(host_ref, player_id, hand, items::WIRE_END_POINT);
        adjust_wire_count(host_ref, player_id, hand, -1);
    }
    true
}

/// `FilterBehaviour.useItemOn`: insert a filter into a connector or sneak-take it back.
pub fn filter_use_on(
    host_ref: &mut dyn ServerHost,
    player_id: u128,
    hand: Hand,
    level: LevelRef,
    pos: Pos,
    held: Option<&ItemStack>,
) -> bool {
    let block = host::block_id_at(&*host_ref, level, pos).unwrap_or_default();
    if !filter_block(&block) {
        return false;
    }
    let now = level.game_time(&*host_ref).unwrap_or(0);
    let action = (player_id, pos, now);
    if let Ok(mut last) = LAST_FILTER_ACTION.lock() {
        if last.is_some_and(|(p, at, t)| p == action.0 && at == action.1 && action.2 - t <= 1) {
            return true;
        }
        *last = Some(action);
    }
    let Some(be) = host_ref.block_entity(level, pos) else {
        return false;
    };
    let mut nbt = be.data(&*host_ref).unwrap_or_default();
    let held = held.filter(|s| s.count() > 0);

    if let Some(existing) = saved_filter(&*host_ref, &nbt) {
        let sneaking = host_ref
            .player(player_id)
            .is_some_and(|p| p.is_secondary_use_active(&*host_ref).unwrap_or(false));
        if !sneaking || held.is_some() {
            return false;
        }
        let given = host_ref.transaction(|host_ref, _tx| {
            let player = host_ref.player(player_id).ok_or(())?;
            player.give_stack(host_ref, existing.clone()).map_err(|_| ())
        });
        if given.is_ok() {
            nbt.remove(&"filter".into());
            let _ = be.replace_data(host_ref, nbt);
            host_ref.set_block_property_with(pos, "has_filter", "false", 2);
            host_ref.block_entity_sync(pos);
        }
        return true;
    }

    let Some(stack) = held else {
        return false;
    };
    let id = stack.item().id(&*host_ref);
    if !matches!(
        id.as_str(),
        "theurgy:list_filter" | "theurgy:attribute_filter"
    ) {
        return false;
    }
    let template = filter::stack_to_template(&*host_ref, stack);
    let Some(player) = host_ref.player(player_id) else {
        return false;
    };
    let slot = match hand {
        Hand::Main => player.selected_slot(&*host_ref).unwrap_or(0) as u32,
        Hand::Off => 40,
    };
    let remainder = if stack.count() > 1 {
        let mut rem = stack.clone();
        let _ = rem.set_count(stack.count() - 1);
        Some(rem)
    } else {
        None
    };
    let moved = host_ref
        .transaction(|host_ref, _tx| {
            player
                .set_inventory_slot(host_ref, slot, remainder)
                .map_err(|_| ())
        })
        .is_ok();
    if moved {
        nbt.insert("filter".into(), template);
        let _ = be.merge_data(host_ref, nbt);
        host_ref.set_block_property_with(pos, "has_filter", "true", 2);
        host_ref.block_entity_sync(pos);
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Network queries & filtered transactional transfers
// ---------------------------------------------------------------------------

fn wire_network(host_ref: &dyn Host, level: LevelRef, dim: &str, start: Pos, own_nbt: &Compound) -> Vec<Pos> {
    let mut edges = wire_records(own_nbt);
    if let Ok(guard) = nodes_map().read()
        && let Some(dim_map) = guard.get(dim)
    {
        for node in dim_map.values() {
            edges.extend(node.wires.iter().copied());
        }
    }
    edges.retain(|(a, b)| {
        host::block_id_at(host_ref, level, *a)
            .is_some_and(|id| items::is_wire_endpoint(&id))
            && host::block_id_at(host_ref, level, *b)
                .is_some_and(|id| items::is_wire_endpoint(&id))
    });
    reachable_nodes(start, &edges)
}

fn collect_network(
    host_ref: &dyn Host,
    level: LevelRef,
    dim: &str,
    nexus_id: Option<&str>,
    wired: &[Pos],
    frequency: i32,
    self_pos: Pos,
) -> Vec<(Pos, LeafNodeMode, CapabilityKind, u8, Option<ItemStack>)> {
    let snapshot: Vec<(Pos, IndexedNode)> = nodes_map()
        .read()
        .ok()
        .and_then(|m| m.get(dim).map(|d| d.iter().map(|(&p, n)| (p, n.clone())).collect()))
        .unwrap_or_default();

    let mut out = Vec::new();
    for (p, node) in snapshot {
        if p == self_pos || node.block_id == "theurgy:logistics_capability_proxy" {
            continue;
        }
        let Some((mode, kind)) = block_kind(&node.block_id) else {
            continue;
        };
        let connected = wired.contains(&p)
            || (nexus_id.is_some() && node.nexus_id.as_deref() == nexus_id);
        if !connected || node.frequency != frequency || !node.enabled {
            continue;
        }
        let Some(be) = host_ref.block_entity(level, p) else {
            continue;
        };
        let nbt = be.data(host_ref).unwrap_or_default();
        let state = read_state(&nbt);
        if !state.enabled {
            continue;
        }
        let facing = facing_of(host_ref, level, p);
        let dir = connector_direction(state.direction_override, facing);
        out.push((p, mode, kind, dir, saved_filter(host_ref, &nbt)));
    }
    out
}

fn connected_probes(dim: &str, nexus_id: Option<&str>, wired: &[Pos]) -> Vec<Pos> {
    let mut probes = Vec::new();
    if let Ok(guard) = nodes_map().read()
        && let Some(dim_map) = guard.get(dim)
    {
        for (&p, node) in dim_map {
            if node.block_id != "theurgy:logistics_capability_probe" {
                continue;
            }
            let connected = wired.contains(&p)
                || (nexus_id.is_some() && node.nexus_id.as_deref() == nexus_id);
            if connected {
                probes.push(p);
            }
        }
    }
    probes.sort_by_key(|p| wire_pos_long(*p));
    probes
}

/// Transactional filtered item transfer via `ITEMS` capability + `move_resources`.
fn transfer_items_filtered(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    source: Pos,
    from_dir: u8,
    sink: Pos,
    to_dir: u8,
    source_filter: Option<&ItemStack>,
    sink_filter: Option<&ItemStack>,
    max: u32,
) -> u32 {
    if source == sink || max == 0 {
        return 0;
    }
    let from_face = host::ordinal_to_direction(from_dir);
    let to_face = host::ordinal_to_direction(to_dir);
    let Some(src_cap) = level.capability(host_ref, &ITEMS, source, Some(from_face)) else {
        return 0;
    };
    let Some(dst_cap) = level.capability(host_ref, &ITEMS, sink, Some(to_face)) else {
        return 0;
    };
    // Pre-evaluate filter predicates against the distinct stacks currently in `src_cap`.
    let size = src_cap.size(host_ref);
    let mut allowed: Vec<ItemStack> = Vec::new();
    for slot in 0..size {
        let Some(res) = src_cap.resource(host_ref, slot) else {
            continue;
        };
        let stack = res.as_stack();
        if allowed.iter().any(|s| res.matches(s)) {
            continue;
        }
        if filter::allows_item(&*host_ref, source_filter, stack)
            && filter::allows_item(&*host_ref, sink_filter, stack)
        {
            allowed.push(stack.clone());
        }
    }
    if allowed.is_empty() {
        return 0;
    }
    host_ref
        .transaction(|host_ref, tx| {
            move_resources(
                host_ref,
                &src_cap,
                &dst_cap,
                |res: &ItemResource| allowed.iter().any(|s| res.matches(s)),
                max,
                tx,
            )
            .map_err(|_| ())
        })
        .unwrap_or(0)
}

/// Transactional filtered fluid transfer via `FLUIDS` capability + `move_resources`.
fn transfer_fluids_filtered(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    source: Pos,
    from_dir: u8,
    sink: Pos,
    to_dir: u8,
    source_filter: Option<&ItemStack>,
    sink_filter: Option<&ItemStack>,
    max: u32,
) -> u32 {
    if source == sink || max == 0 {
        return 0;
    }
    let to_face = host::ordinal_to_direction(to_dir);
    let Some(dst_cap) = level.capability(host_ref, &FLUIDS, sink, Some(to_face)) else {
        return 0;
    };

    // Infinite world fluid source blocks (water, lava, sal ammoniac).
    let world_fluid = host::block_id_at(&*host_ref, level, source).and_then(|block| match block
        .as_str()
    {
        "minecraft:water" | "minecraft:lava" | "theurgy:sal_ammoniac" => Some(block),
        _ => None,
    });
    if let Some(fluid_id) = world_fluid {
        if !filter::allows_fluid(&*host_ref, source_filter, &fluid_id)
            || !filter::allows_fluid(&*host_ref, sink_filter, &fluid_id)
        {
            return 0;
        }
        let Some(fluid_key) = host_ref.fluid_key(&fluid_id) else {
            return 0;
        };
        let resource = FluidResource::of(&FluidStack::new(fluid_key, 1));
        return host_ref
            .transaction(|host_ref, tx| {
                let mut total = 0u32;
                let mut rem = max;
                for idx in 0..dst_cap.size(host_ref) {
                    if rem == 0 {
                        break;
                    }
                    let added = dst_cap.insert(host_ref, tx, idx, &resource, rem);
                    total += added;
                    rem -= added;
                }
                Ok::<u32, ()>(total)
            })
            .unwrap_or(0);
    }

    let from_face = host::ordinal_to_direction(from_dir);
    let Some(src_cap) = level.capability(host_ref, &FLUIDS, source, Some(from_face)) else {
        return 0;
    };
    let size = src_cap.size(host_ref);
    let mut allowed: Vec<FluidResource> = Vec::new();
    for idx in 0..size {
        let Some(res) = src_cap.resource(host_ref, idx) else {
            continue;
        };
        let Some(fluid_id) = res.fluid().key().map(|k| k.id(&*host_ref)) else {
            continue;
        };
        if filter::allows_fluid(&*host_ref, source_filter, &fluid_id)
            && filter::allows_fluid(&*host_ref, sink_filter, &fluid_id)
        {
            allowed.push(res);
        }
    }
    if allowed.is_empty() {
        return 0;
    }
    host_ref
        .transaction(|host_ref, tx| {
            move_resources(
                host_ref,
                &src_cap,
                &dst_cap,
                |res: &FluidResource| allowed.iter().any(|ok| ok == res),
                max,
                tx,
            )
            .map_err(|_| ())
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Block entity tick & probe helpers
// ---------------------------------------------------------------------------

fn sync_nexus_connections(host_ref: &mut dyn ServerHost, level: LevelRef, pos: Pos) {
    for (property, offset) in [
        ("up", [0, 1, 0]),
        ("down", [0, -1, 0]),
        ("north", [0, 0, -1]),
        ("south", [0, 0, 1]),
        ("east", [1, 0, 0]),
        ("west", [-1, 0, 0]),
    ] {
        let neighbor = [pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]];
        let connected = host::block_id_at(&*host_ref, level, neighbor)
            .is_some_and(|b| !b.is_empty() && !items::is_air_name(&b));
        let want = if connected { "true" } else { "false" };
        if level.block_property(&*host_ref, pos, property).as_deref() != Some(want) {
            host_ref.set_block_property_with(pos, property, want, 3);
        }
    }
}

fn restore_attached_target_if_empty(
    host_ref: &dyn Host,
    level: LevelRef,
    entity_id: &str,
    pos: Pos,
    entity: &mut ModBlockEntityView<'_>,
) {
    if entity_id == "theurgy:logistics_capability_proxy" {
        return;
    }
    let phase = entity.data.get(8).copied().unwrap_or(0);
    let target = attached(pos, facing_of(host_ref, level, pos));
    let populated = matches!(
        entity.nbt.get(&"targets".into()),
        Some(Tag::List(targets)) if !targets.is_empty()
    );
    if populated {
        if phase >= 2
            && let Some(Tag::List(targets)) = entity.nbt.get_mut(&"targets".into())
            && targets.len() == 1
            && matches!(
                targets.first(),
                Some(Tag::Long(v)) if items::manhattan(wire_pos_from_long(*v), pos) == 1
            )
        {
            targets[0] = Tag::Long(wire_pos_long(target));
        }
        return;
    }
    if phase == 0 {
        if entity.data.len() <= 8 {
            entity.data.resize(9, 0);
        }
        entity.data[8] = 1;
        return;
    }
    entity
        .nbt
        .insert("targets".into(), Tag::List(vec![Tag::Long(wire_pos_long(target))]));
    if entity.data.len() <= 8 {
        entity.data.resize(9, 0);
    }
    entity.data[8] = 2;
}

pub fn tick(
    host_ref: &mut dyn ServerHost,
    entity_id: &str,
    pos: Pos,
    entity: &mut ModBlockEntityView<'_>,
) {
    let Some(level) = host::level_in_place(host_ref) else {
        return;
    };
    let dim = host_ref.dimension().to_owned();
    let facing = facing_of(&*host_ref, level, pos);

    if is_nexus(entity_id) {
        sync_nexus_connections(host_ref, level, pos);
        if java_nexus_id(&entity.nbt).is_none() {
            let id = fresh_nexus_id(pos, entity_id);
            host::set_string(&mut entity.nbt, "nexusId", &id);
        }
        record_node(&dim, pos, entity_id, &entity.nbt);
        return;
    }

    if !is_connector(entity_id) {
        // Record wire-only nodes (`logistics_connector_node`, `logistics_capability_probe`).
        record_node(&dim, pos, entity_id, &entity.nbt);
        return;
    }

    let is_proxy = entity_id == "theurgy:logistics_capability_proxy";
    let Some((mode, kind)) = block_kind(entity_id) else {
        return;
    };
    restore_attached_target_if_empty(&*host_ref, level, entity_id, pos, entity);

    let mut state = read_state(&entity.nbt);
    let own_filter = saved_filter(&*host_ref, &entity.nbt);
    if state.extraction_amount == 0 {
        state.extraction_amount = match kind {
            CapabilityKind::Item => MAX_ITEM_EXTRACTION_AMOUNT,
            CapabilityKind::Fluid => MAX_FLUID_EXTRACTION_AMOUNT,
            CapabilityKind::MercuryFlux => FLUX_DEFAULT_TRANSFER_RATE,
        };
    }
    state.flux_buffer = state.flux_buffer.clamp(0, FLUX_BUFFER_CAPACITY);
    state.extraction_amount = state.extraction_amount.max(1);
    let target_dir = connector_direction(state.direction_override, facing);

    record_node(&dim, pos, entity_id, &entity.nbt);

    if !state.enabled {
        write_state(&mut entity.nbt, &state);
        return;
    }

    let period = if is_proxy {
        20
    } else {
        match kind {
            CapabilityKind::Item => ITEM_EXTRACTION_EVERY_N_TICKS,
            CapabilityKind::Fluid => FLUID_EXTRACTION_EVERY_N_TICKS,
            CapabilityKind::MercuryFlux => FLUX_TRANSFER_EVERY_N_TICKS,
        }
    };
    let game_time = level.game_time(&*host_ref).unwrap_or(0);
    if !on_slow_tick(slow_tick_offset(pos, period), game_time, period) {
        write_state(&mut entity.nbt, &state);
        return;
    }

    let nexus_id = java_nexus_id(&entity.nbt);
    let wired = wire_network(&*host_ref, level, &dim, pos, &entity.nbt);
    if nexus_id.is_none() && wired.len() <= 1 {
        write_state(&mut entity.nbt, &state);
        return;
    }
    let frequency = host::int(&entity.nbt, "frequency", 0);
    let network = collect_network(
        &*host_ref,
        level,
        &dim,
        nexus_id.as_deref(),
        &wired,
        frequency,
        pos,
    );

    if is_proxy {
        let probes = connected_probes(&dim, nexus_id.as_deref(), &wired);
        if probes.is_empty() {
            write_state(&mut entity.nbt, &state);
            return;
        }
        let idx = (host::int(&entity.nbt, "roundRobinIndex", 0) + 1)
            .rem_euclid(probes.len() as i32) as usize;
        host::set_int(&mut entity.nbt, "roundRobinIndex", idx as i32);
        let probe = probes[idx];
        let probe_facing = facing_of(&*host_ref, level, probe);
        let source = attached(probe, probe_facing);
        let has_fluid = level
            .capability(
                host_ref,
                &FLUIDS,
                source,
                Some(host::ordinal_to_direction(probe_facing)),
            )
            .is_some();
        if has_fluid {
            if let Some((sink_pos, _, _, sink_dir, sink_filter)) = network
                .iter()
                .find(|(_, m, k, _, _)| *m == LeafNodeMode::Insert && *k == CapabilityKind::Fluid)
            {
                let sink = attached(*sink_pos, *sink_dir);
                let _ = transfer_fluids_filtered(
                    host_ref,
                    level,
                    source,
                    probe_facing,
                    sink,
                    *sink_dir,
                    None,
                    sink_filter.as_ref(),
                    MAX_FLUID_EXTRACTION_AMOUNT as u32,
                );
            }
        } else if let Some((sink_pos, _, _, sink_dir, sink_filter)) = network
            .iter()
            .find(|(_, m, k, _, _)| *m == LeafNodeMode::Insert && *k == CapabilityKind::Item)
        {
            let sink = attached(*sink_pos, *sink_dir);
            let _ = transfer_items_filtered(
                host_ref,
                level,
                source,
                probe_facing,
                sink,
                *sink_dir,
                None,
                sink_filter.as_ref(),
                state.extraction_amount as u32,
            );
        }
        write_state(&mut entity.nbt, &state);
        return;
    }

    match (mode, kind) {
        (LeafNodeMode::Extract, CapabilityKind::Item) => {
            let sinks: Vec<_> = network
                .iter()
                .filter(|(_, m, k, _, _)| *m == LeafNodeMode::Insert && *k == CapabilityKind::Item)
                .collect();
            if !sinks.is_empty() {
                let idx = (host::int(&entity.nbt, "distributor", 0) + 1)
                    .rem_euclid(sinks.len() as i32) as usize;
                host::set_int(&mut entity.nbt, "distributor", idx as i32);
                let (sink_pos, _, _, sink_dir, sink_filter) = sinks[idx];
                let source = attached(pos, target_dir);
                let sink = attached(*sink_pos, *sink_dir);
                let moved = transfer_items_filtered(
                    host_ref,
                    level,
                    source,
                    target_dir,
                    sink,
                    *sink_dir,
                    own_filter.as_ref(),
                    sink_filter.as_ref(),
                    state.extraction_amount.clamp(1, MAX_ITEM_EXTRACTION_AMOUNT) as u32,
                );
                if moved > 0 {
                    host::set_int(&mut entity.nbt, "lastTransfer", moved as i32);
                }
            }
        }
        (LeafNodeMode::Extract, CapabilityKind::Fluid) => {
            let sinks: Vec<_> = network
                .iter()
                .filter(|(_, m, k, _, _)| {
                    *m == LeafNodeMode::Insert && *k == CapabilityKind::Fluid
                })
                .collect();
            if !sinks.is_empty() {
                let idx = (host::int(&entity.nbt, "distributor", 0) + 1)
                    .rem_euclid(sinks.len() as i32) as usize;
                host::set_int(&mut entity.nbt, "distributor", idx as i32);
                let (sink_pos, _, _, sink_dir, sink_filter) = sinks[idx];
                let source = attached(pos, target_dir);
                let sink = attached(*sink_pos, *sink_dir);
                let moved = transfer_fluids_filtered(
                    host_ref,
                    level,
                    source,
                    target_dir,
                    sink,
                    *sink_dir,
                    own_filter.as_ref(),
                    sink_filter.as_ref(),
                    state.extraction_amount.clamp(1, MAX_FLUID_EXTRACTION_AMOUNT) as u32,
                );
                if moved > 0 {
                    host::set_int(&mut entity.nbt, "lastTransfer", moved as i32);
                }
            }
        }
        (LeafNodeMode::Insert, CapabilityKind::MercuryFlux) => {
            let sinks: Vec<(Pos, u8)> = network
                .iter()
                .filter(|(p, m, k, _, _)| {
                    *p != pos
                        && *m == LeafNodeMode::Insert
                        && *k == CapabilityKind::MercuryFlux
                })
                .map(|(p, _, _, d, _)| (attached(*p, *d), *d))
                .collect();
            if !sinks.is_empty() && state.flux_buffer > 0 {
                let total = flux_pushes_for(state.flux_buffer, sinks.len());
                let shares = distribute_evenly(total, sinks.len());
                let mut moved_total = 0;
                for ((sink, face), amount) in sinks.iter().zip(shares) {
                    if level.block_property(&*host_ref, *sink, "enabled").as_deref()
                        == Some("false")
                    {
                        continue;
                    }
                    let incoming = flux::FluxDir::from_index(*face as usize);
                    let moved = flux::insert_mercury_flux_at(
                        host_ref,
                        level,
                        *sink,
                        incoming,
                        amount,
                    );
                    if moved > 0 {
                        state.flux_buffer -= moved;
                        moved_total += moved;
                    }
                }
                if moved_total > 0 {
                    host::set_int(&mut entity.nbt, "lastTransfer", moved_total);
                }
            }
        }
        _ => {}
    }

    write_state(&mut entity.nbt, &state);
    record_node(&dim, pos, entity_id, &entity.nbt);
}

/// Probe readout helpers for `probe.rs`.
pub fn probe_insert_targets(host_ref: &dyn Host, level: LevelRef, dim: &str, pos: Pos) -> Vec<Pos> {
    let Some(be) = host_ref.block_entity(level, pos) else {
        return Vec::new();
    };
    let Some(id) = be.kind(host_ref).map(|k| k.id(host_ref)) else {
        return Vec::new();
    };
    let Some((_, kind)) = block_kind(&id) else {
        return Vec::new();
    };
    let nbt = be.data(host_ref).unwrap_or_default();
    let nexus_id = java_nexus_id(&nbt);
    let wired = wire_network(host_ref, level, dim, pos, &nbt);
    collect_network(
        host_ref,
        level,
        dim,
        nexus_id.as_deref(),
        &wired,
        host::int(&nbt, "frequency", 0),
        pos,
    )
    .into_iter()
    .filter(|(_, mode, candidate, _, _)| *mode == LeafNodeMode::Insert && *candidate == kind)
    .map(|(target, _, _, _, _)| target)
    .collect()
}

pub fn probe_nexus_partners(host_ref: &dyn Host, level: LevelRef, dim: &str, pos: Pos) -> usize {
    let Some(be) = host_ref.block_entity(level, pos) else {
        return 0;
    };
    let nbt = be.data(host_ref).unwrap_or_default();
    let Some(id) = java_nexus_id(&nbt) else {
        return 0;
    };
    nodes_map()
        .read()
        .ok()
        .and_then(|m| {
            m.get(dim).map(|d| {
                d.iter()
                    .filter(|(other, node)| {
                        **other != pos
                            && is_nexus(&node.block_id)
                            && node.nexus_id.as_deref() == Some(id.as_str())
                    })
                    .count()
            })
        })
        .unwrap_or(0)
}

fn proxy_fluid_target(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    dim: &str,
    pos: Pos,
) -> Option<(Pos, u8, usize, String, i32)> {
    let be = host_ref.block_entity(level, pos)?;
    let mut own_nbt = be.data(&*host_ref).unwrap_or_default();
    let nexus_id = java_nexus_id(&own_nbt);
    let wired = wire_network(&*host_ref, level, dim, pos, &own_nbt);
    let mut probes: Vec<Pos> = nodes_map()
        .read()
        .ok()
        .and_then(|m| {
            m.get(dim).map(|d| {
                d.iter()
                    .filter(|(p, node)| {
                        node.block_id == "theurgy:logistics_capability_probe"
                            && (wired.contains(*p)
                                || (nexus_id.is_some()
                                    && node.nexus_id.as_deref() == nexus_id.as_deref()))
                    })
                    .map(|(p, _)| *p)
                    .collect()
            })
        })
        .unwrap_or_default();
    probes.sort_by_key(|p| wire_pos_long(*p));
    if probes.is_empty() {
        return None;
    }
    let index =
        (host::int(&own_nbt, "roundRobinIndex", 0) + 1).rem_euclid(probes.len() as i32) as usize;
    host::set_int(&mut own_nbt, "roundRobinIndex", index as i32);
    let _ = be.merge_data(host_ref, own_nbt);
    let probe = probes[index];
    let facing = facing_of(&*host_ref, level, probe);
    let target = attached(probe, facing);
    let dir = host::ordinal_to_direction(facing);
    let cap = level.capability(host_ref, &FLUIDS, target, Some(dir))?;
    let size = cap.size(host_ref);
    for idx in 0..size {
        let amt = cap.amount(host_ref, idx);
        if amt > 0
            && let Some(res) = cap.resource(host_ref, idx)
            && let Some(fluid_id) = res.fluid().key().map(|k| k.id(&*host_ref))
        {
            return Some((target, facing, idx, fluid_id, amt as i32));
        }
    }
    None
}

pub fn proxy_read_fluid(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    dim: &str,
    pos: Pos,
) -> Option<(String, i32)> {
    proxy_fluid_target(host_ref, level, dim, pos).map(|(_, _, _, fluid, amount)| (fluid, amount))
}

pub fn proxy_drain_fluid(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    dim: &str,
    pos: Pos,
    amount: i32,
) -> i32 {
    if amount <= 0 {
        return 0;
    }
    let Some((target, facing, slot_idx, fluid, _)) = proxy_fluid_target(host_ref, level, dim, pos)
    else {
        return 0;
    };
    let Some(fluid_key) = host_ref.fluid_key(&fluid) else {
        return 0;
    };
    let dir = host::ordinal_to_direction(facing);
    let res = FluidResource::of(&FluidStack::new(fluid_key, 1));
    let extracted = host_ref.transaction(|host_ref, tx| {
        let cap = level
            .capability(host_ref, &FLUIDS, target, Some(dir))
            .ok_or(())?;
        Ok::<u32, ()>(cap.extract(host_ref, tx, slot_idx, &res, amount as u32))
    });
    match extracted {
        Ok(moved) => {
            if moved > 0 {
                host_ref.block_entity_sync(target);
            }
            moved as i32
        }
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_java_pos_roundtrip_and_negative_coordinates() {
        for p in [
            [30, 80, 28],
            [-1, -100, -1],
            [-1000, 320, 2000],
            [-33_554_432, -2048, -33_554_432],
        ] {
            assert_eq!(wire_pos_from_long(wire_pos_long(p)), p);
        }
        assert_eq!(wire_pos_long([30, 80, 28]), 8_246_337_323_088);
    }

    #[test]
    fn wire_graph_reaches_cycles_and_not_disconnected_components() {
        let edges = [
            ([0, 0, 0], [1, 0, 0]),
            ([2, 0, 0], [0, 0, 0]),
            ([2, 0, 0], [1, 0, 0]),
            ([9, 0, 0], [10, 0, 0]),
        ];
        assert_eq!(
            reachable_nodes([0, 0, 0], &edges),
            vec![[0, 0, 0], [1, 0, 0], [2, 0, 0]]
        );
        assert!(same_wire(edges[0], [1, 0, 0], [0, 0, 0]));
    }

    #[test]
    fn wire_records_reject_malformed_and_persist_java_longs() {
        let mut bad = Compound::new();
        bad.insert("from".into(), Tag::String("bad".into()));
        let mut nbt = Compound::new();
        nbt.insert(WIRE_CONNECTIONS.into(), Tag::List(vec![Tag::Compound(bad)]));
        assert!(wire_records(&nbt).is_empty());

        let mut wire = Compound::new();
        wire.insert("from".into(), Tag::Long(wire_pos_long([40, 80, 40])));
        wire.insert("to".into(), Tag::Long(wire_pos_long([47, 80, 40])));
        nbt.insert(WIRE_CONNECTIONS.into(), Tag::List(vec![Tag::Compound(wire)]));
        assert_eq!(wire_records(&nbt), vec![([40, 80, 40], [47, 80, 40])]);
    }

    #[test]
    fn slow_tick_and_offset_rules() {
        assert!(on_slow_tick(0, 0, 20));
        assert!(on_slow_tick(0, 20, 20));
        assert!(!on_slow_tick(0, 1, 20));
        assert!(on_slow_tick(5, 15, 20));
        assert!(!on_slow_tick(5, 16, 20));
        assert!(on_slow_tick(0, -20, 20));
        assert!(!on_slow_tick(0, 0, 0));
        assert_eq!(slow_tick_offset([1, 2, 3], 20), slow_tick_offset([1, 2, 3], 20));
        assert!(slow_tick_offset([0, 0, 0], 20) < 20);
    }

    #[test]
    fn distribute_evenly_and_flux_pushes() {
        assert_eq!(distribute_evenly(10, 3), vec![4, 3, 3]);
        assert_eq!(distribute_evenly(5, 5), vec![1, 1, 1, 1, 1]);
        assert!(distribute_evenly(0, 3).is_empty());
        assert!(distribute_evenly(10, 0).is_empty());
        assert_eq!(flux_pushes_for(1000, 3), 300);
        assert_eq!(flux_pushes_for(50, 3), 50);
        assert_eq!(flux_pushes_for(0, 3), 0);
        assert_eq!(flux_pushes_for(1000, 0), 0);
    }

    #[test]
    fn network_queries_and_state_roundtrip() {
        let cands = [
            ([0, 0, 0], "a", LeafNodeMode::Insert, CapabilityKind::Item),
            ([1, 0, 0], "b", LeafNodeMode::Insert, CapabilityKind::Item),
            ([2, 0, 0], "a", LeafNodeMode::Extract, CapabilityKind::Fluid),
        ];
        let nodes = leaf_nodes_of("a", &cands);
        assert_eq!(nodes.len(), 2);
        assert_eq!(insert_targets(&nodes, [9, 9, 9], CapabilityKind::Item).len(), 1);

        let mut nbt = Compound::new();
        let state = LogisticsState {
            enabled: false,
            direction_override: Some(3),
            extraction_amount: 42,
            flux_buffer: 7,
        };
        write_state(&mut nbt, &state);
        assert_eq!(read_state(&nbt), state);
        assert!(read_state(&Compound::new()).enabled);

        nbt.insert("directionOverride".into(), Tag::Int(9));
        assert_eq!(read_state(&nbt).direction_override, None);
    }

    #[test]
    fn wire_count_dedup_and_nexus_id() {
        assert_eq!(next_wire_count(2, -1, false), 1);
        assert_eq!(next_wire_count(1, 1, false), 2);
        assert_eq!(next_wire_count(2, -1, true), 2);
        assert_eq!(next_wire_count(0, -1, false), 0);

        let action = (7, [1, 80, 2], 100);
        assert!(repeated_wire_action(Some(action), (7, [1, 80, 2], 104)));
        assert!(!repeated_wire_action(Some(action), (7, [1, 80, 2], 105)));

        let a = fresh_nexus_id([0, 0, 0], "theurgy:logistics_nexus");
        let b = fresh_nexus_id([0, 0, 0], "theurgy:logistics_nexus");
        let c = fresh_nexus_id([1, 0, 0], "theurgy:logistics_nexus");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
