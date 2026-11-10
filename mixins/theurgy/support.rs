// Game-side glue for Theurgy's calcination slice.
//
// Injected into `crate::game` as `theurgy_state`; it needs the game, so it is
// not part of the standalone library (only `runtime.rs` is).
//
// Two blocks: `theurgy:pyromantic_brazier` burns furnace fuel and publishes its
// `lit` state, `theurgy:calcination_oven` turns one item into its alchemical
// salt while the block below it is lit. Both keep upstream's saved keys — the
// oven `progress` and `totalTime`, the brazier `remainingLitTime` — and the
// oven's heat cache as `heatCache`, refreshed once every 20 game ticks, as
// `HeatConsumerBehaviour` samples it.
//
// Read from Theurgy 1.78.0 (`CalcinationOvenBlockEntity`, `CraftingBehaviour`,
// `HeatConsumerBehaviour`, `TwoSlotItemHandlerBehaviour`,
// `PyromanticBrazierBlockEntity`, `PyromanticBrazierBlock`, `BlockRegistry`).
// See `../../FINDINGS.md`.

use std::array;
use std::sync::atomic::{AtomicI64, Ordering};

use horizon_client::containers::{BlockEntity, ModBlockEntity, ModContainer};
use horizon_client::items::ItemStack;
use horizon_client::text::Text;
use horizon_world::nbt::{Compound, JavaString, Tag};

use crate::game::Game;
use crate::game::native_server::{NativeCommand, NativeServerPlayer};
use crate::game::theurgy::{self, Brazier, Oven, Step, heat_sample_due};

/// The two blocks (upstream `BlockRegistry`).
pub const OVEN: &str = "theurgy:calcination_oven";
pub const BRAZIER: &str = "theurgy:pyromantic_brazier";
/// `TwoSlotItemHandlerBehaviour`: input 0, output 1.
const INPUT: usize = 0;
const OUTPUT: usize = 1;
const SLOTS: usize = 2;
/// One `ItemStackHandler(1)`'s slot limit.
const SLOT_LIMIT: u32 = 64;
/// The oven's and the brazier's saved keys.
const KEY_PROGRESS: &str = "progress";
const KEY_TOTAL: &str = "totalTime";
const KEY_REMAINING: &str = "remainingLitTime";
const KEY_HEAT: &str = "heatCache";
/// The input slot's item the oven last started over on (`onContentTypeChanged`).
const KEY_INPUT_ITEM: &str = "inputItem";

/// The tick an insertion last happened in, so the second use a programmatic
/// click can deliver in that same tick does not take the item straight back out
/// (`ctl interact`'s press and release can both start a use; vanilla's 4-tick
/// use delay makes that impossible for a player).
static LAST_INSERT_TICK: AtomicI64 = AtomicI64::new(i64::MIN);

// ---------------------------------------------------------------------------
// Declaration hooks
// ---------------------------------------------------------------------------

/// Both blocks get a block entity (`native_block_entity`, API 15).
#[allow(
    private_interfaces,
    reason = "the injected bundle is this crate's own module"
)]
pub fn on_block_entity(block: &str, entity: &mut Option<(String, Compound)>) {
    if block == OVEN || block == BRAZIER {
        *entity = Some((block.to_owned(), Compound::new()));
    }
}

/// ... and both tick on the server (`native_block_entity_ticks`, API 15).
#[allow(
    private_interfaces,
    reason = "the injected bundle is this crate's own module"
)]
pub fn on_block_entity_ticks(block: &str, ticks: &mut bool) {
    if block == OVEN || block == BRAZIER {
        *ticks = true;
    }
}

/// The slots the world reaches (`native_block_entity_container`, API 16).
///
/// The oven is upstream's `CombinedInvWrapper(inputInventory,
/// outputInventoryExtractOnlyWrapper)`: a fill reaches the input, a drain takes
/// the input or the output. The brazier's plain `ItemStackHandler` is reachable
/// both ways from every side.
#[allow(
    private_interfaces,
    reason = "the injected bundle is this crate's own module"
)]
pub fn on_container(block: &str, container: &mut Option<ModContainer>) {
    if block == OVEN {
        *container = Some(ModContainer {
            slots: SLOTS,
            faces: array::from_fn(|_| vec![INPUT, OUTPUT]),
            inputs: vec![INPUT],
            outputs: vec![INPUT, OUTPUT],
            menu: None,
        });
    } else if block == BRAZIER {
        *container = Some(ModContainer {
            slots: 1,
            faces: array::from_fn(|_| vec![0]),
            inputs: vec![0],
            outputs: vec![0],
            menu: None,
        });
    }
}

// ---------------------------------------------------------------------------
// The server tick
// ---------------------------------------------------------------------------

/// One server tick of either block (`native_block_entity_tick`, API 17).
#[allow(
    private_interfaces,
    reason = "the injected bundle is this crate's own module"
)]
pub fn on_tick(game: &mut Game<'_>, pos: [i32; 3], entity: &mut ModBlockEntity) {
    if entity.id == OVEN {
        oven_tick(game, pos, entity);
    } else if entity.id == BRAZIER {
        brazier_tick(game, pos, entity);
    }
}

/// `CalcinationOvenBlockEntity.tickServer`: sample the heat below every 20
/// ticks, then advance the recipe by one.
fn oven_tick(game: &mut Game<'_>, pos: [i32; 3], entity: &mut ModBlockEntity) {
    if entity.items.len() < SLOTS {
        entity.items.resize(SLOTS, None);
    }
    let now = game.world.game_time.max(0) as u64;
    if heat_sample_due(now) {
        let heated = heat_below(game, pos);
        write_int(&mut entity.nbt, KEY_HEAT, i32::from(heated));
        set_lit(game, pos, heated);
    }
    let heated = read_int(&entity.nbt, KEY_HEAT) != 0;
    let mut oven = Oven {
        progress: read_int(&entity.nbt, KEY_PROGRESS).clamp(0, i32::from(u16::MAX)) as u16,
        total: read_int(&entity.nbt, KEY_TOTAL).clamp(0, i32::from(u16::MAX)) as u16,
    };
    if oven.total == 0 {
        oven = Oven::new();
    }
    // `InputInventory.onContentTypeChanged`: a new item sets the recipe's time and
    // starts the progress over.
    let input_item = entity.items[INPUT].as_ref().map(|stack| stack.item.to_string());
    if read_str(&entity.nbt, KEY_INPUT_ITEM) != input_item.as_deref().unwrap_or("") {
        write_str(
            &mut entity.nbt,
            KEY_INPUT_ITEM,
            input_item.as_deref().unwrap_or(""),
        );
        match input_item.as_deref() {
            Some(item) => {
                let in_tag = |item: &str, tag: &str| in_item_tag(game, item, tag);
                oven.input_changed(item, &in_tag);
            }
            None => oven.progress = 0,
        }
    }
    let step = {
        let input = entity.items[INPUT].as_ref().map(|stack| (&*stack.item, stack.count));
        let output = entity.items[OUTPUT].as_ref().map(|stack| (&*stack.item, stack.count));
        let in_tag = |item: &str, tag: &str| in_item_tag(game, item, tag);
        oven.tick(heated, input, output, &in_tag)
    };
    if let Step::Craft(recipe) = step {
        if let Some(stack) = entity.items[INPUT].as_mut() {
            stack.count = stack.count.saturating_sub(recipe.count);
            if stack.count == 0 {
                entity.items[INPUT] = None;
            }
        }
        match entity.items[OUTPUT].as_mut() {
            Some(stack) => stack.count = stack.count.saturating_add(recipe.output_count),
            None => entity.items[OUTPUT] = Some(ItemStack::new(recipe.output, recipe.output_count)),
        }
        game.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
    }
    write_int(&mut entity.nbt, KEY_PROGRESS, i32::from(oven.progress));
    write_int(&mut entity.nbt, KEY_TOTAL, i32::from(oven.total));
}

/// `PyromanticBrazierBlockEntity.tickServer`: burn down, then load the next
/// fuel item, and publish `lit`.
fn brazier_tick(game: &mut Game<'_>, pos: [i32; 3], entity: &mut ModBlockEntity) {
    if entity.items.is_empty() {
        entity.items.push(None);
    }
    let mut brazier = Brazier::new();
    brazier.remaining = read_int(&entity.nbt, KEY_REMAINING).clamp(0, i32::from(u16::MAX)) as u16;
    let fuel = entity.items[0].as_ref().map(|stack| burn_time(game, stack));
    let burn = brazier.tick(fuel);
    if burn.consumed {
        take_one_fuel(game, entity);
        game.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
    }
    write_int(&mut entity.nbt, KEY_REMAINING, i32::from(brazier.remaining));
    if burn.changed {
        set_lit(game, pos, burn.lit);
    }
}

// ---------------------------------------------------------------------------
// Heat
// ---------------------------------------------------------------------------

/// `HeatConsumerBehaviour.hasHeatProvider`: a lit heat provider directly below,
/// which at this slice is the pyromantic brazier with `lit=true`.
fn heat_below(game: &Game<'_>, pos: [i32; 3]) -> bool {
    let below = [pos[0], pos[1] - 1, pos[2]];
    game.native_block(below) == BRAZIER
        && game.native_block_property(below, "lit").as_deref() == Some("true")
}

/// The block's own `lit` state, written only when it changed (upstream sets it
/// from the heat answer and from the brazier's burn).
fn set_lit(game: &mut Game<'_>, pos: [i32; 3], lit: bool) {
    let wanted = if lit { "true" } else { "false" };
    if game.native_block_property(pos, "lit").as_deref() != Some(wanted) {
        let _ = game.native_set_block_property(pos, "lit", wanted);
    }
}

// ---------------------------------------------------------------------------
// Items and fuel
// ---------------------------------------------------------------------------

/// `Item.getBurnTime`: the engine's cooking-fuel value, 0 for a non-fuel.
fn burn_time(game: &Game<'_>, stack: &ItemStack) -> u32 {
    let providers = game.res.providers();
    let items = game.res.items();
    providers.burn_time(items, stack, false).max(0) as u32
}

/// Whether `item` is a member of `tag`, through the engine's merged item tags
/// (`native_server_item_in_tag`, API 63). An unreadable tag answers false, so
/// the recipe stays inert rather than matching everything.
fn in_item_tag(game: &Game<'_>, item: &str, tag: &str) -> bool {
    let stack = ItemStack::new(item, 1);
    game.native_server_item_in_tag(&stack, tag).unwrap_or(false)
}

/// Consume one fuel item, honouring a crafting remainder (a lava bucket leaves
/// its bucket), as `PyromanticBrazierBlockEntity.tickServer` does.
fn take_one_fuel(game: &Game<'_>, entity: &mut ModBlockEntity) {
    let Some(slot) = entity.items.first_mut() else {
        return;
    };
    let Some(stack) = slot else {
        return;
    };
    if stack.count > 1 {
        stack.count -= 1;
        return;
    }
    let remainder = game
        .res
        .items()
        .get(&stack.item)
        .and_then(|info| info.remainder.clone());
    *slot = remainder.map(|item| ItemStack::new(item.as_str(), 1));
}

// ---------------------------------------------------------------------------
// The player's click
// ---------------------------------------------------------------------------

/// `TwoSlotItemHandlerBehaviour.useItemOn` and the brazier's own click
/// (`native_use_item_on`, API 13). An empty hand takes the output first and the
/// input second; a held item goes into the input when the block accepts it.
#[allow(
    private_interfaces,
    reason = "the injected bundle is this crate's own module"
)]
pub fn on_use_item_on(game: &mut Game<'_>, stack: &ItemStack, pos: [i32; 3], claimed: &mut bool) {
    match game.native_block(pos).as_str() {
        OVEN => oven_click(game, stack, pos, claimed),
        BRAZIER => brazier_click(game, stack, pos, claimed),
        _ => {}
    }
}

/// An empty hand on either apparatus (`native_right_click_block`, API 21).
///
/// `native_use_item_on` (API 13) only runs for a hand that holds something — the
/// game returns before the hook for an empty one — so the take path upstream puts
/// in `TwoSlotItemHandlerBehaviour.useItemOn` lives here: the oven gives its
/// output first and its input second, the brazier its fuel slot.
#[allow(
    private_interfaces,
    reason = "the injected bundle is this crate's own module"
)]
pub fn on_right_click_block(game: &mut Game<'_>, pos: [i32; 3], canceled: &mut bool) {
    let block = game.native_block(pos);
    if block != OVEN && block != BRAZIER {
        return;
    }
    if game.held().is_some() {
        // The item path is `on_use_item_on`'s, which runs after this.
        return;
    }
    if LAST_INSERT_TICK.load(Ordering::Relaxed) == game.world.game_time {
        return;
    }
    let slots: &[usize] = if block == OVEN { &[OUTPUT, INPUT] } else { &[0] };
    for &slot in slots {
        let taken = game
            .native_with_mod_block_entity(pos, |entity| {
                entity.items.get_mut(slot).and_then(Option::take)
            })
            .flatten();
        let Some(taken) = taken else { continue };
        give_back(game, taken);
        *canceled = true;
        return;
    }
}

/// The oven's click with an item: into the input, onto the stack already there
/// (a slot holds one item kind, as every `ItemStackHandler` does).
///
/// Upstream's `InputInventory.isItemValid` also refuses an item no recipe takes.
/// The port lets it in and lets the tick decide instead: the click hook runs on
/// the client too, where the engine's item tags answer `NotServer`, so a
/// tag-gated insert would refuse every gem, ore and log on that side.
fn oven_click(game: &mut Game<'_>, stack: &ItemStack, pos: [i32; 3], claimed: &mut bool) {
    if stack.count == 0 {
        return;
    }
    let held = &*stack.item;
    let input_item = game
        .native_with_mod_block_entity(pos, |entity| {
            entity.items[INPUT].as_ref().map(|stack| stack.item.to_string())
        })
        .flatten();
    if input_item.as_deref().is_some_and(|stored| stored != held) {
        return;
    }
    let room = game
        .native_with_mod_block_entity(pos, |entity| {
            let filling = entity.items[INPUT].as_ref().map_or(0, |stack| stack.count);
            SLOT_LIMIT.saturating_sub(filling)
        })
        .unwrap_or(0);
    let moved = game.native_take_items(held, stack.count.min(room));
    if moved == 0 {
        return;
    }
    let _ = game.native_with_mod_block_entity(pos, |entity| {
        match entity.items[INPUT].as_mut() {
            Some(filling) => filling.count = filling.count.saturating_add(moved),
            None => entity.items[INPUT] = Some(ItemStack::new(held, moved)),
        }
    });
    LAST_INSERT_TICK.store(game.world.game_time, Ordering::Relaxed);
    *claimed = true;
}

/// The brazier's click (`PyromanticBrazierBlock.useItemOn`): an empty hand, or a
/// click on a slot that holds a non-fuel, gives that item back; otherwise a fuel
/// item goes in.
fn brazier_click(game: &mut Game<'_>, stack: &ItemStack, pos: [i32; 3], claimed: &mut bool) {
    let held = (stack.count > 0).then(|| stack.clone());
    let stored = game
        .native_with_mod_block_entity(pos, |entity| entity.items.first().and_then(Option::clone))
        .flatten();
    let stored_is_fuel = stored
        .as_ref()
        .is_some_and(|stack| burn_time(game, stack) > 0);
    if held.is_none() || (stored.is_some() && !stored_is_fuel) {
        let Some(stored) = stored else { return };
        let _ = game.native_with_mod_block_entity(pos, |entity| {
            if let Some(slot) = entity.items.first_mut() {
                *slot = None;
            }
        });
        give_back(game, stored);
        *claimed = true;
        return;
    }
    let Some(held) = held else { return };
    if burn_time(game, &held) == 0 {
        return;
    }
    let room = game
        .native_with_mod_block_entity(pos, |entity| {
            let filling = entity.items.first().and_then(Option::as_ref).map_or(0, |s| s.count);
            SLOT_LIMIT.saturating_sub(filling)
        })
        .unwrap_or(0);
    let moved = game.native_take_items(&held.item, held.count.min(room));
    if moved == 0 {
        return;
    }
    let item = held.item.clone();
    let _ = game.native_with_mod_block_entity(pos, |entity| {
        if entity.items.is_empty() {
            entity.items.push(None);
        }
        match &mut entity.items[0] {
            Some(filling) => filling.count = filling.count.saturating_add(moved),
            slot => *slot = Some(ItemStack::new(&*item, moved)),
        }
    });
    LAST_INSERT_TICK.store(game.world.game_time, Ordering::Relaxed);
    *claimed = true;
}

/// Give a stack to the acting player (`Inventory.placeItemBackInInventory`).
fn give_back(game: &mut Game<'_>, stack: ItemStack) {
    if let Some(player) = game.native_acting_player() {
        let _ = game.native_give_stack(player.uuid, stack);
    }
}

// ---------------------------------------------------------------------------
// Readouts and the harness controls
// ---------------------------------------------------------------------------

/// `/theurgy info|recipe|recipes` (`native_register_commands`, API 8).
#[allow(
    private_interfaces,
    reason = "the injected bundle is this crate's own module"
)]
pub fn on_register_commands(commands: &mut Vec<NativeCommand>) {
    commands.push(NativeCommand::new("theurgy info <pos:block_pos>"));
    commands.push(NativeCommand::new("theurgy recipe <item:item>"));
    commands.push(NativeCommand::new("theurgy recipes"));
}

/// Their bodies (`native_perform_command`, API 8).
#[allow(
    private_interfaces,
    reason = "the injected bundle is this crate's own module"
)]
pub fn on_perform_command(
    game: &mut Game<'_>,
    _player: &NativeServerPlayer,
    command: &str,
    ran: &mut bool,
) {
    let words: Vec<&str> = command.split_whitespace().collect();
    if words.first() != Some(&"theurgy") {
        return;
    }
    *ran = true;
    match words.get(1).copied() {
        Some("info") => {
            let Some(pos) = parse_pos(&words[2..]) else {
                game.native_feedback(Text::literal("usage: /theurgy info <x> <y> <z>"));
                return;
            };
            game.native_feedback(Text::literal(readout(game, pos)));
        }
        Some("recipe") => {
            let Some(item) = words.get(2).copied() else {
                game.native_feedback(Text::literal("usage: /theurgy recipe <item>"));
                return;
            };
            let recipe = theurgy::recipe_for(item, &|item: &str, tag: &str| in_item_tag(game, item, tag));
            game.native_feedback(Text::literal(match recipe {
                Some(recipe) => format!(
                    "{item} calcines to {} x{} for {} of it in {} ticks.",
                    recipe.output, recipe.output_count, recipe.count, recipe.time
                ),
                None => format!("{item} has no calcination recipe."),
            }));
        }
        Some("recipes") => {
            let total = theurgy::RECIPES.len();
            let tags = theurgy::RECIPES
                .iter()
                .filter(|recipe| matches!(recipe.input, theurgy::Ingredient::Tag(_)))
                .count();
            game.native_feedback(Text::literal(format!(
                "{total} calcination recipes loaded, {tags} of them by tag."
            )));
        }
        _ => game.native_feedback(Text::literal("usage: /theurgy info|recipe|recipes ...")),
    }
}

/// A block's state, as `/theurgy info` prints it.
fn readout(game: &Game<'_>, pos: [i32; 3]) -> String {
    let key = (pos[0], pos[1], pos[2]);
    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&key) else {
        return format!("{pos:?} is not a mod block entity.");
    };
    let lit = game.native_block_property(pos, "lit").unwrap_or_else(|| "?".to_owned());
    match &*entity.id {
        OVEN => {
            let input = slot_text(entity.items.get(INPUT).and_then(Option::as_ref));
            let output = slot_text(entity.items.get(OUTPUT).and_then(Option::as_ref));
            format!(
                "Calcination Oven at {pos:?}: lit={lit}, input {input}, output {output}, progress {}/{}, heatCache {}, t={}",
                read_int(&entity.nbt, KEY_PROGRESS),
                read_int(&entity.nbt, KEY_TOTAL),
                read_int(&entity.nbt, KEY_HEAT),
                game.world.game_time
            )
        }
        BRAZIER => format!(
            "Pyromantic Brazier at {pos:?}: lit={lit}, fuel {}, remaining {} ticks, t={}",
            slot_text(entity.items.first().and_then(Option::as_ref)),
            read_int(&entity.nbt, KEY_REMAINING),
            game.world.game_time
        ),
        other => format!("{pos:?} is a {other}, not a Theurgy apparatus."),
    }
}

fn slot_text(stack: Option<&ItemStack>) -> String {
    match stack {
        Some(stack) => format!("{} x{}", stack.item, stack.count),
        None => "empty".to_owned(),
    }
}

fn parse_pos(words: &[&str]) -> Option<[i32; 3]> {
    Some([
        words.first()?.parse().ok()?,
        words.get(1)?.parse().ok()?,
        words.get(2)?.parse().ok()?,
    ])
}

fn read_int(nbt: &Compound, key: &str) -> i32 {
    match nbt.get(&JavaString::from(key)) {
        Some(Tag::Int(value)) => *value,
        Some(Tag::Short(value)) => i32::from(*value),
        Some(Tag::Byte(value)) => i32::from(*value),
        _ => 0,
    }
}

fn write_int(nbt: &mut Compound, key: &str, value: i32) {
    nbt.insert(JavaString::from(key), Tag::Int(value));
}

fn read_str(nbt: &Compound, key: &str) -> String {
    match nbt.get(&JavaString::from(key)) {
        Some(Tag::String(value)) => value.to_string_lossy(),
        _ => String::new(),
    }
}

fn write_str(nbt: &mut Compound, key: &str, value: &str) {
    nbt.insert(JavaString::from(key), Tag::String(JavaString::from(value)));
}
