// SPDX-License-Identifier: MIT
//! SelectionBehaviour and the three emitter selection states (mercury, caloric,
//! sulfuric flux emitters).
//!
//! Pending points are keyed by acting player UUID and emitter kind; points are
//! never discovered by scanning the world and never persisted on the item.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use mod_api::{
    Compound, Flow, Host, ItemStack, LeftClickBlockEvent, LevelRef, RightClickBlockEvent,
    ServerHost, Tag,
};

use crate::host;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Emitter {
    Mercury,
    Caloric,
    Sulfuric,
}

#[derive(Clone, Debug, Default)]
struct Pending {
    points: Vec<[i32; 3]>,
    target: Option<[i32; 3]>,
    result: Option<[i32; 3]>,
}

static PENDING: OnceLock<Mutex<HashMap<(u128, Emitter), Pending>>> = OnceLock::new();

fn pending() -> &'static Mutex<HashMap<(u128, Emitter), Pending>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_pending() -> std::sync::MutexGuard<'static, HashMap<(u128, Emitter), Pending>> {
    pending().lock().unwrap_or_else(|error| error.into_inner())
}

pub fn clear() {
    lock_pending().clear();
}

pub fn emitter_id(id: &str) -> Option<Emitter> {
    match id {
        "theurgy:mercury_flux_emitter" => Some(Emitter::Mercury),
        "theurgy:caloric_flux_emitter" => Some(Emitter::Caloric),
        "theurgy:sulfuric_flux_emitter" => Some(Emitter::Sulfuric),
        _ => None,
    }
}

fn emitter(host_ref: &dyn Host, item: &ItemStack) -> Option<Emitter> {
    emitter_id(&item.item().id(host_ref))
}

pub fn heat_receiver(block: &str) -> bool {
    matches!(
        block,
        "theurgy:calcination_oven"
            | "theurgy:distiller"
            | "theurgy:liquefaction_cauldron"
            | "theurgy:incubator"
    )
}

pub fn mercury_target(block: &str) -> bool {
    matches!(
        block,
        "theurgy:mercury_catalyst"
            | "theurgy:mercury_capacitor"
            | "theurgy:mercury_flux_emitter"
            | "theurgy:caloric_flux_emitter"
            | "theurgy:sulfuric_flux_emitter"
            | "theurgy:logistics_mercury_flux_connector"
    )
}

/// Whether `block` is a target an emitter of `kind` selects on a click. A mercury emitter placed
/// while sneaking is a block placement, not a selection.
pub fn target_accepts(kind: Emitter, block: &str, sneaking: bool) -> bool {
    match kind {
        Emitter::Mercury => !sneaking && mercury_target(block),
        Emitter::Caloric => heat_receiver(block),
        Emitter::Sulfuric => pedestal(block).is_some(),
    }
}

fn pedestal(block: &str) -> Option<&'static str> {
    match block {
        "theurgy:reformation_source_pedestal" => Some("SOURCE"),
        "theurgy:reformation_target_pedestal" => Some("TARGET"),
        "theurgy:reformation_result_pedestal" => Some("RESULT"),
        _ => None,
    }
}

/// `SelectionBehaviour.onRightClickBlock`. A second click toggles a point off;
/// target/result selections replace the previous point, sources are unlimited.
pub fn right_click_block(
    host_ref: &mut dyn ServerHost,
    event: &mut RightClickBlockEvent<'_>,
) -> Flow {
    if event.cancelled {
        return Flow::Continue;
    }
    let click = event.click;
    let Some(kind) = click.stack.and_then(|s| emitter(&*host_ref, s)) else {
        return Flow::Continue;
    };
    let sneaking = host_ref
        .player(click.player)
        .is_some_and(|p| p.is_secondary_use_active(&*host_ref).unwrap_or(false));
    if kind == Emitter::Mercury && sneaking {
        return Flow::Continue;
    }
    let Some(level) = host_ref.level(click.dimension.key().id()) else {
        return Flow::Continue;
    };
    let Some(block) = host::block_id_at(&*host_ref, level, click.pos) else {
        return Flow::Continue;
    };
    let sulfuric_type = (kind == Emitter::Sulfuric)
        .then(|| pedestal(&block))
        .flatten();
    let valid = match kind {
        Emitter::Mercury => mercury_target(&block),
        Emitter::Caloric => heat_receiver(&block),
        Emitter::Sulfuric => sulfuric_type.is_some(),
    };
    if !valid {
        return Flow::Continue;
    }

    let pos = click.pos;
    let mut pending = lock_pending();
    let selection = pending.entry((click.player, kind)).or_default();
    match kind {
        Emitter::Mercury | Emitter::Caloric => {
            if selection.points.first() == Some(&pos) {
                selection.points.clear();
            } else {
                selection.points.clear();
                selection.points.push(pos);
            }
        }
        Emitter::Sulfuric => match sulfuric_type.expect("validated pedestal") {
            "SOURCE" => {
                if let Some(index) = selection.points.iter().position(|point| *point == pos) {
                    selection.points.remove(index);
                } else {
                    selection.points.push(pos);
                }
            }
            "TARGET" => {
                selection.target = (selection.target != Some(pos)).then_some(pos);
            }
            "RESULT" => {
                selection.result = (selection.result != Some(pos)).then_some(pos);
            }
            _ => unreachable!(),
        },
    }
    event.cancelled = true;
    Flow::Cancel
}

/// `SelectionBehaviour.tick`: changing the held item abandons pending points.
pub fn inventory_tick(
    host_ref: &mut dyn ServerHost,
    stack: &ItemStack,
    selected: bool,
) {
    if !selected {
        return;
    }
    let Some(player) = host_ref.players().acting_player() else {
        return;
    };
    let Some(player_id) = player.entity().uuid(host_ref) else {
        return;
    };
    let current = emitter(&*host_ref, stack);
    lock_pending().retain(|(owner, kind), _| *owner != player_id || Some(*kind) == current);
}

/// `SelectionBehaviour.onLeftClickBlock`: remove only a previously selected point.
pub fn left_click_block(
    host_ref: &mut dyn ServerHost,
    event: &mut LeftClickBlockEvent,
) -> Flow {
    if event.cancelled {
        return Flow::Continue;
    }
    let Some(player) = host_ref.players().acting_player() else {
        return Flow::Continue;
    };
    let Some(player_id) = player.entity().uuid(host_ref) else {
        return Flow::Continue;
    };
    let selected_slot = player.selected_slot(&*host_ref).unwrap_or(0) as u32;
    let held = player
        .inventory_slots(&*host_ref)
        .and_then(|slots| {
            slots
                .into_iter()
                .find(|(slot, _)| *slot == selected_slot)
                .map(|(_, stack)| stack)
        });
    let Some(kind) = held.as_ref().and_then(|s| emitter(&*host_ref, s)) else {
        return Flow::Continue;
    };
    let Some(level) = player.level(&*host_ref) else {
        return Flow::Continue;
    };
    let pos = event.pos;
    let Some(block) = host::block_id_at(&*host_ref, level, pos) else {
        return Flow::Continue;
    };
    let sulfuric_type = (kind == Emitter::Sulfuric)
        .then(|| pedestal(&block))
        .flatten();
    let mut pending = lock_pending();
    let Some(selection) = pending.get_mut(&(player_id, kind)) else {
        return Flow::Continue;
    };
    let removed = match kind {
        Emitter::Mercury | Emitter::Caloric => {
            let found = selection.points.first() == Some(&pos);
            if found {
                selection.points.clear();
            }
            found
        }
        Emitter::Sulfuric => match sulfuric_type {
            Some("SOURCE") => {
                let old = selection.points.len();
                selection.points.retain(|point| *point != pos);
                old != selection.points.len()
            }
            Some("TARGET") => selection.target.take_if(|target| *target == pos).is_some(),
            Some("RESULT") => selection.result.take_if(|result| *result == pos).is_some(),
            _ => false,
        },
    };
    if removed {
        event.cancelled = true;
        return Flow::Cancel;
    }
    Flow::Continue
}

pub fn closer_than(origin: [i32; 3], point: [i32; 3], range: i32) -> bool {
    let dx = i64::from(origin[0] - point[0]);
    let dy = i64::from(origin[1] - point[1]);
    let dz = i64::from(origin[2] - point[2]);
    dx * dx + dy * dy + dz * dz < i64::from(range * range)
}

pub fn point_tag(pos: [i32; 3], kind: Option<&str>) -> Tag {
    let mut point = Compound::new();
    point.insert("blockPos".into(), Tag::IntArray(pos.to_vec()));
    if let Some(kind) = kind {
        point.insert("type".into(), Tag::String(kind.into()));
    }
    Tag::Compound(point)
}

/// `BlockPos.CODEC`, with read compatibility for the old flat `{x,y,z}` form.
pub fn read_point_tag(tag: &Tag) -> Option<[i32; 3]> {
    let Tag::Compound(point) = tag else {
        return None;
    };
    if let Some(tag) = point.get(&"blockPos".into()) {
        match tag {
            Tag::IntArray(values) if values.len() == 3 => {
                return Some([values[0], values[1], values[2]]);
            }
            Tag::List(values) if values.len() == 3 => {
                let mut out = [0; 3];
                for (index, value) in values.iter().enumerate() {
                    let Tag::Int(value) = value else {
                        return None;
                    };
                    out[index] = *value;
                }
                return Some(out);
            }
            _ => return None,
        }
    }
    let x = host::int(point, "x", i32::MIN);
    let y = host::int(point, "y", i32::MIN);
    let z = host::int(point, "z", i32::MIN);
    (x != i32::MIN && y != i32::MIN && z != i32::MIN).then_some([x, y, z])
}

pub fn read_point(nbt: &Compound, key: &str) -> Option<[i32; 3]> {
    read_point_tag(nbt.get(&key.into())?)
}

pub fn read_points(nbt: &Compound, key: &str) -> Vec<[i32; 3]> {
    match nbt.get(&key.into()) {
        Some(Tag::List(points)) => points.iter().filter_map(read_point_tag).collect(),
        _ => Vec::new(),
    }
}

/// `SelectionBehaviour.onPlace`: transfer pending points into the placed emitter BE.
pub fn placed(
    host_ref: &mut dyn ServerHost,
    level: LevelRef,
    player_id: u128,
    block_id: &str,
    pos: [i32; 3],
) {
    let Some(kind) = emitter_id(block_id) else {
        return;
    };
    let Some(mut selection) = lock_pending().remove(&(player_id, kind)) else {
        return;
    };
    let range = if kind == Emitter::Sulfuric { 16 } else { 8 };
    selection
        .points
        .retain(|point| closer_than(pos, *point, range));
    selection.target = selection.target.filter(|point| closer_than(pos, *point, range));
    selection.result = selection.result.filter(|point| closer_than(pos, *point, range));

    let Some(be) = host_ref.block_entity(level, pos) else {
        return;
    };
    let mut nbt = be.data(&*host_ref).unwrap_or_default();
    match kind {
        Emitter::Mercury | Emitter::Caloric => {
            let points = selection
                .points
                .iter()
                .map(|point| point_tag(*point, None))
                .collect();
            nbt.insert("selectedPoints".into(), Tag::List(points));
        }
        Emitter::Sulfuric => {
            let sources = selection
                .points
                .iter()
                .map(|point| point_tag(*point, Some("SOURCE")))
                .collect();
            nbt.insert("sourcePedestals".into(), Tag::List(sources));
            if let Some(target) = selection.target {
                nbt.insert("targetPedestal".into(), point_tag(target, Some("TARGET")));
            } else {
                nbt.remove(&"targetPedestal".into());
            }
            if let Some(result) = selection.result {
                nbt.insert("resultPedestal".into(), point_tag(result, Some("RESULT")));
            } else {
                nbt.remove(&"resultPedestal".into());
            }
        }
    }
    let _ = be.replace_data(host_ref, nbt);
    host_ref.block_entity_sync(pos);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_block_pos_codec_round_trip() {
        let tag = point_tag([4, -5, 6], Some("SOURCE"));
        assert_eq!(read_point_tag(&tag), Some([4, -5, 6]));
        let mut nbt = Compound::new();
        nbt.insert("sourcePedestals".into(), Tag::List(vec![tag]));
        assert_eq!(read_points(&nbt, "sourcePedestals"), vec![[4, -5, 6]]);
    }

    #[test]
    fn closer_than_matches_block_pos_boundary() {
        assert!(closer_than([0, 0, 0], [15, 0, 0], 16));
        assert!(!closer_than([0, 0, 0], [16, 0, 0], 16));
        assert!(!closer_than([0, 0, 0], [8, 0, 0], 8));
    }

    #[test]
    fn legacy_flat_points_remain_readable() {
        let mut point = Compound::new();
        point.insert("x".into(), Tag::Int(1));
        point.insert("y".into(), Tag::Int(2));
        point.insert("z".into(), Tag::Int(3));
        assert_eq!(read_point_tag(&Tag::Compound(point)), Some([1, 2, 3]));
    }
}
