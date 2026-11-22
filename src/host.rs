// SPDX-License-Identifier: MIT
//! Shared NBT, crafting, tank and SDK host helpers used across Theurgy modules.

use mod_api::{
    BlockKey, BlockState, Compound, Direction, Host, Identifier, ItemKey, ItemStack, LevelHost,
    LevelRef, ModBlockEntityView, ServerHost, Tag,
};

use crate::engine::crafting::Crafting;
use crate::engine::recipes::Tank;

pub fn int(nbt: &Compound, key: &str, default: i32) -> i32 {
    match nbt.get(&key.into()) {
        Some(Tag::Int(v)) => *v,
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Byte(v)) => i32::from(*v),
        Some(Tag::Long(v)) => {
            if *v > i64::from(i32::MAX) {
                i32::MAX
            } else if *v < i64::from(i32::MIN) {
                i32::MIN
            } else {
                *v as i32
            }
        }
        _ => default,
    }
}

pub fn set_int(nbt: &mut Compound, key: &str, value: i32) {
    nbt.insert(key.into(), Tag::Int(value));
}

pub fn bool(nbt: &Compound, key: &str) -> bool {
    int(nbt, key, 0) != 0
}

pub fn string(nbt: &Compound, key: &str) -> Option<String> {
    match nbt.get(&key.into()) {
        Some(Tag::String(v)) => Some(v.to_string_lossy()),
        _ => None,
    }
}

pub fn set_string(nbt: &mut Compound, key: &str, value: &str) {
    nbt.insert(key.into(), Tag::String(value.into()));
}

pub fn clamp_non_negative(value: i32) -> i32 {
    value.max(0)
}

pub fn crafting(nbt: &Compound, data: &[i32]) -> Crafting {
    Crafting {
        progress: clamp_non_negative(int(nbt, "progress", 0)),
        total_time: clamp_non_negative(int(nbt, "totalTime", 0)),
        // Java's saveAdditional stores only the two shorts; keep runtime flags in menu data.
        is_processing: data.get(4).copied().unwrap_or(0) != 0,
        could_craft_last_tick: data.get(5).copied().unwrap_or(0) != 0,
    }
}

pub fn save_crafting(nbt: &mut Compound, data: &mut Vec<i32>, state: &Crafting) {
    nbt.insert("progress".into(), Tag::Short(state.progress as i16));
    nbt.insert("totalTime".into(), Tag::Short(state.total_time as i16));
    nbt.remove(&"isProcessing".into());
    nbt.remove(&"couldCraftLastTick".into());
    if data.len() < 6 {
        data.resize(6, 0);
    }
    data[4] = i32::from(state.is_processing);
    data[5] = i32::from(state.could_craft_last_tick);
}

pub fn tank(entity: &ModBlockEntityView<'_>, index: usize, capacity: i32) -> Tank {
    tank_from_nbt(&entity.nbt, index, capacity)
}

pub fn tank_from_nbt(nbt: &Compound, index: usize, capacity: i32) -> Tank {
    let capacity = clamp_non_negative(capacity);
    let empty = || Tank {
        fluid: None,
        amount: 0,
        capacity,
    };
    let Some(Tag::List(list)) = nbt.get(&"fluids".into()) else {
        return empty();
    };
    let Some(Tag::Compound(map)) = list.get(index) else {
        return empty();
    };
    let fluid = match map.get(&"Fluid".into()) {
        Some(Tag::String(v)) => {
            let v = v.to_string_lossy();
            if v.is_empty() || v == "minecraft:empty" {
                None
            } else {
                Some(v)
            }
        }
        _ => None,
    };
    let amount = int(map, "Amount", 0).clamp(0, capacity);
    if fluid.is_none() || amount == 0 {
        return empty();
    }
    Tank {
        fluid,
        amount,
        capacity,
    }
}

pub fn save_tanks(entity: &mut ModBlockEntityView<'_>, tanks: &[Tank]) {
    save_tanks_nbt(&mut entity.nbt, tanks);
}

pub fn save_tanks_nbt(nbt: &mut Compound, tanks: &[Tank]) {
    let list = tanks
        .iter()
        .map(|t| {
            let mut map = Compound::new();
            let fluid = match t.fluid.as_deref() {
                Some(id) if !id.is_empty() && id != "minecraft:empty" && t.amount > 0 => id,
                _ => "minecraft:empty",
            };
            let amount = if fluid == "minecraft:empty" {
                0
            } else {
                t.amount.clamp(0, clamp_non_negative(t.capacity))
            };
            map.insert("Fluid".into(), Tag::String(fluid.into()));
            map.insert("Amount".into(), Tag::Int(amount));
            Tag::Compound(map)
        })
        .collect();
    nbt.insert("fluids".into(), Tag::List(list));
}

/// The level currently ticking on the server.
pub fn level_in_place(host: &dyn ServerHost) -> Option<LevelRef> {
    host.levels()
        .list_levels()
        .into_iter()
        .find(|level| level.in_place(host))
}

pub fn item_key(host: &dyn Host, id: &str) -> Option<ItemKey> {
    host.item_key(&Identifier::parse(id).ok()?)
}

pub fn block_key(host: &dyn Host, id: &str) -> Option<BlockKey> {
    host.block_key(id)
}

pub fn make_stack(host: &dyn Host, id: &str, count: u32) -> Option<ItemStack> {
    let key = item_key(host, id)?;
    ItemStack::new(key, count)
}

pub fn item_max_stack(host: &dyn Host, stack: &ItemStack) -> u32 {
    host.max_stack_size(stack.item()).unwrap_or(64).max(1)
}

pub fn item_max_stack_id(host: &dyn Host, id: &str) -> u32 {
    item_key(host, id)
        .and_then(|key| host.max_stack_size(key))
        .unwrap_or(64)
        .max(1)
}

pub fn in_item_tag(host: &dyn Host, item: ItemKey, tag: &str) -> bool {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    let Ok(id) = Identifier::parse(tag) else {
        return false;
    };
    let Some(key) = host.tags().key(&id) else {
        return false;
    };
    host.tags().is_in_tag(item, key).unwrap_or(false)
}

pub fn in_block_tag(host: &dyn Host, block: BlockKey, tag: &str) -> bool {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    let Ok(id) = Identifier::parse(tag) else {
        return false;
    };
    let Some(key) = host.tags().key(&id) else {
        return false;
    };
    host.tags()
        .blocks(key)
        .is_some_and(|blocks| blocks.contains(&block))
}

pub fn block_id_at(host: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<String> {
    level.block(host, pos).map(|key| key.id(host))
}

/// Extract `key=value` from a serialized `BlockState` string (`mod:block[a=1,b=2]`).
pub fn parse_state_property<'a>(serialized: &'a str, key: &str) -> Option<&'a str> {
    let start = serialized.find('[')?;
    let end = serialized.rfind(']')?;
    if end <= start {
        return None;
    }
    for pair in serialized[start + 1..end].split(',') {
        if let Some((k, v)) = pair.split_once('=')
            && k == key
        {
            return Some(v);
        }
    }
    None
}

pub fn level_state_property(level: &dyn LevelHost, state: BlockState, key: &str) -> Option<String> {
    let serialized = level.state_string(state)?;
    parse_state_property(&serialized, key).map(str::to_owned)
}

pub fn facing_name_to_dir(name: &str) -> Option<u8> {
    match name {
        "down" => Some(0),
        "up" => Some(1),
        "north" => Some(2),
        "south" => Some(3),
        "west" => Some(4),
        "east" => Some(5),
        _ => None,
    }
}

pub fn dir_to_facing_name(dir: u8) -> &'static str {
    match dir {
        0 => "down",
        1 => "up",
        2 => "north",
        3 => "south",
        4 => "west",
        5 => "east",
        _ => "north",
    }
}

pub fn direction_to_ordinal(dir: Direction) -> u8 {
    match dir {
        Direction::Down => 0,
        Direction::Up => 1,
        Direction::North => 2,
        Direction::South => 3,
        Direction::West => 4,
        Direction::East => 5,
    }
}

pub fn ordinal_to_direction(dir: u8) -> Direction {
    match dir {
        0 => Direction::Down,
        1 => Direction::Up,
        2 => Direction::North,
        3 => Direction::South,
        4 => Direction::West,
        5 => Direction::East,
        _ => Direction::North,
    }
}

pub fn dir_offset(dir: u8) -> [i32; 3] {
    match dir {
        0 => [0, -1, 0],
        1 => [0, 1, 0],
        2 => [0, 0, -1],
        3 => [0, 0, 1],
        4 => [-1, 0, 0],
        5 => [1, 0, 0],
        _ => [0, 0, 0],
    }
}

pub fn offset_pos(pos: [i32; 3], dir: u8) -> [i32; 3] {
    let d = dir_offset(dir);
    [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]]
}

pub fn opposite_dir(dir: u8) -> u8 {
    match dir {
        0 => 1,
        1 => 0,
        2 => 3,
        3 => 2,
        4 => 5,
        5 => 4,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nbt_int_reads_all_numeric_tags() {
        let mut nbt = Compound::new();
        nbt.insert("a".into(), Tag::Int(42));
        nbt.insert("b".into(), Tag::Short(12));
        nbt.insert("c".into(), Tag::Byte(1));
        nbt.insert("d".into(), Tag::Long(i64::from(i32::MAX) + 10));
        assert_eq!(int(&nbt, "a", 0), 42);
        assert_eq!(int(&nbt, "b", 0), 12);
        assert_eq!(int(&nbt, "c", 0), 1);
        assert_eq!(int(&nbt, "d", 0), i32::MAX);
        assert_eq!(int(&nbt, "missing", 7), 7);
    }

    #[test]
    fn parse_state_property_extracts_exact_keys() {
        let s = "theurgy:fermentation_vat[facing=east,has_output=true,open=false]";
        assert_eq!(parse_state_property(s, "facing"), Some("east"));
        assert_eq!(parse_state_property(s, "has_output"), Some("true"));
        assert_eq!(parse_state_property(s, "open"), Some("false"));
        assert_eq!(parse_state_property(s, "lit"), None);
        assert_eq!(parse_state_property("minecraft:stone", "facing"), None);
    }

    #[test]
    fn crafting_and_tank_roundtrip_on_nbt() {
        let mut nbt = Compound::new();
        let mut data = Vec::new();
        let state = Crafting {
            progress: 15,
            total_time: 100,
            is_processing: true,
            could_craft_last_tick: true,
        };
        save_crafting(&mut nbt, &mut data, &state);
        assert_eq!(crafting(&nbt, &data), state);

        let t = Tank {
            fluid: Some("theurgy:sal_ammoniac".into()),
            amount: 750,
            capacity: 2_000,
        };
        save_tanks_nbt(&mut nbt, std::slice::from_ref(&t));
        assert_eq!(tank_from_nbt(&nbt, 0, 2_000), t);
        assert_eq!(tank_from_nbt(&nbt, 1, 2_000).fluid, None);
    }
}
