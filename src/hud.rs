// SPDX-License-Identifier: MIT
//! The two HUDs of Theurgy 26.3:
//! - `InWorldHUD`: a box at the top centre for the apparatus under the crosshair. Its title is
//!   known to the client; its progress, flux and tank lines are a snapshot the server answers
//!   every few ticks (`MessageRequestInWorldHUD` / `MessageSyncInWorldHUD`), so they are never
//!   the client's stale copy of the block entity.
//! - `ItemHUD`: the held mercurial wand's mode at the bottom left, with "current -> new" for the
//!   block under the crosshair (`MercurialWandItemMode.appendHUDText`).

use std::sync::{Arc, Mutex, OnceLock};

use mod_api::client::hud_layout::PoseOp;
use mod_api::{
    ClientHost, Compound, DrawContext, GuiDraw, GuiLayerDrawEvent, Hand, Hit, HudMode,
    IPayloadContext, ItemStack, PayloadCodec, PayloadId, PayloadRegistrar, PayloadWork, ServerHost,
    Tag, Text, ToClient, ToServer,
};

use crate::{client, config, filter, flux, host, interactions, items, registry};

pub const LAYER_ID: &str = "theurgy:hud";

/// Ticks between two requests for the looked-at block (`InWorldHUD.REQUEST_INTERVAL_TICKS`).
const REQUEST_INTERVAL_TICKS: u32 = 5;
/// How far from the player a block may be for the server to describe it.
const MAX_REQUEST_DISTANCE: f64 = 16.0;

const WHITE: u32 = 0xFFFF_FFFF;
const GRAY: u32 = 0xFFAA_AAAA;
const GREEN: u32 = 0xFF55_FF55;
const YELLOW: u32 = 0xFFFF_FF55;
const RED: u32 = 0xFFFF_5555;
const AQUA: u32 = 0xFF55_FFFF;
/// `ItemHUD`'s text colour.
const ITEM_HUD: u32 = 0xFFC8_C8C8;

// ---------------------------------------------------------------------------
// Payloads
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PayloadCodec)]
pub struct HudRequestPayload {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// What the server knows about the apparatus at a position. `progress` is a percentage, or -1
/// while nothing is being crafted; `flux` is empty or `[amount, capacity]`; the three tank
/// lists run in parallel, one entry per tank, with an empty name for an empty tank.
#[derive(Clone, Debug, Default, PartialEq, Eq, PayloadCodec)]
pub struct HudSnapshotPayload {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub progress: i32,
    pub flux: Vec<i32>,
    pub fluids: Vec<String>,
    pub amounts: Vec<i32>,
    pub capacities: Vec<i32>,
}

static REQUEST: OnceLock<ToServer<HudRequestPayload>> = OnceLock::new();
static SNAPSHOT: OnceLock<ToClient<HudSnapshotPayload>> = OnceLock::new();

struct AnswerWork([i32; 3]);
impl PayloadWork for AnswerWork {
    fn run(&self, cx: &mut dyn IPayloadContext) -> Result<(), String> {
        if let (Ok(player), Ok(host_ref)) = (cx.player(), cx.server_host())
            && let Some(uuid) = player.entity().uuid(host_ref)
        {
            answer(host_ref, uuid, self.0);
        }
        Ok(())
    }
}

pub fn register(view: &mut PayloadRegistrar<'_>) {
    let request = view.play_to_server_typed::<HudRequestPayload>(
        PayloadId("theurgy:hud_request".into()),
        |value, cx| {
            let _ = cx.enqueue_work(Arc::new(AnswerWork([value.x, value.y, value.z])));
            Ok(())
        },
    );
    let _ = REQUEST.set(request);
    let snapshot = view.play_to_client_typed::<HudSnapshotPayload>(
        PayloadId("theurgy:hud_snapshot".into()),
        |value, _cx| {
            accept(value);
            Ok(())
        },
    );
    let _ = SNAPSHOT.set(snapshot);
}

// ---------------------------------------------------------------------------
// Server: the snapshot
// ---------------------------------------------------------------------------

/// Blocks whose block entity crafts (`HasCraftingBehaviour`), so `data[4]` is its
/// "is processing" flag.
fn crafts(block: &str) -> bool {
    matches!(
        block,
        "theurgy:calcination_oven"
            | "theurgy:distiller"
            | "theurgy:liquefaction_cauldron"
            | "theurgy:fermentation_vat"
            | "theurgy:digestion_vat"
            | "theurgy:incubator"
            | "theurgy:sal_ammoniac_accumulator"
            | "theurgy:sulfuric_flux_emitter"
    )
}

/// `CraftingBehaviour.progressPercent`.
pub fn progress_percent(progress: i32, total: i32) -> i32 {
    if total <= 0 {
        return 0;
    }
    (i64::from(progress) * 100 / i64::from(total)).clamp(0, 100) as i32
}

/// The snapshot of the apparatus whose block entity data is `nbt` and menu data is `data`:
/// the lines of `GenericCraftingProgress`-, `MercuryFluxHandler`- and
/// `FluidStorageInWorldHUDProvider`.
pub fn snapshot_of(block: &str, pos: [i32; 3], nbt: &Compound, data: &[i32]) -> HudSnapshotPayload {
    let mut out = HudSnapshotPayload {
        x: pos[0],
        y: pos[1],
        z: pos[2],
        progress: -1,
        ..HudSnapshotPayload::default()
    };
    if block == "theurgy:mercury_catalyst" {
        // `MercuryCatalystCraftingBehaviour.progressPercent`: flux converted of the total.
        let left = host::int(nbt, "mercuryFluxToConvert", 0);
        let total = host::int(nbt, "totalMercuryFluxToConvert", 0);
        if left > 0 {
            out.progress = progress_percent(total - left, total);
        }
    } else if crafts(block) && data.get(4).copied().unwrap_or(0) != 0 {
        out.progress =
            progress_percent(host::int(nbt, "progress", 0), host::int(nbt, "totalTime", 0));
    }
    if let Some(capacity) = flux::hud_capacity(block) {
        let amount = match nbt.get(&"mercuryFluxHandler".into()) {
            Some(Tag::Compound(handler)) => host::int(handler, "energy", 0),
            _ => 0,
        };
        out.flux = vec![amount.clamp(0, capacity), capacity];
    }
    if let Some(tanks) = registry::fluids(block) {
        for (index, capacity) in tanks.tanks.iter().enumerate() {
            let capacity = i32::try_from(*capacity).unwrap_or(i32::MAX);
            let tank = host::tank_from_nbt(nbt, index, capacity);
            out.fluids.push(tank.fluid.unwrap_or_default());
            out.amounts.push(tank.amount);
            out.capacities.push(capacity);
        }
    }
    out
}

/// Server: describe the block at `pos` to `player_id`, who asked for it. In singleplayer there
/// is no remote session to answer through, and the client half of this process takes it directly.
pub fn answer(host_ref: &mut dyn ServerHost, player_id: u128, pos: [i32; 3]) {
    let Some(player) = host_ref.player(player_id) else {
        return;
    };
    let Some(level) = player.level(&*host_ref) else {
        return;
    };
    let near = player.entity().position(&*host_ref).is_some_and(|at| {
        (0..3).all(|a| (at[a] - f64::from(pos[a]) - 0.5).abs() <= MAX_REQUEST_DISTANCE)
    });
    if !near {
        return;
    }
    let Some((entity_pos, block)) = interactions::resolve_entity_pos(&*host_ref, level, pos) else {
        return;
    };
    if !block.starts_with("theurgy:") {
        return;
    }
    let Some(entity) = host_ref.block_entity(level, entity_pos) else {
        return;
    };
    let nbt = entity.data(&*host_ref).unwrap_or_default();
    let data = entity.data_slots(&*host_ref).unwrap_or_default();
    let payload = snapshot_of(&block, pos, &nbt, &data);
    let sent = SNAPSHOT
        .get()
        .is_some_and(|token| token.send(host_ref, player, &payload).is_ok());
    if !sent {
        accept(payload);
    }
}

// ---------------------------------------------------------------------------
// Client: what is being looked at
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Watch {
    pos: Option<[i32; 3]>,
    snapshot: Option<HudSnapshotPayload>,
    wait: u32,
}

static WATCH: Mutex<Watch> = Mutex::new(Watch { pos: None, snapshot: None, wait: 0 });

fn watch() -> std::sync::MutexGuard<'static, Watch> {
    WATCH.lock().unwrap_or_else(|error| error.into_inner())
}

fn accept(snapshot: HudSnapshotPayload) {
    let mut watch = watch();
    if watch.pos == Some([snapshot.x, snapshot.y, snapshot.z]) {
        watch.snapshot = Some(snapshot);
    }
}

fn looked_at(host_ref: &dyn ClientHost) -> Option<(Hit, String)> {
    let hit = host_ref.raycast(6.0)?;
    let block = hit.block.id(host_ref);
    block.starts_with("theurgy:").then_some((hit, block))
}

/// `InWorldHUD.tick`: follow the crosshair and ask the server about the block every few ticks.
pub fn client_tick(host_ref: &mut dyn ClientHost) {
    let settings = config::get();
    let target = (settings.enable_in_world_hud && host_ref.playing())
        .then(|| looked_at(host_ref))
        .flatten()
        .map(|(hit, _)| [hit.block_x, hit.block_y, hit.block_z]);
    let ask = {
        let mut watch = watch();
        if watch.pos != target {
            *watch = Watch { pos: target, snapshot: None, wait: 0 };
        }
        match target {
            Some(pos) if watch.wait == 0 => {
                watch.wait = REQUEST_INTERVAL_TICKS;
                Some(pos)
            }
            _ => {
                watch.wait = watch.wait.saturating_sub(1);
                None
            }
        }
    };
    if let Some([x, y, z]) = ask {
        let sent = REQUEST
            .get()
            .is_some_and(|token| token.send(host_ref, &HudRequestPayload { x, y, z }).is_ok());
        if !sent {
            client::queue_hud_request([x, y, z]);
        }
    }
}

// ---------------------------------------------------------------------------
// Text: translated templates filled with coloured values
// ---------------------------------------------------------------------------

/// One line of a HUD: runs of text, each in its own colour.
type Line = Vec<(String, u32)>;

/// Fills a translation template's `%s`, `%1$s` and `%%` with `values`, each in its own colour;
/// the template's own text is `base`.
pub fn fill(template: &str, values: &[(String, u32)], base: u32) -> Line {
    let mut out: Line = Vec::new();
    let mut literal = String::new();
    let mut next = 0usize;
    let mut rest = template;
    while let Some(at) = rest.find('%') {
        literal.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        let (index, skip) = if after.starts_with('%') {
            literal.push('%');
            rest = &after[1..];
            continue;
        } else if after.starts_with('s') {
            next += 1;
            (next - 1, 1)
        } else if digits > 0 && after[digits..].starts_with("$s") {
            (after[..digits].parse::<usize>().unwrap_or(1).saturating_sub(1), digits + 2)
        } else {
            literal.push('%');
            rest = after;
            continue;
        };
        if !literal.is_empty() {
            out.push((std::mem::take(&mut literal), base));
        }
        if let Some(value) = values.get(index) {
            out.push(value.clone());
        }
        rest = &after[skip..];
    }
    literal.push_str(rest);
    if !literal.is_empty() {
        out.push((literal, base));
    }
    out
}

fn translated(host_ref: &dyn ClientHost, key: &str, values: &[(String, u32)], base: u32) -> Line {
    let template = host_ref.translate(key).unwrap_or_else(|| key.to_owned());
    fill(&template, values, base)
}

fn word(host_ref: &dyn ClientHost, key: &str) -> String {
    host_ref.translate(key).unwrap_or_else(|| key.rsplit('.').next().unwrap_or(key).to_owned())
}

/// A block's display name as the language has it (`block.<namespace>.<path>`).
fn block_name(host_ref: &dyn ClientHost, block: &str) -> String {
    match block.split_once(':') {
        Some((namespace, path)) => host_ref
            .translate(&format!("block.{namespace}.{path}"))
            .unwrap_or_else(|| block.to_owned()),
        None => block.to_owned(),
    }
}

/// `GenericCraftingProgressInWorldHUDProvider.progressBar`: ten segments.
pub fn progress_bar(percent: i32) -> String {
    let filled = (i64::from(percent) * 10 / 100).clamp(0, 10) as usize;
    "█".repeat(filled) + &"░".repeat(10 - filled)
}

// ---------------------------------------------------------------------------
// In-world HUD
// ---------------------------------------------------------------------------

fn snapshot_lines(host_ref: &dyn ClientHost, snapshot: &HudSnapshotPayload) -> Vec<Line> {
    let mut lines = Vec::new();
    if snapshot.progress >= 0 {
        lines.push(translated(
            host_ref,
            "theurgy.misc.crafting_progress",
            &[(progress_bar(snapshot.progress), GREEN), (snapshot.progress.to_string(), GRAY)],
            GRAY,
        ));
    }
    if let [amount, capacity] = snapshot.flux[..] {
        lines.push(translated(
            host_ref,
            "theurgy.jei.misc.mercury_flux",
            &[(format!("{amount} / {capacity}"), GRAY)],
            GRAY,
        ));
    }
    for (index, fluid) in snapshot.fluids.iter().enumerate() {
        let amount = snapshot.amounts.get(index).copied().unwrap_or(0);
        let capacity = snapshot.capacities.get(index).copied().unwrap_or(0);
        let name = if fluid.is_empty() || amount <= 0 {
            word(host_ref, "theurgy.misc.empty")
        } else {
            host_ref.fluid_name(fluid)
        };
        let mut line: Line = vec![(format!("{name} ("), GRAY)];
        line.extend(translated(
            host_ref,
            "theurgy.misc.amount_capacity_millibuckets",
            &[(amount.to_string(), GRAY), (capacity.to_string(), GRAY)],
            GRAY,
        ));
        line.push((")".to_owned(), GRAY));
        lines.push(line);
    }
    lines
}

/// The stacks the looked-at apparatus holds, from the client's copy of its block entity.
fn held_stacks(host_ref: &dyn ClientHost, pos: [i32; 3]) -> Option<Vec<ItemStack>> {
    let level = host_ref.local_player()?.level(host_ref)?;
    let (entity_pos, _) = interactions::resolve_entity_pos(host_ref, level, pos)?;
    let entity = host_ref.block_entity(level, entity_pos)?;
    let mut stacks: Vec<ItemStack> =
        (0..4).filter_map(|slot| entity.item_slot(host_ref, slot).ok().flatten()).collect();
    // A client's copy has no slots of its own: the server mirrors them into `Items`.
    if stacks.is_empty()
        && let Some(nbt) = entity.data(host_ref)
        && let Some(Tag::List(items)) = nbt.get(&"Items".into())
    {
        stacks.extend(items.iter().filter_map(|item| filter::stack_from_template(host_ref, item)));
    }
    Some(stacks)
}

fn line_text(line: &Line) -> String {
    line.iter().map(|(text, _)| text.as_str()).collect()
}

fn draw_line(draw: &mut dyn DrawContext, line: &Line, at: [f32; 2], scale: f32) {
    let mut x = 0.0;
    let mut draws = vec![GuiDraw::Push(vec![PoseOp::Translate(at[0], at[1]), PoseOp::Scale(scale, scale)])];
    for (text, color) in line {
        draws.push(GuiDraw::Text {
            text: Text::literal(text.clone()),
            x: x as i32,
            y: 0,
            color: *color,
            shadow: true,
        });
        x += draw.text_width(text);
    }
    draws.push(GuiDraw::Pop);
    let _ = draw.gui_paint(&draws);
}

/// `InWorldHUD.render`: title, the server's lines and the stacks, in a box at the top centre.
fn draw_in_world(host_ref: &dyn ClientHost, draw: &mut dyn DrawContext) {
    let Some((hit, block)) = looked_at(host_ref) else {
        return;
    };
    let pos = [hit.block_x, hit.block_y, hit.block_z];
    // Only an apparatus has a HUD: a block with a block entity (its lower half for a tall one).
    let Some(stacks) = held_stacks(host_ref, pos) else {
        return;
    };
    let mut lines: Vec<Line> = vec![vec![(block_name(host_ref, &block), WHITE)]];
    {
        let watch = watch();
        if watch.pos == Some(pos)
            && let Some(snapshot) = watch.snapshot.as_ref()
        {
            lines.extend(snapshot_lines(host_ref, snapshot));
        }
    }
    let widths: Vec<f32> = lines.iter().map(|line| draw.text_width(&line_text(line))).collect();
    let (line_height, stack_step, pad, top) = (10.0, 18.0, 4.0, 4.0);
    let widest = widths.iter().copied().fold(0.0_f32, f32::max);
    let row_width = stacks.len() as f32 * stack_step;
    let box_width = widest.max(row_width) + 2.0 * pad;
    let text_height = lines.len() as f32 * line_height;
    let row_height = if stacks.is_empty() { 0.0 } else { stack_step };
    let box_height = text_height + row_height + 2.0 * pad - 1.0;
    let left = ((draw.size()[0] - box_width) / 2.0).max(4.0);
    // A tooltip's frame: dark fill inside a one pixel purple border.
    draw.fill([left, top, box_width, box_height], 0xF010_0010);
    draw.fill_gradient([left, top, 1.0, box_height], 0x5050_00FF, 0x5028_007F);
    draw.fill_gradient([left + box_width - 1.0, top, 1.0, box_height], 0x5050_00FF, 0x5028_007F);
    draw.fill([left, top, box_width, 1.0], 0x5050_00FF);
    draw.fill([left, top + box_height - 1.0, box_width, 1.0], 0x5028_007F);
    for (index, line) in lines.iter().enumerate() {
        draw_line(draw, line, [left + pad, top + pad + index as f32 * line_height], 1.0);
    }
    for (index, stack) in stacks.iter().enumerate() {
        draw.item_stack(
            stack,
            [left + pad + index as f32 * stack_step, top + pad + text_height],
            16.0,
        );
    }
}

// ---------------------------------------------------------------------------
// Item HUD: the mercurial wand
// ---------------------------------------------------------------------------

/// The face of the block at `block` that the point `at` on its surface lies on.
pub fn hit_face(block: [i32; 3], at: [f64; 3]) -> [i32; 3] {
    let offset: [f64; 3] = std::array::from_fn(|a| at[a] - f64::from(block[a]) - 0.5);
    let axis = (0..3)
        .max_by(|a, b| offset[*a].abs().total_cmp(&offset[*b].abs()))
        .unwrap_or(1);
    let mut face = [0; 3];
    face[axis] = if offset[axis] < 0.0 { -1 } else { 1 };
    face
}

/// What the wand would change on the block under the crosshair, read from the client's copy of
/// its block entity: `(current, new)` as the mode's values.
fn wand_target(host_ref: &dyn ClientHost) -> Option<(Hit, String, Compound)> {
    let hit = host_ref.raycast(6.0)?;
    let block = hit.block.id(host_ref);
    let level = host_ref.local_player()?.level(host_ref)?;
    let entity = host_ref.block_entity(level, [hit.block_x, hit.block_y, hit.block_z])?;
    Some((hit, block, entity.data(host_ref).unwrap_or_default()))
}

/// `MercurialWandItemMode.appendHUDText` for the wand `stack`.
fn wand_line(host_ref: &dyn ClientHost, stack: &ItemStack) -> Line {
    let key = |suffix: &str| format!("item.mode.theurgy.mercurial_wand.{suffix}");
    let frequency = items::component_i32(host_ref, stack, items::SELECTED_FREQUENCY).unwrap_or(0);
    let target = wand_target(host_ref);
    let connector = target
        .as_ref()
        .filter(|(_, block, _)| items::WAND_CONNECTORS.contains(&block.as_str()));
    // The current value is yellow when the click would change it, green when it would not.
    let current = |changes: bool| if changes { YELLOW } else { GREEN };
    match items::wand_mode(host_ref, stack) {
        0 => match connector {
            Some((hit, _, nbt)) => {
                let pos = [hit.block_x, hit.block_y, hit.block_z];
                let facing = host_ref
                    .local_player()
                    .and_then(|player| player.level(host_ref))
                    .and_then(|level| level.block_property(host_ref, pos, "facing"))
                    .as_deref()
                    .and_then(host::facing_name_to_dir)
                    .map_or(2, i32::from);
                let now = host::int(nbt, "directionOverride", facing);
                let next = items::next_direction(now);
                let name = |dir: i32| {
                    word(host_ref, &format!("theurgy.side.{}", host::dir_to_facing_name(dir.clamp(0, 5) as u8)))
                };
                translated(
                    host_ref,
                    &key("rotate_selected_direction.with_target"),
                    &[(name(now), current(now != next)), (name(next), GREEN)],
                    ITEM_HUD,
                )
            }
            None => translated(host_ref, &key("rotate_selected_direction"), &[], ITEM_HUD),
        },
        1 => {
            let mut line = translated(host_ref, &key("switch_logistics_enabled"), &[], ITEM_HUD);
            if let Some((_, _, nbt)) = connector {
                let enabled = nbt.get(&"enabled".into()).is_none() || host::bool(nbt, "enabled");
                let state = if enabled { (word(host_ref, &key("enabled")), GREEN) } else { (word(host_ref, &key("disabled")), RED) };
                line.extend(translated(host_ref, &key("switch_logistics_enabled.hud"), &[state], ITEM_HUD));
            }
            line
        }
        2 => translated(host_ref, &key("select_frequency"), &[(frequency.to_string(), GREEN)], ITEM_HUD),
        3 => match connector {
            Some((_, _, nbt)) => {
                let now = host::int(nbt, "frequency", 0);
                translated(
                    host_ref,
                    &key("set_selected_frequency.with_target"),
                    &[(now.to_string(), current(now != frequency)), (frequency.to_string(), GREEN)],
                    ITEM_HUD,
                )
            }
            None => translated(host_ref, &key("set_selected_frequency"), &[(frequency.to_string(), GREEN)], ITEM_HUD),
        },
        _ => match target.as_ref().filter(|(_, block, _)| block == "theurgy:mercury_capacitor") {
            Some((hit, _, nbt)) => {
                let face = hit_face([hit.block_x, hit.block_y, hit.block_z], [hit.x, hit.y, hit.z]);
                let side = items::face_direction_upper(face);
                let now = match nbt.get(&"sideModes".into()) {
                    Some(Tag::Compound(modes)) => host::int(modes, side, items::default_side_mode(side)),
                    _ => items::default_side_mode(side),
                };
                let next = (now + 1).rem_euclid(4);
                let mode = |ordinal: i32| {
                    let name = ["none", "input", "output", "both"][ordinal.rem_euclid(4) as usize];
                    word(host_ref, &format!("theurgy.side_mode.{name}"))
                };
                translated(
                    host_ref,
                    &key("cycle_capacitor_side.with_target"),
                    &[
                        (word(host_ref, &format!("theurgy.side.{}", side.to_ascii_lowercase())), AQUA),
                        (mode(now), current(now != next)),
                        (mode(next), GREEN),
                    ],
                    ITEM_HUD,
                )
            }
            None => translated(host_ref, &key("cycle_capacitor_side"), &[], ITEM_HUD),
        },
    }
}

/// `ItemHUD.render`: the held item's lines at the bottom left, scaled by the config.
fn draw_item_hud(host_ref: &dyn ClientHost, draw: &mut dyn DrawContext, scale: f32) {
    let Some(stack) = client::local_hand_stack(host_ref, Hand::Main) else {
        return;
    };
    if stack.item().id(host_ref) != items::MERCURIAL_WAND {
        return;
    }
    let lines = [wand_line(host_ref, &stack)];
    let height = draw.size()[1];
    for (index, line) in lines.iter().enumerate() {
        let from_bottom = 2.0 + (lines.len() - index) as f32 * 9.0;
        draw_line(draw, line, [2.0 * scale, height - from_bottom * scale], scale);
    }
}

pub fn gui_layer_draw(host_ref: &dyn ClientHost, event: &mut GuiLayerDrawEvent<'_>) {
    if event.layer != LAYER_ID || host_ref.hud_mode() != HudMode::AllVisible {
        return;
    }
    // A HUD belongs to the world's view: not over an inventory, the book or any other screen.
    if !host_ref.playing() {
        return;
    }
    let settings = config::get();
    if settings.enable_item_hud {
        draw_item_hud(host_ref, &mut *event.draw, settings.hud_scale.clamp(0.25, 3.0));
    }
    if settings.enable_in_world_hud {
        draw_in_world(host_ref, &mut *event.draw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &Line) -> String {
        line_text(line)
    }

    #[test]
    fn templates_fill_positional_and_sequential_holes() {
        let values = [("1100".to_owned(), GRAY), ("2000".to_owned(), GRAY)];
        assert_eq!(plain(&fill("%1$s / %2$smB", &values, GRAY)), "1100 / 2000mB");
        assert_eq!(plain(&fill("%2$s of %1$s", &values, GRAY)), "2000 of 1100");
        assert_eq!(plain(&fill("Set %s to %s", &values, GRAY)), "Set 1100 to 2000");
        let bar = [("█░".to_owned(), GREEN), ("15".to_owned(), GRAY)];
        let line = fill("Crafting: %1$s %2$s%%", &bar, GRAY);
        assert_eq!(plain(&line), "Crafting: █░ 15%");
        // The value keeps its own colour, the template's text the base one.
        assert_eq!(line[0], ("Crafting: ".to_owned(), GRAY));
        assert_eq!(line[1], ("█░".to_owned(), GREEN));
        assert_eq!(plain(&fill("no holes", &[], GRAY)), "no holes");
        assert_eq!(plain(&fill("50% done %s", &[], GRAY)), "50% done ");
    }

    #[test]
    fn progress_bar_has_ten_segments() {
        assert_eq!(progress_bar(0), "░░░░░░░░░░");
        assert_eq!(progress_bar(35), "███░░░░░░░");
        assert_eq!(progress_bar(100), "██████████");
        assert_eq!(progress_bar(250).chars().count(), 10);
        assert_eq!(progress_percent(50, 200), 25);
        assert_eq!(progress_percent(5, 0), 0);
        assert_eq!(progress_percent(300, 200), 100);
    }

    #[test]
    fn snapshot_shows_progress_only_while_processing() {
        let mut nbt = Compound::new();
        nbt.insert("progress".into(), Tag::Short(30));
        nbt.insert("totalTime".into(), Tag::Short(120));
        let idle = snapshot_of("theurgy:calcination_oven", [1, 2, 3], &nbt, &[0, 0, 0, 0, 0, 0]);
        assert_eq!(idle.progress, -1);
        let busy = snapshot_of("theurgy:calcination_oven", [1, 2, 3], &nbt, &[0, 0, 0, 0, 1, 1]);
        assert_eq!(busy.progress, 25);
        assert_eq!([busy.x, busy.y, busy.z], [1, 2, 3]);
        assert!(busy.flux.is_empty() && busy.fluids.is_empty());
    }

    #[test]
    fn snapshot_reports_tanks_with_capacity_even_when_empty() {
        let empty = snapshot_of("theurgy:sal_ammoniac_tank", [0; 3], &Compound::new(), &[]);
        assert_eq!(empty.fluids, vec![String::new()]);
        assert_eq!(empty.amounts, vec![0]);
        assert_eq!(empty.capacities.len(), 1);
        assert!(empty.capacities[0] > 0);

        let mut nbt = Compound::new();
        let tank = crate::engine::recipes::Tank {
            fluid: Some("theurgy:sal_ammoniac".into()),
            amount: 1100,
            capacity: empty.capacities[0],
        };
        host::save_tanks_nbt(&mut nbt, std::slice::from_ref(&tank));
        let filled = snapshot_of("theurgy:sal_ammoniac_tank", [0; 3], &nbt, &[]);
        assert_eq!(filled.fluids, vec!["theurgy:sal_ammoniac".to_owned()]);
        assert_eq!(filled.amounts, vec![1100]);
    }

    #[test]
    fn snapshot_reports_catalyst_flux_and_conversion() {
        let mut nbt = Compound::new();
        let mut handler = Compound::new();
        handler.insert("energy".into(), Tag::Int(7360));
        nbt.insert("mercuryFluxHandler".into(), Tag::Compound(handler));
        nbt.insert("mercuryFluxToConvert".into(), Tag::Int(250));
        nbt.insert("totalMercuryFluxToConvert".into(), Tag::Int(1000));
        let snapshot = snapshot_of("theurgy:mercury_catalyst", [0; 3], &nbt, &[]);
        assert_eq!(snapshot.flux, vec![7360, flux::CATALYST_CAPACITY]);
        assert_eq!(snapshot.progress, 75);
    }

    #[test]
    fn hit_face_is_the_side_the_point_lies_on() {
        assert_eq!(hit_face([10, 64, -3], [10.4, 65.0, -2.6]), [0, 1, 0]);
        assert_eq!(hit_face([10, 64, -3], [10.0, 64.3, -2.6]), [-1, 0, 0]);
        assert_eq!(hit_face([10, 64, -3], [10.7, 64.3, -2.0]), [0, 0, 1]);
    }
}
