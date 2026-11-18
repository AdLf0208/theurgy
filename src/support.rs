//! Game-side glue for Theurgy's calcination slice.
//!
//! [`crate::runtime`] is the pure truth — the jar's 86 recipes, the oven's progress rule and
//! the brazier's burn rule, and its own tests. This module is the only place that reads the
//! game and writes it back: the two block entities' saved compounds and item slots, the heat
//! sample, the clicks, the readouts.
//!
//! The 0.2.0 package replaced the retired `.hnmod` bundle; the mapping from the old hook
//! helpers to this module and to the Parts is in `notes.md` under
//! `## Migration to the SDK format`.

use std::sync::atomic::{AtomicI64, Ordering};

use mod_api::{
    BlockClick, BlockEntityRef, BlockEntityTickEvent, Command, CommandArgument,
    CommandArgumentValue, Compound, Flow, Host, HostTransaction, Identifier, InteractionResult,
    ItemKey, ItemPlace, ItemResource, ItemStack, LevelRef, ModBlockEntityView, ModContainer,
    Registrar, RunModCommandEvent, ServerHost, Tag, Text, UseItemOnEvent, UseWithoutItemEvent,
};

use crate::runtime::{self, Brazier, Ingredient, Oven, Step, heat_sample_due};

/// The two blocks (upstream `BlockRegistry`).
pub const OVEN: &str = "theurgy:calcination_oven";
pub const BRAZIER: &str = "theurgy:pyromantic_brazier";
/// The block entity types, one per block (upstream's `BlockEntityRegistry`).
pub const OVEN_TYPE: &str = "calcination_oven";
pub const BRAZIER_TYPE: &str = "pyromantic_brazier";
/// `TwoSlotItemHandlerBehaviour`: input 0, output 1.
const INPUT: usize = 0;
const OUTPUT: usize = 1;
const SLOTS: usize = 2;
/// The oven gives its output first and its input second on an empty hand; the brazier has
/// one slot, so the same list is its fuel slot.
const OVEN_TAKE: [usize; 2] = [OUTPUT, INPUT];
const BRAZIER_TAKE: [usize; 1] = [0];
/// The oven's and the brazier's saved keys, as upstream's `writeNBT` spells them.
const KEY_PROGRESS: &str = "progress";
const KEY_TOTAL: &str = "totalTime";
const KEY_REMAINING: &str = "remainingLitTime";
const KEY_HEAT: &str = "heatCache";
/// The input slot's item the oven last started over on (`onContentTypeChanged`).
const KEY_INPUT_ITEM: &str = "inputItem";

/// The game tick an insertion last happened in, so the second use a programmatic click can
/// deliver in that same tick does not take the item straight back out (`ctl interact`'s press
/// and release can both start a use; vanilla's 4-tick use delay makes that impossible for a
/// player). Recorded as a deviation in `notes.md`.
static LAST_INSERT_TICK: AtomicI64 = AtomicI64::new(i64::MIN);

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

/// Declare the two block entity types, their Parts, and the item slots the world reaches.
///
/// The same Part answers both what the game asks the *block* (the click) and what it asks the
/// *block entity type* (each server tick), so one type does the whole block.
pub fn declare(r: &mut dyn Registrar) {
    let oven = r.block_key(OVEN);
    let brazier = r.block_key(BRAZIER);
    let oven_type = r
        .block_entity_type(OVEN_TYPE)
        .blocks(&["calcination_oven"])
        .build();
    let brazier_type = r
        .block_entity_type(BRAZIER_TYPE)
        .blocks(&["pyromantic_brazier"])
        .build();
    r.block_part(oven, crate::Oven);
    r.block_part(brazier, crate::Brazier);
    // Each type was declared on the lines above; a refusal is a programming error.
    r.block_entity_part(&oven_type, crate::Oven)
        .expect("the oven's Part attaches to the type this mod just declared");
    r.block_entity_part(&brazier_type, crate::Brazier)
        .expect("the brazier's Part attaches to the type this mod just declared");
    r.block_container(&oven, oven_container());
    r.block_container(&brazier, brazier_container());
}

/// The oven is upstream's `CombinedInvWrapper(inputInventory, outputInventoryExtractOnly)`:
/// a fill reaches the input, a drain takes the input or the output.
fn oven_container() -> ModContainer {
    ModContainer {
        slots: SLOTS,
        faces: std::array::from_fn(|_| vec![INPUT, OUTPUT]),
        inputs: vec![INPUT],
        outputs: vec![INPUT, OUTPUT],
        menu: None,
    }
}

/// The brazier's plain `ItemStackHandler(1)` is reachable both ways from every side.
fn brazier_container() -> ModContainer {
    ModContainer {
        slots: 1,
        faces: std::array::from_fn(|_| vec![0]),
        inputs: vec![0],
        outputs: vec![0],
        menu: None,
    }
}

/// `/theurgy info|recipe|recipes` — the readouts players and the harness use.
pub fn declare_commands(commands: &mut Vec<Command>) {
    commands.push(Command::new("theurgy info <pos:block_pos>"));
    commands.push(Command::new("theurgy recipe <item:item>"));
    commands.push(Command::new("theurgy recipes"));
}

// ---------------------------------------------------------------------------
// The server tick
// ---------------------------------------------------------------------------

/// `CalcinationOvenBlockEntity.tickServer`: sample the heat below every 20 ticks, then advance
/// the recipe by one.
pub fn oven_tick(host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
    let Some(level) = level_in_place(host) else {
        return;
    };
    let pos = event.pos;
    if event.entity.items.len() < SLOTS {
        event.entity.items.resize(SLOTS, None);
    }
    let now = level.game_time(&*host).unwrap_or(0).max(0).unsigned_abs();
    if heat_sample_due(now) {
        let heated = heat_below(&*host, level, pos);
        write_int(&mut event.entity.nbt, KEY_HEAT, i32::from(heated));
        set_lit(host, level, pos, heated);
    }
    let heated = read_int(&event.entity.nbt, KEY_HEAT) != 0;
    let mut oven = Oven {
        progress: read_int(&event.entity.nbt, KEY_PROGRESS).clamp(0, i32::from(u16::MAX)) as u16,
        total: read_int(&event.entity.nbt, KEY_TOTAL).clamp(0, i32::from(u16::MAX)) as u16,
    };
    if oven.total == 0 {
        oven = Oven::new();
    }
    // `InputInventory.onContentTypeChanged`: a new item sets the recipe's time and starts the
    // progress over.
    let input_id = event.entity.items[INPUT]
        .as_ref()
        .map(|stack| stack.item().id(&*host));
    let stored = read_str(&event.entity.nbt, KEY_INPUT_ITEM);
    let current = input_id.as_deref().unwrap_or("");
    if stored != current {
        write_str(&mut event.entity.nbt, KEY_INPUT_ITEM, current);
        match input_id.as_deref() {
            Some(item) => {
                let host_ref: &dyn Host = &*host;
                oven.input_changed(item, &|item: &str, tag: &str| {
                    in_item_tag(host_ref, item, tag)
                });
            }
            None => oven.progress = 0,
        }
    }
    let input_count = event.entity.items[INPUT]
        .as_ref()
        .map_or(0, ItemStack::count);
    let output_id = event.entity.items[OUTPUT]
        .as_ref()
        .map(|stack| stack.item().id(&*host));
    let output_count = event.entity.items[OUTPUT]
        .as_ref()
        .map_or(0, ItemStack::count);
    let step = {
        let host_ref: &dyn Host = &*host;
        let in_tag = |item: &str, tag: &str| in_item_tag(host_ref, item, tag);
        oven.tick(
            heated,
            input_id.as_deref().map(|id| (id, input_count)),
            output_id.as_deref().map(|id| (id, output_count)),
            &in_tag,
        )
    };
    if let Step::Craft(recipe) = step {
        let left = input_count.saturating_sub(recipe.count);
        if left == 0 {
            event.entity.items[INPUT] = None;
        } else if let Some(stack) = event.entity.items[INPUT].as_mut() {
            let _ = stack.set_count(left);
        }
        match event.entity.items[OUTPUT].as_mut() {
            Some(stack) => {
                let total = stack.count().saturating_add(recipe.output_count);
                let _ = stack.set_count(total);
            }
            None => {
                event.entity.items[OUTPUT] = item_key(&*host, recipe.output)
                    .and_then(|key| ItemStack::new(key, recipe.output_count));
            }
        }
    }
    write_int(
        &mut event.entity.nbt,
        KEY_PROGRESS,
        i32::from(oven.progress),
    );
    write_int(&mut event.entity.nbt, KEY_TOTAL, i32::from(oven.total));
}

/// `PyromanticBrazierBlockEntity.tickServer`: burn down, then load the next fuel item, and
/// publish `lit`.
pub fn brazier_tick(host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
    let Some(level) = level_in_place(host) else {
        return;
    };
    let pos = event.pos;
    if event.entity.items.is_empty() {
        event.entity.items.push(None);
    }
    let fuel = event.entity.items[0]
        .as_ref()
        .map(|stack| burn_time(&*host, stack));
    let mut brazier = Brazier {
        remaining: read_int(&event.entity.nbt, KEY_REMAINING).clamp(0, i32::from(u16::MAX)) as u16,
    };
    let burn = brazier.tick(fuel);
    if burn.consumed {
        take_one_fuel(host, &mut event.entity);
    }
    write_int(
        &mut event.entity.nbt,
        KEY_REMAINING,
        i32::from(brazier.remaining),
    );
    if burn.changed {
        set_lit(host, level, pos, burn.lit);
    }
}

// ---------------------------------------------------------------------------
// The player's click
// ---------------------------------------------------------------------------

/// `TwoSlotItemHandlerBehaviour.useItemOn` on the oven: a held item goes into the input, onto a
/// stack of the same item (a slot holds one item kind, as every `ItemStackHandler` does).
///
/// Upstream's `InputInventory.isItemValid` also refuses an item no recipe takes. The port lets
/// it in and lets the tick decide instead: a tag-gated insert would refuse every gem, ore and
/// log on a client, where the engine's item tags are not the server's.
pub fn oven_use_item_on(
    host: &mut dyn ServerHost,
    event: &mut UseItemOnEvent<'_>,
) -> Option<InteractionResult> {
    let level = host.level(event.click.dimension.key().id())?;
    let pos = event.click.pos;
    let stack = event.click.stack?;
    if stack.count() == 0 {
        return None;
    }
    let entity = host.block_entity(level, pos)?;
    if !is_kind(&*host, entity, OVEN) {
        return None;
    }
    let held = stack.item().id(&*host);
    let stored = entity.item_slot(&*host, INPUT as u32).ok().flatten();
    if stored
        .as_ref()
        .is_some_and(|stored| stored.item().id(&*host) != held)
    {
        return None;
    }
    let filling = stored.as_ref().map_or(0, ItemStack::count);
    let room = runtime::SLOT_LIMIT.saturating_sub(filling);
    let want = stack.count().min(room);
    if want == 0 {
        return None;
    }
    let hand = ItemPlace::PlayerHand {
        player: event.click.player,
        hand: event.click.hand,
    };
    let resource = ItemResource::of(stack);
    let placed = host.transaction(|host, tx| {
        let moved = hand.extract(host, tx, &resource, want);
        if moved == 0 {
            return Err(());
        }
        let entity = host.block_entity(level, pos).ok_or(())?;
        let total = filling.saturating_add(moved);
        let placed = ItemStack::new(stack.item(), total).ok_or(())?;
        entity
            .set_item_slot(host, tx, INPUT as u32, Some(placed))
            .map_err(|_| ())?;
        Ok(())
    });
    if placed.is_err() {
        return None;
    }
    remember_insert(host, level);
    Some(InteractionResult::Success)
}

/// The oven on an empty hand: the output first, then the input
/// (`TwoSlotItemHandlerBehaviour.useItemOn`).
///
/// The retired pin ran this from `right_click_block` because API 13's `use_item_on` never runs
/// for an empty hand; the SDK splits the two hands into `use_item_on` and `use_without_item`,
/// so the take lives where upstream puts it.
pub fn oven_use_without_item(
    host: &mut dyn ServerHost,
    event: &mut UseWithoutItemEvent<'_>,
) -> Option<InteractionResult> {
    take_from(host, event.click, OVEN, &OVEN_TAKE)
}

/// `PyromanticBrazierBlock.useItemOn`: an empty hand, or a click on a slot that holds a
/// non-fuel, gives that item back; otherwise a fuel item goes in.
pub fn brazier_use_item_on(
    host: &mut dyn ServerHost,
    event: &mut UseItemOnEvent<'_>,
) -> Option<InteractionResult> {
    let level = host.level(event.click.dimension.key().id())?;
    let pos = event.click.pos;
    let entity = host.block_entity(level, pos)?;
    if !is_kind(&*host, entity, BRAZIER) {
        return None;
    }
    let stored = entity.item_slot(&*host, 0).ok().flatten();
    let stored_is_fuel = stored
        .as_ref()
        .is_some_and(|stored| burn_time(&*host, stored) > 0);
    let held = event.click.stack.filter(|stack| stack.count() > 0);
    if held.is_none() || (stored.is_some() && !stored_is_fuel) {
        let stored = stored?;
        return give_back(host, level, pos, 0, stored, event.click.player);
    }
    let held = held?;
    if burn_time(&*host, held) == 0 {
        return None;
    }
    let filling = stored.as_ref().map_or(0, ItemStack::count);
    let room = runtime::SLOT_LIMIT.saturating_sub(filling);
    let want = held.count().min(room);
    if want == 0 {
        return None;
    }
    let hand = ItemPlace::PlayerHand {
        player: event.click.player,
        hand: event.click.hand,
    };
    let resource = ItemResource::of(held);
    let placed = host.transaction(|host, tx| {
        let moved = hand.extract(host, tx, &resource, want);
        if moved == 0 {
            return Err(());
        }
        let entity = host.block_entity(level, pos).ok_or(())?;
        let total = filling.saturating_add(moved);
        let placed = ItemStack::new(held.item(), total).ok_or(())?;
        entity
            .set_item_slot(host, tx, 0, Some(placed))
            .map_err(|_| ())?;
        Ok(())
    });
    if placed.is_err() {
        return None;
    }
    remember_insert(host, level);
    Some(InteractionResult::Success)
}

/// The brazier on an empty hand gives its fuel slot back.
pub fn brazier_use_without_item(
    host: &mut dyn ServerHost,
    event: &mut UseWithoutItemEvent<'_>,
) -> Option<InteractionResult> {
    take_from(host, event.click, BRAZIER, &BRAZIER_TAKE)
}

// ---------------------------------------------------------------------------
// The readouts
// ---------------------------------------------------------------------------

/// The bodies of `/theurgy info|recipe|recipes` (`Hooks::run_mod_command`).
pub fn run_command(host: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
    if event.cancelled || !is_theurgy(&event.command.literals) {
        return Flow::Continue;
    }
    match event.command.literals.get(1).map(String::as_str) {
        Some("info") => {
            let Some(pos) = info_position(&event.command.arguments) else {
                feedback(host, "usage: /theurgy info <x> <y> <z>");
                return Flow::Cancel;
            };
            let level = event
                .command
                .dimension
                .id(&*host)
                .and_then(|id| host.level(&id));
            let line = match level {
                Some(level) => readout(host, level, pos),
                None => format!("{pos:?} is not a mod block entity."),
            };
            feedback(host, line);
            Flow::Cancel
        }
        Some("recipe") => {
            let Some(CommandArgumentValue::Item(stack)) = event.command.argument("item").cloned()
            else {
                feedback(host, "usage: /theurgy recipe <item>");
                return Flow::Cancel;
            };
            let item = stack.item().id(&*host);
            let recipe = {
                let host_ref: &dyn Host = &*host;
                let in_tag = |item: &str, tag: &str| in_item_tag(host_ref, item, tag);
                runtime::recipe_for(&item, &in_tag)
            };
            feedback(
                host,
                match recipe {
                    Some(recipe) => format!(
                        "{item} calcines to {} x{} for {} of it in {} ticks.",
                        recipe.output, recipe.output_count, recipe.count, recipe.time
                    ),
                    None => format!("{item} has no calcination recipe."),
                },
            );
            Flow::Cancel
        }
        Some("recipes") => {
            let total = runtime::RECIPES.len();
            let tags = runtime::RECIPES
                .iter()
                .filter(|recipe| matches!(recipe.input, Ingredient::Tag(_)))
                .count();
            feedback(
                host,
                format!("{total} calcination recipes loaded, {tags} of them by tag."),
            );
            Flow::Cancel
        }
        _ => {
            feedback(host, "usage: /theurgy info|recipe|recipes ...");
            Flow::Cancel
        }
    }
}

/// A block's state, as `/theurgy info` prints it.
fn readout(host: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3]) -> String {
    let Some(entity) = host.block_entity(level, pos) else {
        return format!("{pos:?} is not a mod block entity.");
    };
    let lit = level
        .block_property(&*host, pos, "lit")
        .unwrap_or_else(|| "?".to_owned());
    let kind = entity.kind(&*host).map(|kind| kind.id(&*host));
    let data = entity.data(&*host).unwrap_or_else(Compound::new);
    match kind.as_deref() {
        Some(OVEN) => {
            let input = entity.item_slot(&*host, INPUT as u32).ok().flatten();
            let output = entity.item_slot(&*host, OUTPUT as u32).ok().flatten();
            format!(
                "Calcination Oven at {pos:?}: lit={lit}, input {}, output {}, progress {}/{}, heatCache {}, t={}",
                slot_text(&*host, input.as_ref()),
                slot_text(&*host, output.as_ref()),
                read_int(&data, KEY_PROGRESS),
                read_int(&data, KEY_TOTAL),
                read_int(&data, KEY_HEAT),
                level.game_time(&*host).unwrap_or(0)
            )
        }
        Some(BRAZIER) => {
            let fuel = entity.item_slot(&*host, 0).ok().flatten();
            format!(
                "Pyromantic Brazier at {pos:?}: lit={lit}, fuel {}, remaining {} ticks, t={}",
                slot_text(&*host, fuel.as_ref()),
                read_int(&data, KEY_REMAINING),
                level.game_time(&*host).unwrap_or(0)
            )
        }
        Some(other) => format!("{pos:?} is a {other}, not a Theurgy apparatus."),
        None => format!("{pos:?} is not a mod block entity."),
    }
}

/// `"<item> x<count>"`, or `empty` for a slot that holds nothing.
fn slot_text(host: &dyn Host, stack: Option<&ItemStack>) -> String {
    match stack {
        Some(stack) => format!("{} x{}", stack.item().id(host), stack.count()),
        None => "empty".to_owned(),
    }
}

fn feedback(host: &mut dyn ServerHost, line: impl Into<String>) {
    let _ = host.commands().feedback(Text::literal(line.into()));
}

// ---------------------------------------------------------------------------
// Heat, fuel and the hand
// ---------------------------------------------------------------------------

/// `HeatConsumerBehaviour.hasHeatProvider`: a lit heat provider directly below, which in this
/// slice is the pyromantic brazier with `lit=true`.
fn heat_below(host: &dyn Host, level: LevelRef, pos: [i32; 3]) -> bool {
    let below = [pos[0], pos[1] - 1, pos[2]];
    level
        .block(host, below)
        .map(|block| block.id(host))
        .as_deref()
        == Some(BRAZIER)
        && level.block_property(host, below, "lit").as_deref() == Some("true")
}

/// The block's own `lit` state, written only when it changed (`native_set_block_property`, the
/// write that keeps the block entity).
fn set_lit(host: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3], lit: bool) {
    let wanted = if lit { "true" } else { "false" };
    if level.block_property(&*host, pos, "lit").as_deref() != Some(wanted) {
        let _ = host.set_block_property(pos, "lit", wanted);
    }
}

/// `Item.getBurnTime`: the engine's cooking-fuel value, 0 for a non-fuel.
fn burn_time(host: &dyn Host, stack: &ItemStack) -> u32 {
    host.item_burn_time(stack.item()).unwrap_or(0).max(0) as u32
}

/// Consume one fuel item, honouring a crafting remainder (a lava bucket leaves its bucket), as
/// `PyromanticBrazierBlockEntity.tickServer` does.
fn take_one_fuel(host: &mut dyn ServerHost, entity: &mut ModBlockEntityView<'_>) {
    let Some(slot) = entity.items.first_mut() else {
        return;
    };
    let Some(stack) = slot else {
        return;
    };
    if stack.count() > 1 {
        let left = stack.count() - 1;
        let _ = stack.set_count(left);
        return;
    }
    *slot = host.item_crafting_remainder(stack);
}

/// An empty hand on the oven or the brazier: the first non-empty slot of `slots` goes to the
/// player, who is given nothing when they have no room (then the slot stays).
fn take_from(
    host: &mut dyn ServerHost,
    click: &BlockClick<'_>,
    id: &str,
    slots: &[usize],
) -> Option<InteractionResult> {
    // `use_without_item` also runs as the fall-through after `use_item_on` passed a held item
    // (`BlockState.useItemOn` returning TRY_WITH_EMPTY_HAND, the default), so the hand must be
    // empty for the take — the retired pin's `right_click_block` checked `game.held()`, and
    // without this a refused item would take the output instead of staying in the hand.
    if click.stack.is_some_and(|stack| stack.count() > 0) {
        return None;
    }
    let level = host.level(click.dimension.key().id())?;
    let pos = click.pos;
    let entity = host.block_entity(level, pos)?;
    if !is_kind(&*host, entity, id) {
        return None;
    }
    if LAST_INSERT_TICK.load(Ordering::Relaxed) == level.game_time(&*host).unwrap_or(0) {
        // The second use a programmatic click delivers in the insert's own tick.
        return None;
    }
    for &slot in slots {
        let Some(stack) = entity.item_slot(&*host, slot as u32).ok().flatten() else {
            continue;
        };
        let given = host.transaction(|host, tx| {
            let entity = host.block_entity(level, pos).ok_or(())?;
            // `Inventory.placeItemBackInInventory`: what does not fit is thrown to the player.
            let player = host.player(click.player).ok_or(())?;
            player.give_stack(host, stack.clone()).map_err(|_| ())?;
            entity
                .set_item_slot(host, tx, slot as u32, None)
                .map_err(|_| ())?;
            Ok::<(), ()>(())
        });
        if given.is_ok() {
            return Some(InteractionResult::Success);
        }
    }
    None
}

/// Give a stored stack back to the acting player and empty its slot, in one transaction.
fn give_back(
    host: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    slot: usize,
    stack: ItemStack,
    player: u128,
) -> Option<InteractionResult> {
    let given = host.transaction(|host, tx| {
        let entity = host.block_entity(level, pos).ok_or(())?;
        let actor = host.player(player).ok_or(())?;
        actor.give_stack(host, stack).map_err(|_| ())?;
        entity
            .set_item_slot(host, tx, slot as u32, None)
            .map_err(|_| ())?;
        Ok::<(), ()>(())
    });
    given.ok().map(|()| InteractionResult::Success)
}

fn remember_insert(host: &dyn Host, level: LevelRef) {
    LAST_INSERT_TICK.store(level.game_time(host).unwrap_or(0), Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Handles and saved-compound reads
// ---------------------------------------------------------------------------

/// The level whose block entities the server ticks now. A block-entity tick is not told its
/// level; the level in place is the one being ticked.
fn level_in_place(host: &dyn ServerHost) -> Option<LevelRef> {
    host.levels()
        .list_levels()
        .into_iter()
        .find(|level| level.in_place(host))
}

/// Whether the block entity is the type `id` (`theurgy:<path>`).
fn is_kind(host: &dyn Host, entity: BlockEntityRef, id: &str) -> bool {
    entity.kind(host).is_some_and(|kind| kind.id(host) == id)
}

fn item_key(host: &dyn Host, id: &str) -> Option<ItemKey> {
    host.item_key(&Identifier::parse(id).ok()?)
}

/// Whether `item` is a member of `tag`, through the engine's merged item tags. An unreadable
/// tag answers false, so the recipe stays inert rather than matching everything.
fn in_item_tag(host: &dyn Host, item: &str, tag: &str) -> bool {
    let Ok(item) = Identifier::parse(item) else {
        return false;
    };
    let Some(item) = host.item_key(&item) else {
        return false;
    };
    let Ok(tag) = Identifier::parse(tag) else {
        return false;
    };
    let Some(tag) = host.tags().key(&tag) else {
        return false;
    };
    host.tags().is_in_tag(item, tag).unwrap_or(false)
}

fn is_theurgy(literals: &[String]) -> bool {
    literals.first().is_some_and(|word| word == "theurgy")
}

/// The three integers `/theurgy info <x> <y> <z>` carries as one `block_pos`.
fn info_position(arguments: &[CommandArgument]) -> Option<[i32; 3]> {
    arguments
        .iter()
        .rev()
        .find(|entry| entry.name == "pos")
        .and_then(|entry| match &entry.value {
            CommandArgumentValue::BlockPos(pos) => Some(*pos),
            _ => None,
        })
}

/// An integer counter: `Int`, or the `Short`/`Byte` upstream wrote some of them as.
fn read_int(nbt: &Compound, key: &str) -> i32 {
    match nbt.get(&key.into()) {
        Some(Tag::Int(value)) => *value,
        Some(Tag::Short(value)) => i32::from(*value),
        Some(Tag::Byte(value)) => i32::from(*value),
        _ => 0,
    }
}

fn write_int(nbt: &mut Compound, key: &str, value: i32) {
    nbt.insert(key.into(), Tag::Int(value));
}

/// An item id the oven remembers between ticks (`onContentTypeChanged`), `""` for an empty
/// slot.
fn read_str(nbt: &Compound, key: &str) -> String {
    match nbt.get(&key.into()) {
        Some(Tag::String(value)) => value.to_string_lossy(),
        _ => String::new(),
    }
}

fn write_str(nbt: &mut Compound, key: &str, value: &str) {
    nbt.insert(key.into(), Tag::String(value.into()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_counter_reads_back() {
        let mut nbt = Compound::new();
        write_int(&mut nbt, KEY_PROGRESS, 42);
        assert_eq!(read_int(&nbt, KEY_PROGRESS), 42);
        assert_eq!(read_int(&nbt, "totalTime"), 0, "a missing key reads zero");
    }

    #[test]
    fn a_short_or_byte_counter_reads_as_an_int() {
        let mut nbt = Compound::new();
        nbt.insert("remainingLitTime".into(), Tag::Short(1600));
        nbt.insert("heatCache".into(), Tag::Byte(1));
        assert_eq!(read_int(&nbt, "remainingLitTime"), 1600);
        assert_eq!(read_int(&nbt, "heatCache"), 1);
    }

    #[test]
    fn a_written_item_key_reads_back() {
        let mut nbt = Compound::new();
        write_str(&mut nbt, KEY_INPUT_ITEM, "minecraft:blackstone");
        assert_eq!(read_str(&nbt, KEY_INPUT_ITEM), "minecraft:blackstone");
        assert_eq!(read_str(&nbt, "missing"), "");
    }

    #[test]
    fn only_the_theurgy_root_is_ours() {
        let words = |words: &[&str]| {
            words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        assert!(is_theurgy(&words(&["theurgy", "info"])));
        assert!(!is_theurgy(&words(&["other", "info"])));
        assert!(!is_theurgy(&[]));
    }

    #[test]
    fn the_info_position_is_one_block_pos() {
        let given = [CommandArgument::new(
            "pos".to_owned(),
            CommandArgumentValue::BlockPos([200, 102, 0]),
        )];
        assert_eq!(info_position(&given), Some([200, 102, 0]));
        let wrong = [CommandArgument::new(
            "pos".to_owned(),
            CommandArgumentValue::Integer(1),
        )];
        assert_eq!(
            info_position(&wrong),
            None,
            "a wrongly typed argument is no position"
        );
    }

    #[test]
    fn an_empty_hand_takes_the_output_before_the_input() {
        assert_eq!(OVEN_TAKE, [OUTPUT, INPUT]);
        assert_eq!(OUTPUT, 1, "upstream's output slot");
        assert_eq!(INPUT, 0, "upstream's input slot");
        assert_eq!(BRAZIER_TAKE, [0]);
    }
}
