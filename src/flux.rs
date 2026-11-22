// SPDX-License-Identifier: MIT
//! Mercury flux conversion and distribution:
//! - `SimpleMercuryFluxHandler`
//! - `MercuryCatalyst` (`CatalystConversion`)
//! - `MercuryCapacitor` (per-side `SideMode` configuration)
//! - `MercuryFluxEmitter` (separate Mercury Flux & FE buffers)
//! - `CaloricFluxEmitter` (writes `hotUntil` on heat receivers)

use mod_api::serde_json::Value;
use mod_api::{Compound, Host, ItemStack, LevelRef, ModBlockEntityView, ServerHost, Tag};

use crate::{host, recipes, selection};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FluxDir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl FluxDir {
    pub const ALL: [FluxDir; 6] = [
        FluxDir::Down,
        FluxDir::Up,
        FluxDir::North,
        FluxDir::South,
        FluxDir::West,
        FluxDir::East,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FluxDir::Down => "DOWN",
            FluxDir::Up => "UP",
            FluxDir::North => "NORTH",
            FluxDir::South => "SOUTH",
            FluxDir::West => "WEST",
            FluxDir::East => "EAST",
        }
    }

    pub fn opposite(self) -> FluxDir {
        match self {
            FluxDir::Down => FluxDir::Up,
            FluxDir::Up => FluxDir::Down,
            FluxDir::North => FluxDir::South,
            FluxDir::South => FluxDir::North,
            FluxDir::West => FluxDir::East,
            FluxDir::East => FluxDir::West,
        }
    }

    pub fn offset(self, pos: [i32; 3]) -> [i32; 3] {
        let [x, y, z] = pos;
        match self {
            FluxDir::Down => [x, y - 1, z],
            FluxDir::Up => [x, y + 1, z],
            FluxDir::North => [x, y, z - 1],
            FluxDir::South => [x, y, z + 1],
            FluxDir::West => [x - 1, y, z],
            FluxDir::East => [x + 1, y, z],
        }
    }

    pub fn from_name(name: &str) -> Option<FluxDir> {
        Self::ALL.into_iter().find(|d| d.name() == name)
    }

    pub fn from_index(index: usize) -> Option<FluxDir> {
        Self::ALL.get(index).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideMode {
    None,
    Input,
    Output,
    Both,
}

impl SideMode {
    pub fn ordinal(self) -> i32 {
        match self {
            SideMode::None => 0,
            SideMode::Input => 1,
            SideMode::Output => 2,
            SideMode::Both => 3,
        }
    }

    pub fn from_ordinal(value: i32) -> SideMode {
        match value.clamp(0, 3) {
            0 => SideMode::None,
            1 => SideMode::Input,
            2 => SideMode::Output,
            _ => SideMode::Both,
        }
    }

    pub fn cycle(self) -> SideMode {
        match self {
            SideMode::None => SideMode::Input,
            SideMode::Input => SideMode::Output,
            SideMode::Output => SideMode::Both,
            SideMode::Both => SideMode::None,
        }
    }

    pub fn can_insert(self) -> bool {
        matches!(self, SideMode::Input | SideMode::Both)
    }

    pub fn can_extract(self) -> bool {
        matches!(self, SideMode::Output | SideMode::Both)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FluxTransfer {
    pub inserted: i32,
    pub extracted: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimpleMercuryFluxHandler {
    pub energy: i32,
    pub capacity: i32,
    pub max_insert: i32,
    pub max_extract: i32,
}

impl SimpleMercuryFluxHandler {
    pub fn new(capacity: i32) -> Self {
        Self::with_transfer(capacity, capacity, capacity)
    }

    pub fn with_transfer(capacity: i32, max_insert: i32, max_extract: i32) -> Self {
        Self::with_energy(capacity, max_insert, max_extract, 0)
    }

    pub fn with_energy(capacity: i32, max_insert: i32, max_extract: i32, energy: i32) -> Self {
        Self {
            energy: energy.max(0),
            capacity: capacity.max(0),
            max_insert: max_insert.max(0),
            max_extract: max_extract.max(0),
        }
    }

    pub fn amount(&self) -> i32 {
        self.energy
    }

    pub fn set(&mut self, amount: i32) {
        self.energy = amount.clamp(0, self.capacity.max(0));
    }

    pub fn insert(&mut self, amount: i32) -> i32 {
        let amount = amount.max(0);
        let space = (self.capacity - self.energy).max(0);
        let inserted = space.min(amount.min(self.max_insert));
        if inserted > 0 {
            self.energy += inserted;
        }
        inserted
    }

    pub fn extract(&mut self, amount: i32) -> i32 {
        let amount = amount.max(0);
        let extracted = self.energy.max(0).min(amount.min(self.max_extract));
        if extracted > 0 {
            self.energy -= extracted;
        }
        extracted
    }

    pub fn is_full(&self) -> bool {
        self.energy >= self.capacity
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CatalystConversion {
    pub mercury_flux_to_convert: i32,
    pub total_mercury_flux_to_convert: i32,
    pub current_mercury_flux_per_tick: i32,
}

impl CatalystConversion {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn progress_percent(&self) -> i32 {
        if self.total_mercury_flux_to_convert <= 0 {
            return 0;
        }
        let processed = self.total_mercury_flux_to_convert - self.mercury_flux_to_convert;
        ((i64::from(processed) * 100) / i64::from(self.total_mercury_flux_to_convert))
            .clamp(0, 100) as i32
    }

    pub fn can_craft(buffer_space: i32, has_recipe: bool) -> bool {
        has_recipe && buffer_space > 0
    }

    pub fn begin(&mut self, total_flux: i32, flux_per_tick: i32) {
        let total = total_flux.max(0);
        self.mercury_flux_to_convert = total;
        self.total_mercury_flux_to_convert = total;
        self.current_mercury_flux_per_tick = flux_per_tick.max(0);
    }

    pub fn convert_tick(&mut self, buffer: &mut SimpleMercuryFluxHandler) -> i32 {
        if self.mercury_flux_to_convert <= 0 || buffer.is_full() {
            return 0;
        }
        let wanted = self
            .mercury_flux_to_convert
            .min(self.current_mercury_flux_per_tick.max(0));
        if wanted <= 0 {
            return 0;
        }
        let before = buffer.energy;
        let _ = buffer.insert(wanted);
        let added = buffer.energy - before;
        if added > 0 {
            self.mercury_flux_to_convert = (self.mercury_flux_to_convert - added).max(0);
        }
        added
    }

    pub fn is_processing(&self) -> bool {
        self.mercury_flux_to_convert > 0
    }
}

pub const PUSH_TICK_INTERVAL: i32 = 20;
pub const PUSH_RATE_PER_SIDE_PER_TICK: i32 = 2;

pub fn split_evenly(total: i32, targets: usize) -> Vec<i32> {
    let mut shares = vec![0; targets];
    if targets == 0 {
        return shares;
    }
    let total = total.max(0);
    let base = total / targets as i32;
    let remainder = (total % targets as i32) as usize;
    for (i, share) in shares.iter_mut().enumerate() {
        *share = base + i32::from(i < remainder);
    }
    shares
}

pub fn total_to_push(stored: i32, target_count: usize) -> i32 {
    stored
        .max(0)
        .min(PUSH_RATE_PER_SIDE_PER_TICK * PUSH_TICK_INTERVAL * target_count.max(1) as i32)
}

pub const CATALYST_CAPACITY: i32 = 50_000;
pub const CATALYST_UPDATE_THRESHOLD: i32 = 100;

pub const CAPACITOR_CAPACITY: i32 = 1_000 * 50 * 10;
pub const CAPACITOR_UPDATE_THRESHOLD: i32 = 1_000;

pub const MERCURY_EMITTER_CAPACITY: i32 = 1_000;
pub const MERCURY_EMITTER_FLUX_PER_TRANSFER: i32 = 10;
pub const MERCURY_EMITTER_TICK_INTERVAL: i32 = 20;

pub const CALORIC_CAPACITY: i32 = 1_000;
pub const CALORIC_FLUX_PER_HEAT: i32 = 100;
pub const CALORIC_HEAT_TARGET_FOR_TICKS: i32 = 20 * 6;
pub const CALORIC_TICK_INTERVAL: i32 = 20;

pub fn default_capacitor_side_modes() -> [(FluxDir, SideMode); 6] {
    [
        (FluxDir::Up, SideMode::Output),
        (FluxDir::Down, SideMode::Output),
        (FluxDir::North, SideMode::Input),
        (FluxDir::South, SideMode::Input),
        (FluxDir::East, SideMode::Input),
        (FluxDir::West, SideMode::Input),
    ]
}

fn compound_child<'a>(nbt: &'a Compound, key: &str) -> Option<&'a Compound> {
    match nbt.get(&key.into()) {
        Some(Tag::Compound(c)) => Some(c),
        _ => None,
    }
}

fn set_child_compound(nbt: &mut Compound, key: &str, child: Compound) {
    nbt.insert(key.into(), Tag::Compound(child));
}

pub fn tank_energy(nbt: &Compound, key: &str) -> i32 {
    compound_child(nbt, key)
        .map(|c| host::int(c, "energy", 0))
        .unwrap_or(0)
}

pub fn store_energy_child(nbt: &mut Compound, key: &str, energy: i32) {
    let mut child = Compound::new();
    host::set_int(&mut child, "energy", energy.max(0));
    set_child_compound(nbt, key, child);
}

fn enabled(nbt: &Compound) -> bool {
    nbt.get(&"enabled".into()).is_none() || host::bool(nbt, "enabled")
}

fn json_int(value: &Value, key: &str) -> Option<i32> {
    value.get(key).and_then(Value::as_i64).map(|v| v as i32)
}

pub fn recipe_flux_budget(json: &Value) -> Option<(i32, i32)> {
    let total = json_int(json, "totalMercuryFlux").or_else(|| json_int(json, "mercuryFlux"))?;
    let per_tick = json_int(json, "mercuryFluxPerTick").unwrap_or(total);
    Some((total, per_tick))
}

fn find_catalysation_recipe(host_ref: &dyn Host, stack: &ItemStack) -> Option<(String, i32, i32)> {
    for recipe in recipes::of_type("theurgy:catalysation").iter() {
        let matched = recipe
            .inputs
            .first()
            .is_some_and(|ing| recipes::matches_cached(host_ref, ing, stack))
            || recipe
                .json
                .get("ingredient")
                .is_some_and(|v| recipes::matches_value(host_ref, v, stack));
        if matched {
            let (total, per_tick) = recipe_flux_budget(&recipe.json)?;
            return Some((recipe.id.clone(), total, per_tick));
        }
    }
    None
}

fn load_catalyst_conversion(nbt: &Compound) -> CatalystConversion {
    CatalystConversion {
        mercury_flux_to_convert: host::int(nbt, "mercuryFluxToConvert", 0),
        total_mercury_flux_to_convert: host::int(nbt, "totalMercuryFluxToConvert", 0),
        current_mercury_flux_per_tick: host::int(nbt, "currentMercuryFluxPerTick", 0),
    }
}

fn save_catalyst_conversion(nbt: &mut Compound, state: &CatalystConversion) {
    host::set_int(nbt, "mercuryFluxToConvert", state.mercury_flux_to_convert);
    host::set_int(
        nbt,
        "totalMercuryFluxToConvert",
        state.total_mercury_flux_to_convert,
    );
    host::set_int(
        nbt,
        "currentMercuryFluxPerTick",
        state.current_mercury_flux_per_tick,
    );
}

fn sync_enabled_from_redstone(host_ref: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3]) {
    let is_enabled = level.best_neighbor_signal(&*host_ref, pos).unwrap_or(0) == 0;
    let want = if is_enabled { "true" } else { "false" };
    if level.block_property(&*host_ref, pos, "enabled").as_deref() != Some(want) {
        host_ref.set_block_property_with(pos, "enabled", want, 3);
    }
}

pub fn receiver_limits(id: &str) -> Option<(i32, i32)> {
    Some(match id {
        "theurgy:mercury_catalyst" => (CATALYST_CAPACITY, 0),
        "theurgy:mercury_capacitor" => (CAPACITOR_CAPACITY, CAPACITOR_CAPACITY),
        "theurgy:mercury_flux_emitter"
        | "theurgy:caloric_flux_emitter"
        | "theurgy:sulfuric_flux_emitter"
        | "theurgy:logistics_mercury_flux_connector" => (1_000, 1_000),
        _ => return None,
    })
}

pub fn load_side_modes(nbt: &Compound) -> [(FluxDir, SideMode); 6] {
    let mut out = default_capacitor_side_modes();
    if let Some(child) = compound_child(nbt, "sideModes") {
        for (dir, mode) in out.iter_mut() {
            let fallback = mode.ordinal();
            let ordinal = host::int(child, dir.name(), fallback);
            *mode = SideMode::from_ordinal(ordinal);
        }
    }
    out
}

pub fn save_side_modes(nbt: &mut Compound, modes: &[(FluxDir, SideMode); 6]) {
    let mut child = Compound::new();
    for (dir, mode) in modes {
        host::set_int(&mut child, dir.name(), mode.ordinal());
    }
    set_child_compound(nbt, "sideModes", child);
}

/// Insert mercury flux into a target block entity at `target`, respecting side
/// modes and enabled flags. Returns the amount accepted.
pub fn insert_mercury_flux_at(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    target: [i32; 3],
    incoming_side: Option<FluxDir>,
    amount: i32,
) -> i32 {
    if amount <= 0 {
        return 0;
    }
    let Some(be) = host_ref.block_entity(level, target) else {
        return 0;
    };
    let Some(id) = be.kind(&*host_ref).map(|k| k.id(&*host_ref)) else {
        return 0;
    };
    let Some((capacity, max_insert)) = receiver_limits(&id) else {
        return 0;
    };
    if max_insert <= 0 {
        return 0;
    }
    let mut nbt = be.data(&*host_ref).unwrap_or_default();
    if id == "theurgy:logistics_mercury_flux_connector" && !enabled(&nbt) {
        return 0;
    }
    if id == "theurgy:mercury_capacitor" {
        let block_enabled =
            level.block_property(&*host_ref, target, "enabled").as_deref() != Some("false");
        if !block_enabled || !enabled(&nbt) {
            return 0;
        }
        if let Some(side) = incoming_side {
            let modes = load_side_modes(&nbt);
            let mode = modes
                .iter()
                .find(|(d, _)| *d == side)
                .map_or(SideMode::None, |(_, m)| *m);
            if !mode.can_insert() {
                return 0;
            }
        }
    }
    let stored = tank_energy(&nbt, "mercuryFluxHandler").clamp(0, capacity);
    let mut receiver = SimpleMercuryFluxHandler::with_energy(capacity, max_insert, capacity, stored);
    let accepted = receiver.insert(amount);
    if accepted > 0 {
        store_energy_child(&mut nbt, "mercuryFluxHandler", receiver.energy);
        let _ = be.merge_data(host_ref, nbt);
        host_ref.block_entity_sync(target);
    }
    accepted
}

fn push_flux(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    buffer: &mut SimpleMercuryFluxHandler,
    dirs: &[FluxDir],
    modes: Option<&[(FluxDir, SideMode); 6]>,
) {
    let mut directions: Vec<FluxDir> = Vec::new();
    for dir in dirs.iter().copied() {
        if let Some(modes) = modes {
            let mode = modes
                .iter()
                .find(|(candidate, _)| *candidate == dir)
                .map_or(SideMode::None, |(_, mode)| *mode);
            if !mode.can_extract() {
                continue;
            }
        }
        let neighbor = dir.offset(pos);
        let neighbor_enabled =
            level.block_property(&*host_ref, neighbor, "enabled").as_deref() != Some("false");
        let allowed = host_ref.block_entity(level, neighbor).is_some_and(|be| {
            let Some(id) = be.kind(&*host_ref).map(|k| k.id(&*host_ref)) else {
                return false;
            };
            if receiver_limits(&id).is_none() {
                return false;
            }
            if id == "theurgy:mercury_capacitor" {
                let nbt = be.data(&*host_ref).unwrap_or_default();
                if !neighbor_enabled || !enabled(&nbt) {
                    return false;
                }
                let neighbor_modes = load_side_modes(&nbt);
                let side_mode = neighbor_modes
                    .iter()
                    .find(|(candidate, _)| *candidate == dir.opposite())
                    .map_or(SideMode::None, |(_, mode)| *mode);
                if !side_mode.can_insert() {
                    return false;
                }
            }
            true
        });
        if allowed {
            directions.push(dir);
        }
    }
    if directions.is_empty() {
        return;
    }

    // Deterministic Fisher-Yates shuffle seeded from game time and position.
    let mut seed = (level.game_time(&*host_ref).unwrap_or(0) as u64)
        ^ (pos[0] as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (pos[1] as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ (pos[2] as u64).wrapping_mul(0x94d0_49bb_1331_11eb);
    for index in (1..directions.len()).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let other = ((seed >> 33) as usize) % (index + 1);
        directions.swap(index, other);
    }

    let total = total_to_push(buffer.energy, directions.len());
    if total <= 0 {
        return;
    }
    let shares = split_evenly(total, directions.len());
    for (dir, share) in directions.iter().zip(shares.iter()) {
        if *share <= 0 {
            continue;
        }
        let neighbor = dir.offset(pos);
        let accepted =
            insert_mercury_flux_at(host_ref, level, neighbor, Some(dir.opposite()), *share);
        if accepted > 0 {
            let extracted = buffer.extract(accepted);
            debug_assert_eq!(extracted, accepted);
        }
    }
}

fn tick_catalyst(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    sync_enabled_from_redstone(host_ref, level, pos);
    if entity.items.is_empty() {
        entity.items.push(None);
    }
    let mut conversion = load_catalyst_conversion(&entity.nbt);
    let stored_energy = tank_energy(&entity.nbt, "mercuryFluxHandler");
    let mut buffer = SimpleMercuryFluxHandler::with_energy(
        CATALYST_CAPACITY,
        CATALYST_CAPACITY,
        CATALYST_CAPACITY,
        stored_energy.clamp(0, CATALYST_CAPACITY),
    );

    if conversion.mercury_flux_to_convert > 0 {
        let _ = conversion.convert_tick(&mut buffer);
    } else if let Some(stack) = entity.items.first().and_then(Option::as_ref).cloned()
        && stack.count() > 0
        && let Some((_id, total, per_tick)) = find_catalysation_recipe(&*host_ref, &stack)
    {
        let space = (CATALYST_CAPACITY - buffer.energy).max(0);
        if CatalystConversion::can_craft(space, true) {
            conversion.begin(total, per_tick);
            if let Some(slot) = entity.items.first_mut()
                && let Some(item) = slot.as_mut()
            {
                if item.count() > 1 {
                    let left = item.count() - 1;
                    let _ = item.set_count(left);
                } else {
                    *slot = None;
                }
            }
        }
    }

    save_catalyst_conversion(&mut entity.nbt, &conversion);
    store_energy_child(&mut entity.nbt, "mercuryFluxHandler", buffer.energy);

    let game_time = level.game_time(&*host_ref).unwrap_or(0);
    if game_time % i64::from(PUSH_TICK_INTERVAL) == 0 {
        let is_enabled = enabled(&entity.nbt)
            && level.block_property(&*host_ref, pos, "enabled").as_deref() != Some("false");
        if is_enabled {
            push_flux(host_ref, level, pos, &mut buffer, &FluxDir::ALL, None);
            store_energy_child(&mut entity.nbt, "mercuryFluxHandler", buffer.energy);
        }
    }
}

fn tick_capacitor(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    sync_enabled_from_redstone(host_ref, level, pos);
    let energy = tank_energy(&entity.nbt, "mercuryFluxHandler").clamp(0, CAPACITOR_CAPACITY);
    store_energy_child(&mut entity.nbt, "mercuryFluxHandler", energy);
    let modes = load_side_modes(&entity.nbt);
    save_side_modes(&mut entity.nbt, &modes);

    let game_time = level.game_time(&*host_ref).unwrap_or(0);
    if game_time % i64::from(PUSH_TICK_INTERVAL) == 0
        && level.block_property(&*host_ref, pos, "enabled").as_deref() != Some("false")
    {
        let mut buffer = SimpleMercuryFluxHandler::with_energy(
            CAPACITOR_CAPACITY,
            CAPACITOR_CAPACITY,
            CAPACITOR_CAPACITY,
            energy,
        );
        push_flux(
            host_ref,
            level,
            pos,
            &mut buffer,
            &FluxDir::ALL,
            Some(&modes),
        );
        store_energy_child(&mut entity.nbt, "mercuryFluxHandler", buffer.energy);
    }
}

fn tick_mercury_emitter(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    let energy = tank_energy(&entity.nbt, "mercuryFluxHandler").clamp(0, MERCURY_EMITTER_CAPACITY);
    let game_time = level.game_time(&*host_ref).unwrap_or(0);
    if game_time % i64::from(MERCURY_EMITTER_TICK_INTERVAL) != 0
        || !enabled(&entity.nbt)
        || level.block_property(&*host_ref, pos, "enabled").as_deref() == Some("false")
    {
        store_energy_child(&mut entity.nbt, "mercuryFluxHandler", energy);
        return;
    }
    let Some(target) = selection::read_points(&entity.nbt, "selectedPoints")
        .first()
        .copied()
    else {
        store_energy_child(&mut entity.nbt, "mercuryFluxHandler", energy);
        return;
    };

    if energy >= MERCURY_EMITTER_FLUX_PER_TRANSFER {
        let accepted = insert_mercury_flux_at(
            host_ref,
            level,
            target,
            None,
            MERCURY_EMITTER_FLUX_PER_TRANSFER,
        );
        store_energy_child(&mut entity.nbt, "mercuryFluxHandler", energy - accepted);
    } else {
        store_energy_child(&mut entity.nbt, "mercuryFluxHandler", energy);
    }

    // Separate FE buffer (`energyStorage`), never converted to/from Mercury Flux.
    let energy_stored = compound_child(&entity.nbt, "energyStorage")
        .map(|child| host::int(child, "energy", 0).clamp(0, MERCURY_EMITTER_CAPACITY))
        .unwrap_or(0);
    if energy_stored >= MERCURY_EMITTER_FLUX_PER_TRANSFER {
        let accepted = if let Some(be) = host_ref.block_entity(level, target) {
            let mut target_nbt = be.data(&*host_ref).unwrap_or_default();
            let key = if compound_child(&target_nbt, "energyStorage").is_some() {
                Some("energyStorage")
            } else if compound_child(&target_nbt, "energyBuffer").is_some() {
                Some("energyBuffer")
            } else {
                None
            };
            if let Some(key) = key {
                let stored = tank_energy(&target_nbt, key).clamp(0, 1_000);
                let accepted = MERCURY_EMITTER_FLUX_PER_TRANSFER.min(1_000 - stored);
                if accepted > 0 {
                    store_energy_child(&mut target_nbt, key, stored + accepted);
                    let _ = be.merge_data(host_ref, target_nbt);
                    host_ref.block_entity_sync(target);
                }
                accepted
            } else {
                0
            }
        } else {
            0
        };
        store_energy_child(&mut entity.nbt, "energyStorage", energy_stored - accepted);
    } else {
        store_energy_child(&mut entity.nbt, "energyStorage", energy_stored);
    }
}

fn tick_caloric_emitter(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    sync_enabled_from_redstone(host_ref, level, pos);
    let energy = tank_energy(&entity.nbt, "mercuryFluxHandler").clamp(0, CALORIC_CAPACITY);
    store_energy_child(&mut entity.nbt, "mercuryFluxHandler", energy);

    let game_time = level.game_time(&*host_ref).unwrap_or(0);
    if game_time % i64::from(CALORIC_TICK_INTERVAL) != 0 {
        return;
    }
    let is_enabled = enabled(&entity.nbt)
        && level.block_property(&*host_ref, pos, "enabled").as_deref() != Some("false");
    if !is_enabled || energy < CALORIC_FLUX_PER_HEAT {
        return;
    }
    let Some(target) = selection::read_points(&entity.nbt, "selectedPoints")
        .first()
        .copied()
    else {
        return;
    };
    let Some(be) = host_ref.block_entity(level, target) else {
        return;
    };
    let Some(target_id) = be.kind(&*host_ref).map(|k| k.id(&*host_ref)) else {
        return;
    };
    let target_slots = be.data_slots(&*host_ref).unwrap_or_default();
    if !selection::heat_receiver(&target_id) || target_slots.get(4).copied().unwrap_or(0) != 0 {
        return;
    }
    let mut target_nbt = be.data(&*host_ref).unwrap_or_default();
    let hot_until = host::int(&target_nbt, "hotUntil", 0);
    if i64::from(hot_until) > game_time + i64::from(CALORIC_TICK_INTERVAL) {
        return;
    }
    let new_hot_until = game_time
        .saturating_add(i64::from(CALORIC_HEAT_TARGET_FOR_TICKS))
        .min(i64::from(i32::MAX)) as i32;
    host::set_int(&mut target_nbt, "hotUntil", new_hot_until);
    if be.merge_data(host_ref, target_nbt).unwrap_or(false) {
        store_energy_child(
            &mut entity.nbt,
            "mercuryFluxHandler",
            (energy - CALORIC_FLUX_PER_HEAT).max(0),
        );
        host_ref.block_entity_sync(target);
    }
}

pub fn tick(
    host_ref: &mut dyn ServerHost,
    entity_id: &str,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    let Some(level) = host::level_in_place(host_ref) else {
        return;
    };
    match entity_id {
        "theurgy:mercury_catalyst" => tick_catalyst(host_ref, level, pos, entity),
        "theurgy:mercury_capacitor" => tick_capacitor(host_ref, level, pos, entity),
        "theurgy:mercury_flux_emitter" => tick_mercury_emitter(host_ref, level, pos, entity),
        "theurgy:caloric_flux_emitter" => tick_caloric_emitter(host_ref, level, pos, entity),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::serde_json::json;

    #[test]
    fn split_sums_and_shares_remainder() {
        assert_eq!(split_evenly(10, 3), vec![4, 3, 3]);
        assert_eq!(split_evenly(0, 4), vec![0, 0, 0, 0]);
        assert_eq!(split_evenly(-5, 2), vec![0, 0]);
        assert!(split_evenly(7, 0).is_empty());
        assert_eq!(split_evenly(17, 5).iter().sum::<i32>(), 17);
    }

    #[test]
    fn total_to_push_scales_with_targets() {
        assert_eq!(total_to_push(0, 3), 0);
        assert_eq!(
            total_to_push(1_000_000, 1),
            PUSH_RATE_PER_SIDE_PER_TICK * PUSH_TICK_INTERVAL
        );
        assert_eq!(
            total_to_push(1_000_000, 5),
            PUSH_RATE_PER_SIDE_PER_TICK * PUSH_TICK_INTERVAL * 5
        );
    }

    #[test]
    fn simple_handler_respects_insert_and_extract_limits() {
        let mut h = SimpleMercuryFluxHandler::with_transfer(100, 20, 7);
        assert_eq!(h.insert(100), 20);
        assert_eq!(h.amount(), 20);
        assert_eq!(h.extract(100), 7);
        assert_eq!(h.amount(), 13);
        h.set(500);
        assert_eq!(h.amount(), 100);
        assert_eq!(h.insert(5), 0);
    }

    #[test]
    fn catalyst_progress_percent_tracks_converted_flux() {
        let mut c = CatalystConversion::new();
        c.begin(400, 10);
        assert_eq!(c.progress_percent(), 0);
        c.mercury_flux_to_convert = 300;
        assert_eq!(c.progress_percent(), 25);
        c.mercury_flux_to_convert = 0;
        assert_eq!(c.progress_percent(), 100);
        assert!(!c.is_processing());
    }

    #[test]
    fn catalyst_convert_tick_caps_by_rate_and_space() {
        let mut buffer = SimpleMercuryFluxHandler::new(100);
        let mut conv = CatalystConversion::new();
        conv.begin(1_000, 10);
        assert_eq!(conv.convert_tick(&mut buffer), 10);
        assert_eq!(buffer.energy, 10);
        assert_eq!(conv.mercury_flux_to_convert, 990);
        buffer.set(95);
        assert_eq!(conv.convert_tick(&mut buffer), 5);
        assert_eq!(conv.mercury_flux_to_convert, 985);
        buffer.set(100);
        assert_eq!(conv.convert_tick(&mut buffer), 0);
    }

    #[test]
    fn side_mode_cycle_and_ordinal_round_trip() {
        let mut m = SideMode::None;
        for want in [SideMode::Input, SideMode::Output, SideMode::Both, SideMode::None] {
            m = m.cycle();
            assert_eq!(m, want);
            assert_eq!(SideMode::from_ordinal(m.ordinal()), m);
        }
        assert!(SideMode::Both.can_insert() && SideMode::Both.can_extract());
        assert!(SideMode::Input.can_insert() && !SideMode::Input.can_extract());
        assert!(SideMode::Output.can_extract() && !SideMode::Output.can_insert());
        assert!(!SideMode::None.can_insert() && !SideMode::None.can_extract());
    }

    #[test]
    fn direction_opposite_offset_and_name_round_trip() {
        for d in FluxDir::ALL {
            assert_eq!(d.opposite().opposite(), d);
            assert_eq!(d.offset(d.opposite().offset([0, 0, 0])), [0, 0, 0]);
            assert_eq!(FluxDir::from_name(d.name()), Some(d));
        }
        assert_eq!(FluxDir::from_name("NOPE"), None);
        assert_eq!(FluxDir::from_index(0), Some(FluxDir::Down));
        assert_eq!(FluxDir::from_index(6), None);
    }

    #[test]
    fn side_modes_round_trip_through_nbt() {
        let mut modes = default_capacitor_side_modes();
        modes[0].1 = SideMode::Both;
        modes[3].1 = SideMode::None;
        let mut nbt = Compound::new();
        save_side_modes(&mut nbt, &modes);
        assert_eq!(load_side_modes(&nbt), modes);
        assert_eq!(load_side_modes(&Compound::new()), default_capacitor_side_modes());
    }

    #[test]
    fn recipe_flux_budget_reads_json_fields() {
        assert_eq!(
            recipe_flux_budget(&json!({"mercuryFlux": 800, "mercuryFluxPerTick": 20})),
            Some((800, 20))
        );
        assert_eq!(
            recipe_flux_budget(&json!({"totalMercuryFlux": 3200, "mercuryFluxPerTick": 20})),
            Some((3200, 20))
        );
        assert_eq!(recipe_flux_budget(&json!({})), None);
    }
}
