// SPDX-License-Identifier: MIT
//! SelectionBehaviour and the three emitter selection states (mercury, caloric,
//! sulfuric flux emitters).
//!
//! Pending points are keyed by acting player UUID and emitter kind; points are
//! never discovered by scanning the world and never persisted on the item.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use mod_api::text::TextStyle;
use mod_api::{
    BlockShapeKind, ClientHost, Compound, Flow, Hand, Host, ItemPlace, ItemStack,
    LeftClickBlockEvent, LevelRef, RightClickBlockEvent, ServerHost, SubmitCustomGeometryEvent,
    Tag, Text, WorldDepth, WorldLine,
};

use crate::{client, host};

/// Outline colour of a mercury or caloric emitter's target (`*SelectedPoint.color`).
const FLUX_TARGET_COLOR: u32 = 0xDDC166;
/// Outline width in pixels; upstream's `Outliner` draws 1/16 of a block.
const OUTLINE_WIDTH: f32 = 2.5;

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

impl Pending {
    fn count(&self) -> usize {
        self.points.len() + usize::from(self.target.is_some()) + usize::from(self.result.is_some())
    }
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

/// Outline and message colour of a selected point (`SulfuricFluxEmitterSelectedPoint.Type`).
pub fn point_color(kind: Emitter, block: &str) -> u32 {
    match (kind, pedestal(block)) {
        (Emitter::Sulfuric, Some("SOURCE")) => 0xFF00FF,
        (Emitter::Sulfuric, Some("TARGET")) => 0x0000FF,
        (Emitter::Sulfuric, Some(_)) => 0x008000,
        _ => FLUX_TARGET_COLOR,
    }
}

fn emitter_name(kind: Emitter) -> &'static str {
    match kind {
        Emitter::Mercury => "mercury_flux_emitter",
        Emitter::Caloric => "caloric_flux_emitter",
        Emitter::Sulfuric => "sulfuric_flux_emitter",
    }
}

fn colored(color: &str) -> TextStyle {
    let mut style = TextStyle::default();
    style.color = Some(color.to_owned());
    style
}

fn block_name(block: &str) -> Text {
    let (namespace, path) = block.split_once(':').unwrap_or(("minecraft", block));
    Text::translate(format!("block.{namespace}.{path}"), Vec::new())
}

fn overlay(host_ref: &mut dyn ServerHost, player_id: u128, message: Text) {
    if let Some(player) = host_ref.player(player_id) {
        let _ = player.action_bar(host_ref, message);
    }
}

/// `SelectionBehaviour.getModeMessage`: what the emitter will do with the block just selected,
/// in the point's colour with the block's name in white.
fn mode_message(kind: Emitter, block: &str) -> Text {
    Text::translate(
        "theurgy.behaviour.selection.mode",
        vec![
            Text::translate(
                format!("theurgy.behaviour.selection.mode.{}", emitter_name(kind)),
                Vec::new(),
            ),
            block_name(block).with_style(colored("white")),
        ],
    )
    .with_style(colored(&format!("#{:06X}", point_color(kind, block))))
}

/// The lang key suffix of `displaySummary` for a sulfuric emitter, by the number of linked
/// source, target and result pedestals. Upstream sends each complaint in turn to the same
/// overlay, so the last one that applies is the one shown.
pub fn sulfuric_summary(sources: usize, targets: usize, results: usize) -> &'static str {
    if sources + targets + results == 0 {
        ".no_selection"
    } else if results == 0 {
        ".no_result"
    } else if sources == 0 {
        ".no_sources"
    } else if targets == 0 {
        ".no_target"
    } else {
        ""
    }
}

/// `displaySummary`: what the placed emitter ended up linked to.
fn summary_message(host_ref: &dyn Host, level: LevelRef, kind: Emitter, selection: &Pending) -> Text {
    let key = format!("theurgy.behaviour.selection.summary.{}", emitter_name(kind));
    let red = colored("red");
    let white = colored("white");
    match kind {
        Emitter::Mercury | Emitter::Caloric => match selection.points.first() {
            None => Text::translate(format!("{key}.no_selection"), Vec::new()).with_style(red),
            Some(point) => {
                let block = host::block_id_at(host_ref, level, *point).unwrap_or_default();
                Text::translate(key, vec![block_name(&block)]).with_style(white)
            }
        },
        Emitter::Sulfuric => {
            let counts = [
                selection.points.len(),
                usize::from(selection.target.is_some()),
                usize::from(selection.result.is_some()),
            ];
            let suffix = sulfuric_summary(counts[0], counts[1], counts[2]);
            if !suffix.is_empty() {
                return Text::translate(format!("{key}{suffix}"), Vec::new()).with_style(red);
            }
            let count = |index: usize, color: &str| {
                Text::literal(counts[index].to_string()).with_style(colored(color))
            };
            Text::translate(
                key,
                vec![count(0, "dark_purple"), count(1, "blue"), count(2, "green")],
            )
            .with_style(white)
        }
    }
}

/// The world-space bounds of the outline shape of the block at `pos` (`shape.bounds().move(pos)`).
/// None for a block without a shape.
fn outline_box(host_ref: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<[f64; 6]> {
    let state = level.block_state(host_ref, pos)?;
    let boxes = level.block_shape(host_ref, state, pos, BlockShapeKind::Outline)?;
    let bounds = union_bounds(&boxes)?;
    Some(std::array::from_fn(|i| bounds[i] + f64::from(pos[i % 3])))
}

/// The smallest box around `boxes` (`VoxelShape.bounds`).
pub fn union_bounds(boxes: &[[f64; 6]]) -> Option<[f64; 6]> {
    let mut iter = boxes.iter();
    let mut out = *iter.next()?;
    for b in iter {
        for a in 0..3 {
            out[a] = out[a].min(b[a]);
            out[a + 3] = out[a + 3].max(b[a + 3]);
        }
    }
    Some(out)
}

/// Sends the player's pending points to their client, which outlines them while the emitter
/// is held (`SelectionBehaviour.prepareRender`).
fn sync(host_ref: &mut dyn ServerHost, player_id: u128) {
    let points: Vec<(Emitter, [i32; 3])> = lock_pending()
        .iter()
        .filter(|((owner, _), _)| *owner == player_id)
        .flat_map(|((_, kind), selection)| {
            selection
                .points
                .iter()
                .chain(&selection.target)
                .chain(&selection.result)
                .map(|point| (*kind, *point))
                .collect::<Vec<_>>()
        })
        .collect();
    let level = host_ref.player(player_id).and_then(|p| p.level(&*host_ref));
    let mut outlines = Vec::new();
    if let Some(level) = level {
        for (kind, point) in points {
            let Some(block) = host::block_id_at(&*host_ref, level, point) else {
                continue;
            };
            if let Some(bounds) = outline_box(&*host_ref, level, point) {
                outlines.push((bounds, point_color(kind, &block)));
            }
        }
    }
    client::send_selection(host_ref, player_id, outlines);
}

static OUTLINES: Mutex<Vec<([f64; 6], u32)>> = Mutex::new(Vec::new());

/// Client: the outlines the server last sent for this player's pending points.
pub fn set_outlines(outlines: Vec<([f64; 6], u32)>) {
    *OUTLINES.lock().unwrap_or_else(|error| error.into_inner()) = outlines;
}

/// The twelve edges of an axis-aligned box, as `(from, to)` corners.
pub fn box_edges(b: [f64; 6]) -> [([f64; 3], [f64; 3]); 12] {
    let corner = |i: usize| [b[(i & 1) * 3], b[1 + ((i >> 1) & 1) * 3], b[2 + ((i >> 2) & 1) * 3]];
    // Each edge joins two corners that differ in exactly one axis bit.
    let pairs = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
    pairs.map(|(from, to)| (corner(from), corner(to)))
}

/// Client `submit_custom_geometry`: the coloured outlines of the selected blocks, drawn only
/// while an emitter is in the main hand.
pub fn draw_outlines(host_ref: &dyn ClientHost, event: &mut SubmitCustomGeometryEvent<'_>) {
    let outlines = OUTLINES.lock().unwrap_or_else(|error| error.into_inner()).clone();
    if outlines.is_empty() {
        return;
    }
    let holding = client::local_hand_stack(host_ref, Hand::Main)
        .is_some_and(|stack| emitter(host_ref, &stack).is_some());
    if !holding {
        return;
    }
    let lines: Vec<WorldLine> = outlines
        .iter()
        .flat_map(|(bounds, color)| {
            box_edges(*bounds).map(|(from, to)| {
                WorldLine::new(from, to, 0xFF00_0000 | color, OUTLINE_WIDTH, WorldDepth::Tested)
            })
        })
        .collect();
    let _ = event.draw.lines(&lines);
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
    // True when the click selected the block, false when it took a selected block off.
    let selected = {
        let mut pending = lock_pending();
        let selection = pending.entry((click.player, kind)).or_default();
        match kind {
            Emitter::Mercury | Emitter::Caloric => {
                let was = selection.points.first() == Some(&pos);
                selection.points.clear();
                if !was {
                    selection.points.push(pos);
                }
                !was
            }
            Emitter::Sulfuric => match sulfuric_type.expect("validated pedestal") {
                "SOURCE" => {
                    if let Some(index) = selection.points.iter().position(|point| *point == pos) {
                        selection.points.remove(index);
                        false
                    } else {
                        selection.points.push(pos);
                        true
                    }
                }
                "TARGET" => {
                    selection.target = (selection.target != Some(pos)).then_some(pos);
                    selection.target.is_some()
                }
                "RESULT" => {
                    selection.result = (selection.result != Some(pos)).then_some(pos);
                    selection.result.is_some()
                }
                _ => unreachable!(),
            },
        }
    };
    // Taking a block off is silent upstream; only a new selection names its mode.
    if selected {
        overlay(host_ref, click.player, mode_message(kind, &block));
    }
    sync(host_ref, click.player);
    event.cancelled = true;
    Flow::Cancel
}

/// `SelectionBehaviour.tick`: a player who no longer holds the emitter abandons its pending
/// points, and a point whose block stopped being a valid target is dropped (`isValid`).
pub fn tick(host_ref: &mut dyn ServerHost) {
    let keys: Vec<(u128, Emitter)> = lock_pending().keys().copied().collect();
    if keys.is_empty() {
        return;
    }
    let mut changed: Vec<u128> = Vec::new();
    for (player_id, kind) in keys {
        let held = ItemPlace::PlayerHand { player: player_id, hand: Hand::Main }.stack(host_ref);
        let holding = held.as_ref().and_then(|stack| emitter(&*host_ref, stack)) == Some(kind);
        let level = host_ref.player(player_id).and_then(|p| p.level(&*host_ref));
        let Some(level) = level.filter(|_| holding) else {
            lock_pending().remove(&(player_id, kind));
            changed.push(player_id);
            continue;
        };
        let valid = |point: &[i32; 3], wanted: Option<&str>| {
            host::block_id_at(&*host_ref, level, *point).is_some_and(|block| match kind {
                Emitter::Mercury => mercury_target(&block),
                Emitter::Caloric => heat_receiver(&block),
                Emitter::Sulfuric => pedestal(&block) == wanted,
            })
        };
        let mut pending = lock_pending();
        let Some(selection) = pending.get_mut(&(player_id, kind)) else {
            continue;
        };
        let before = (selection.points.len(), selection.target, selection.result);
        selection.points.retain(|point| valid(point, Some("SOURCE")));
        selection.target = selection.target.filter(|point| valid(point, Some("TARGET")));
        selection.result = selection.result.filter(|point| valid(point, Some("RESULT")));
        if before != (selection.points.len(), selection.target, selection.result) {
            changed.push(player_id);
        }
    }
    changed.dedup();
    for player_id in changed {
        sync(host_ref, player_id);
    }
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
    let removed = {
        let mut pending = lock_pending();
        let Some(selection) = pending.get_mut(&(player_id, kind)) else {
            return Flow::Continue;
        };
        remove_point(selection, kind, sulfuric_type, pos)
    };
    if removed {
        sync(host_ref, player_id);
        event.cancelled = true;
        return Flow::Cancel;
    }
    Flow::Continue
}

fn remove_point(
    selection: &mut Pending,
    kind: Emitter,
    sulfuric_type: Option<&str>,
    pos: [i32; 3],
) -> bool {
    match kind {
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
    }
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
    // Placing with nothing selected still reports "has no target", as upstream does.
    let mut selection = lock_pending().remove(&(player_id, kind)).unwrap_or_default();
    let range = if kind == Emitter::Sulfuric { 16 } else { 8 };
    let before = selection.count();
    selection
        .points
        .retain(|point| closer_than(pos, *point, range));
    selection.target = selection.target.filter(|point| closer_than(pos, *point, range));
    selection.result = selection.result.filter(|point| closer_than(pos, *point, range));
    // `SelectionBehaviour.onPlace`: points out of range are dropped and counted; otherwise
    // the summary says what the emitter is now linked to.
    let removed = before - selection.count();
    let message = if removed > 0 {
        Text::translate(
            "theurgy.behaviour.selection.outside_range",
            vec![Text::literal(removed.to_string())],
        )
        .with_style(colored("red"))
    } else {
        summary_message(&*host_ref, level, kind, &selection)
    };
    overlay(host_ref, player_id, message);
    sync(host_ref, player_id);

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
    fn points_are_coloured_by_what_they_are() {
        assert_eq!(point_color(Emitter::Sulfuric, "theurgy:reformation_source_pedestal"), 0xFF00FF);
        assert_eq!(point_color(Emitter::Sulfuric, "theurgy:reformation_target_pedestal"), 0x0000FF);
        assert_eq!(point_color(Emitter::Sulfuric, "theurgy:reformation_result_pedestal"), 0x008000);
        assert_eq!(point_color(Emitter::Caloric, "theurgy:distiller"), 0xDDC166);
        assert_eq!(point_color(Emitter::Mercury, "theurgy:mercury_capacitor"), 0xDDC166);
    }

    #[test]
    fn sulfuric_summary_names_the_last_missing_part() {
        assert_eq!(sulfuric_summary(0, 0, 0), ".no_selection");
        assert_eq!(sulfuric_summary(2, 1, 0), ".no_result");
        assert_eq!(sulfuric_summary(0, 1, 1), ".no_sources");
        assert_eq!(sulfuric_summary(0, 0, 1), ".no_sources");
        assert_eq!(sulfuric_summary(3, 0, 1), ".no_target");
        assert_eq!(sulfuric_summary(3, 1, 1), "");
    }

    #[test]
    fn an_outline_is_the_bounds_of_the_whole_shape() {
        let boxes = [[0.25, 0.0, 0.25, 0.75, 0.5, 0.75], [0.0, 0.5, 0.0, 1.0, 1.0, 1.0]];
        assert_eq!(union_bounds(&boxes), Some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]));
        assert_eq!(union_bounds(&[]), None);
        let edges = box_edges([0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
        // Every edge runs along exactly one axis, and each axis has four of them.
        let mut per_axis = [0; 3];
        for (from, to) in edges {
            let moved: Vec<usize> = (0..3).filter(|a| from[*a] != to[*a]).collect();
            assert_eq!(moved.len(), 1, "{from:?} -> {to:?}");
            per_axis[moved[0]] += 1;
        }
        assert_eq!(per_axis, [4, 4, 4]);
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
