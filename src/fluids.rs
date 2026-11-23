// SPDX-License-Identifier: MIT
//! Sal ammoniac fluid registration, model hook, accumulator and tank ticks.
//!
//! Horizon 0a875da natively simulates and meshes world fluids declared in
//! `data/theurgy/horizon/fluid/sal_ammoniac.json` + `register_fluid_models`,
//! so no water-conversion hack or mesher mixin is required.

use mod_api::serde_json::Value;
use mod_api::{
    Direction, FLUIDS, FluidKey, FluidModel, FluidResource, FluidStack, Hand, Host,
    HostTransaction, ItemPlace, ItemResource, ItemStack, LevelRef, ModBlockEntityView, PlayerRef,
    RegisterFluidModelsEvent, ResourceHandler, ServerHost, Transaction, fluid_util,
};

use crate::engine::recipes::Tank;
use crate::{host, recipes};

pub const SAL_AMMONIAC: &str = "theurgy:sal_ammoniac";
pub const ACCUMULATOR: &str = "theurgy:sal_ammoniac_accumulator";
pub const TANK: &str = "theurgy:sal_ammoniac_tank";

const WATER_CAPACITY: i32 = 10_000;
const TANK_CAPACITY: i32 = 2_000;
const DEFAULT_ACCUMULATION_TIME: i32 = 100;

/// Register the animated still/flowing/overlay sprites for `theurgy:sal_ammoniac`.
pub fn register_fluid_models(event: &mut RegisterFluidModelsEvent<'_>) {
    let mut model = FluidModel::default();
    model.fluid = SAL_AMMONIAC.to_owned();
    model.still = Some("theurgy:block/sal_ammoniac/still".to_owned());
    model.flowing = Some("theurgy:block/sal_ammoniac/flow".to_owned());
    model.overlay = Some("theurgy:block/sal_ammoniac/overlay".to_owned());
    event.models.push(model);
}

/// Item type (id + components, not count) of the solute slot, for
/// `MonitoredItemStackHandler.onContentTypeChanged`.
fn item_type_hash(host_ref: &dyn Host, stack: Option<&ItemStack>) -> i32 {
    let Some(stack) = stack else {
        return 0;
    };
    let mut hash = 0x811c_9dc5u32;
    let id = stack.item().id(host_ref);
    let desc = format!("{id}|{}|{:?}", stack.damage(), stack.components());
    for byte in desc.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    (hash | 1) as i32
}

pub fn fluid_field(value: Option<&Value>) -> Option<(String, i32)> {
    let value = value?;
    let id = value.get("ingredient").or_else(|| value.get("id"))?.as_str()?;
    let amount = value.get("amount")?.as_i64()?.clamp(0, i64::from(i32::MAX)) as i32;
    Some((id.to_owned(), amount))
}

pub fn fluid_matches(wanted: &str, actual: Option<&str>) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    if let Some(tag) = wanted.strip_prefix('#') {
        return match tag {
            "minecraft:water" => actual == "minecraft:water",
            "theurgy:sal_ammoniac" | "theurgy:solvent" => {
                actual == SAL_AMMONIAC || actual == "theurgy:flowing_sal_ammoniac"
            }
            _ => actual == tag,
        };
    }
    actual == wanted
}

pub fn fits(output: &Tank, fluid: &str, amount: i32) -> bool {
    amount > 0
        && amount <= output.capacity
        && output.fluid.as_deref().is_none_or(|f| f == fluid)
        && output.amount.saturating_add(amount) <= output.capacity
}

fn output_tank(host_ref: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<Tank> {
    let below = [pos[0], pos[1] - 1, pos[2]];
    let be = host_ref.block_entity(level, below)?;
    if be.kind(host_ref).map(|k| k.id(host_ref)).as_deref() != Some(TANK) {
        return None;
    }
    let data = be.data(host_ref)?;
    Some(host::tank_from_nbt(&data, 0, TANK_CAPACITY))
}

fn fill_below(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    fluid_id: &str,
    amount: i32,
) -> bool {
    let below = [pos[0], pos[1] - 1, pos[2]];
    if host::block_id_at(&*host_ref, level, below).as_deref() != Some(TANK) {
        return false;
    }
    if amount <= 0 {
        return false;
    }
    let Some(fluid_key) = host_ref.fluid_key(fluid_id) else {
        return false;
    };
    let Some(cap) = level.capability(host_ref, &FLUIDS, below, Some(Direction::Up)) else {
        return false;
    };
    let resource = FluidResource::of(&FluidStack::new(fluid_key, 1));
    let want = amount as u32;
    let ok = host_ref
        .transaction(|host_ref, tx| {
            let inserted = cap.insert(host_ref, tx, 0, &resource, want);
            if inserted == want {
                Ok(())
            } else {
                Err(())
            }
        })
        .is_ok();
    if ok {
        host_ref.block_entity_sync(below);
    }
    ok
}

/// The sal ammoniac bucket: the item a sal ammoniac fluid travels in.
pub const BUCKET_ITEM: &str = "theurgy:sal_ammoniac_bucket";
const EMPTY_BUCKET: &str = "minecraft:bucket";
const BUCKET: u32 = 1000;

/// The sal ammoniac bucket as a fluid container (`ITEM_FLUIDS`): one tank of 1000 mB, filled and
/// drained whole, an empty vanilla bucket taking sal ammoniac. The game answers `ITEM_FLUIDS`
/// for vanilla buckets only, so without this provider a click with this bucket finds no fluid
/// container and vanilla's bucket places the fluid in the world next to the block.
pub struct SalAmmoniacBucket {
    pub place: ItemPlace,
    pub fluid: FluidKey,
}

impl SalAmmoniacBucket {
    fn holds_item(&self, host: &mut dyn ServerHost, item: &str) -> bool {
        self.place
            .stack(host)
            .is_some_and(|stack| stack.item().id(&*host) == item)
    }

    fn accepts(&self, resource: &FluidResource) -> bool {
        !resource.to_stack(1).has_components() && resource.fluid().key() == Some(self.fluid)
    }

    /// Swaps the one bucket in the place for `item`, which the place's exchange splits off a
    /// stack for. `BUCKET` when it goes through, else 0.
    fn exchange(&self, host: &mut dyn ServerHost, tx: &Transaction, item: &str) -> u32 {
        let Some(key) = crate::host::item_key(&*host, item) else {
            return 0;
        };
        let bucket = ItemResource::of(&ItemStack::of(key));
        match self.place.exchange(host, tx, &bucket, 1) {
            1 => BUCKET,
            _ => 0,
        }
    }
}

impl ResourceHandler<FluidResource> for SalAmmoniacBucket {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }

    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<FluidResource> {
        (index == 0 && self.holds_item(host, BUCKET_ITEM))
            .then(|| FluidResource::of(&FluidStack::new(self.fluid, 1)))
    }

    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        match self.resource(host, index) {
            Some(_) => BUCKET,
            None => 0,
        }
    }

    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: &FluidResource) -> u32 {
        if index == 0 && self.accepts(resource) && self.holds_item(host, EMPTY_BUCKET) {
            BUCKET
        } else {
            0
        }
    }

    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &FluidResource,
        amount: u32,
    ) -> u32 {
        if index == 0
            && amount >= BUCKET
            && self.accepts(resource)
            && self.holds_item(host, EMPTY_BUCKET)
        {
            self.exchange(host, tx, BUCKET_ITEM)
        } else {
            0
        }
    }

    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &FluidResource,
        amount: u32,
    ) -> u32 {
        let held = self.resource(host, index);
        match held {
            Some(held) if held == *resource && amount >= BUCKET => {
                self.exchange(host, tx, EMPTY_BUCKET)
            }
            _ => 0,
        }
    }
}

/// An empty vanilla bucket in `player`'s `hand` takes one bucket of sal ammoniac out of the tanks
/// of the block at `pos` as `side` reaches them. The game's own `ITEM_FLUIDS` of an empty bucket
/// fills with vanilla fluids only, so `fluid_util::interact_with_fluid_handler` moves nothing
/// here. All of it or nothing; true when the bucket was filled.
pub fn fill_empty_bucket(
    host_ref: &mut dyn ServerHost,
    player: PlayerRef,
    hand: Hand,
    level: LevelRef,
    pos: [i32; 3],
    side: Option<Direction>,
) -> bool {
    let place = ItemPlace::PlayerHand {
        player: player.uuid(),
        hand,
    };
    let Some(fluid) = host_ref.fluid_key(SAL_AMMONIAC) else {
        return false;
    };
    let bucket = SalAmmoniacBucket { place, fluid };
    if !bucket.holds_item(host_ref, EMPTY_BUCKET) {
        return false;
    }
    let Some(tanks) = level.capability(host_ref, &FLUIDS, pos, side) else {
        return false;
    };
    let held = (0..tanks.size(host_ref)).find_map(|index| {
        let resource = tanks.resource(host_ref, index)?;
        (bucket.accepts(&resource) && tanks.amount(host_ref, index) >= BUCKET)
            .then_some((index, resource))
    });
    let Some((index, resource)) = held else {
        return false;
    };
    let filled = host_ref.transaction(|host_ref, tx| {
        if tanks.extract(host_ref, tx, index, &resource, BUCKET) != BUCKET
            || bucket.exchange(host_ref, tx, BUCKET_ITEM) != BUCKET
        {
            return Err(());
        }
        Ok(())
    });
    if filled.is_err() {
        return false;
    }
    let at = pos.map(|c| f64::from(c) + 0.5);
    fluid_util::trigger_sound_and_game_event(host_ref, &resource, level, at, Some(player), true);
    true
}

/// `FluidStorageHelper.drain(handler, Integer.MAX_VALUE, null)`: empties every tank of the
/// apparatus at `pos`. True when anything was drained.
pub fn void_tanks(host_ref: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3]) -> bool {
    let Some(tanks) = level.capability(host_ref, &FLUIDS, pos, None) else {
        return false;
    };
    let mut drained = false;
    for index in 0..tanks.size(host_ref) {
        let Some(resource) = tanks.resource(host_ref, index) else {
            continue;
        };
        let amount = tanks.amount(host_ref, index);
        if amount == 0 {
            continue;
        }
        let result = host_ref.transaction(|host_ref, tx| {
            (tanks.extract(host_ref, tx, index, &resource, amount) == amount)
                .then_some(())
                .ok_or(())
        });
        drained |= result.is_ok();
    }
    drained
}

#[derive(Clone)]
struct Accumulation {
    solute: Option<String>,
    evaporant: Option<(String, i32)>,
    result: (String, i32),
    time: i32,
}

fn accumulation_recipes() -> Vec<Accumulation> {
    recipes::of_type("theurgy:accumulation")
        .iter()
        .filter_map(|r| {
            Some(Accumulation {
                solute: r
                    .json
                    .get("solute")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                evaporant: fluid_field(r.json.get("evaporant")),
                result: fluid_field(r.json.get("result"))?,
                time: r.time,
            })
        })
        .collect()
}

fn solute_matches(host_ref: &dyn Host, solute: &str, stack: &ItemStack) -> bool {
    recipes::matches_value(host_ref, &Value::String(solute.to_owned()), stack)
}

/// Accumulator and tank server tick.
pub fn tick(
    host_ref: &mut dyn ServerHost,
    entity_id: &str,
    pos: [i32; 3],
    entity: &mut ModBlockEntityView<'_>,
) {
    if entity_id == TANK {
        let mut tank = host::tank(entity, 0, TANK_CAPACITY);
        tank.amount = tank.amount.clamp(0, TANK_CAPACITY);
        if tank.amount == 0 {
            tank.fluid = None;
        }
        host::save_tanks(entity, std::slice::from_ref(&tank));
        entity.data.clear();
        entity.data.extend([tank.amount, tank.capacity]);
        return;
    }
    if entity_id != ACCUMULATOR {
        return;
    }
    let Some(level) = host::level_in_place(host_ref) else {
        return;
    };
    if entity.items.is_empty() {
        entity.items.push(None);
    }
    let mut water = host::tank(entity, 0, WATER_CAPACITY);
    let mut state = host::crafting(&entity.nbt, &entity.data);
    let recipes = accumulation_recipes();
    let mut matched: Option<Accumulation> = None;
    for recipe in &recipes {
        let stack = entity.items.first().and_then(Option::as_ref);
        let solute_ok = match (&recipe.solute, stack) {
            (Some(solute), Some(stack)) => solute_matches(&*host_ref, solute, stack),
            (None, None) => true,
            _ => false,
        };
        let evaporant_ok = recipe.evaporant.as_ref().is_none_or(|(wanted, amount)| {
            fluid_matches(wanted, water.fluid.as_deref()) && water.amount >= *amount
        });
        if solute_ok && evaporant_ok {
            matched = Some(recipe.clone());
            break;
        }
    }
    let next_time = matched
        .as_ref()
        .map_or(DEFAULT_ACCUMULATION_TIME, |r| r.time);
    let item_type = item_type_hash(&*host_ref, entity.items.first().and_then(Option::as_ref));
    if entity.data.get(7) == Some(&1) && entity.data.get(6) != Some(&item_type) {
        state.input_changed(next_time);
    }
    let below = [pos[0], pos[1] - 1, pos[2]];
    let has_tank = host::block_id_at(&*host_ref, level, below).as_deref() == Some(TANK);
    if has_tank {
        let can_craft = matched.as_ref().is_some_and(|recipe| {
            output_tank(&*host_ref, level, pos)
                .is_some_and(|output| fits(&output, &recipe.result.0, recipe.result.1))
        });
        let has_input = water.amount > 0 || entity.items.first().is_some_and(Option::is_some);
        let effects = state.tick(true, has_input, can_craft, next_time);
        if effects.craft_attempt
            && let Some(recipe) = matched
            && fill_below(host_ref, level, pos, &recipe.result.0, recipe.result.1)
        {
            if recipe.solute.is_some()
                && let Some(slot) = entity.items.first_mut().and_then(Option::as_mut)
            {
                if slot.count() > 1 {
                    let left = slot.count() - 1;
                    let _ = slot.set_count(left);
                } else {
                    entity.items[0] = None;
                }
            }
            if let Some((_, amount)) = recipe.evaporant {
                water.amount = (water.amount - amount).max(0);
                if water.amount == 0 {
                    water.fluid = None;
                }
            }
        }
    }
    let item_type = item_type_hash(&*host_ref, entity.items.first().and_then(Option::as_ref));
    if has_tank {
        host::save_tanks(entity, std::slice::from_ref(&water));
    }
    let fields = &mut **entity;
    host::save_crafting(&mut fields.nbt, &mut fields.data, &state);
    fields.data.clear();
    fields.data.extend([
        state.progress,
        state.total_time,
        water.amount,
        water.capacity,
        i32::from(state.is_processing),
        i32::from(state.could_craft_last_tick),
        item_type,
        1,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::serde_json::json;

    #[test]
    fn tank_holds_two_buckets_and_never_mixes() {
        let empty = Tank {
            fluid: None,
            amount: 0,
            capacity: 2_000,
        };
        assert!(fits(&empty, "minecraft:water", 2_000));
        assert!(!fits(&empty, "minecraft:water", 2_001));
        let water = Tank {
            fluid: Some("minecraft:water".into()),
            amount: 1_000,
            capacity: 2_000,
        };
        assert!(fits(&water, "minecraft:water", 1_000));
        assert!(!fits(&water, "theurgy:sal_ammoniac", 1_000));
    }

    #[test]
    fn water_tag_matches_only_water() {
        assert!(fluid_matches("#minecraft:water", Some("minecraft:water")));
        assert!(!fluid_matches(
            "#minecraft:water",
            Some("theurgy:sal_ammoniac")
        ));
        assert!(fluid_matches(
            "#theurgy:sal_ammoniac",
            Some("theurgy:sal_ammoniac")
        ));
        assert!(fluid_matches(
            "#theurgy:solvent",
            Some("theurgy:flowing_sal_ammoniac")
        ));
    }

    #[test]
    fn result_and_evaporant_fields_parse() {
        assert_eq!(
            fluid_field(Some(&json!({"id":"minecraft:lava","amount":1000}))),
            Some(("minecraft:lava".into(), 1000))
        );
        assert_eq!(
            fluid_field(Some(&json!({"ingredient":"#minecraft:water","amount":1000}))),
            Some(("#minecraft:water".into(), 1000))
        );
    }
}
