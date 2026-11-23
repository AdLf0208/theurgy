// SPDX-License-Identifier: MIT
//! The list and attribute filter screens, laid out and behaving like CodeDefinedGUI 1.14.0's
//! `ListFilterScreen` / `AttributeFilterScreen` (the menus Theurgy 26.3 opens for its filters),
//! in Theurgy's own style (`FilterStyles`).
//!
//! Upstream's are container menus with ghost slots. Here the screen is the mod's own: it shows
//! the player's inventory, a click on a stack there picks up a ghost of it (nothing leaves the
//! inventory), and a click on a ghost slot puts it down, as a carried stack does on a ghost
//! slot. Everything is a draft until the confirm button, which sends it to the server in one
//! piece; closing the screen any other way discards it. The draft names where each ghost came
//! from (a slot the filter already had, or an inventory slot), never a stack of its own.

use std::sync::Mutex;

use mod_api::client::hud_layout::PoseOp;
use mod_api::serde_json::{self, Value, json};
use mod_api::text::TextStyle;
use mod_api::{
    ClientHost, Compound, DrawContext, GuiDraw, Hand, Host, ItemStack, MenuSlotBacking, ScreenView,
    Tag, Text,
};

use crate::filter::{self, GhostSource, StandardKind};
use crate::{host, items};

/// Item tags the attribute screen can offer: the mod API cannot list the tags of an item, so
/// the reference is tested against vanilla's and this package's (`tools/gen_filter_item_tags.py`).
const KNOWN_ITEM_TAGS: &str = include_str!("../data/theurgy/filter_item_tags.json");

const SPRITES: &str = "theurgy:gui/codedefinedgui/";

// `FilterStyles`: Theurgy's tints of the CodeDefinedGUI sprites.
const BACKGROUND_TINT: u32 = 0xFFC2_AA88;
const TOP_BAR_TINT: u32 = 0xFFC4_D8F5;
const SLOT_TINT: u32 = 0xFFB8_946A;
const BUTTON_TINT: u32 = 0xFFB7_8F63;
const BUTTON_HOVER_TINT: u32 = 0xFFC8_9E70;
const BLACK: u32 = 0xFF00_0000;
/// `AbstractScrollSelectionWidget`'s entry text and tooltip header colours.
const SELECTION_TEXT: u32 = 0xFFF3_EBDE;
const SELECTION_HEADER: &str = "#5391E1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    List,
    Attribute,
}

/// A stack shown in a ghost slot (or carried by the cursor), with where the server finds it.
#[derive(Clone)]
struct Ghost {
    stack: ItemStack,
    source: GhostSource,
}

/// One rule of the attribute draft: `kept` is its index among the filter's stored rules.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    kept: Option<usize>,
    type_id: String,
    value: String,
    inverted: bool,
}

/// An attribute the reference stack has, offered as a rule (`AttributeCandidate`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    type_id: &'static str,
    value: String,
}

struct FilterUi {
    kind: Kind,
    hand: Hand,
    title: String,
    // List filter draft.
    deny: bool,
    respect: bool,
    ghosts: Vec<Option<Ghost>>,
    // Attribute filter draft.
    mode: usize,
    reference: Option<Ghost>,
    rules: Vec<Rule>,
    candidates: Vec<Candidate>,
    selected: usize,
    // Shared.
    carried: Option<Ghost>,
    inventory: Vec<Option<ItemStack>>,
    locked_slot: Option<usize>,
    closing: bool,
}

#[derive(Clone, Debug)]
pub struct PendingEdit {
    pub hand: Hand,
    pub action: String,
    pub args: String,
}

static FILTER_UI: Mutex<Option<FilterUi>> = Mutex::new(None);
static UI_ACTIONS: Mutex<Vec<PendingEdit>> = Mutex::new(Vec::new());

const ATTRIBUTE_MODES: [&str; 3] = ["match_any", "match_all", "deny"];

// ---------------------------------------------------------------------------
// Layout (`ListFilterLayout`, `AttributeFilterLayout`, `PlayerInventoryLayoutFragment`)
// ---------------------------------------------------------------------------

/// Positions of a filter screen's parts, relative to its top left corner.
struct Layout {
    size: [f32; 2],
    panel: [f32; 4],
    inventory: [f32; 2],
    confirm: [f32; 2],
    reset: [f32; 2],
    /// Mode buttons, left to right; each has its indicator 6 pixels above.
    toggles: Vec<[f32; 2]>,
    horizontal_separator: [f32; 4],
    vertical_separator: [f32; 4],
}

const BUTTON: f32 = 18.0;
const SLOT: f32 = 18.0;

// List filter: the 2 x 9 ghost grid.
const LIST_GRID: [f32; 2] = [25.0, 23.0];
// Attribute filter: reference slot, candidate selection, add buttons and the rule summary.
const REFERENCE: [f32; 2] = [19.0, 23.0];
const SELECTION: [f32; 4] = [42.0, 22.0, 137.0, 18.0];
const ADD: [f32; 2] = [190.0, 22.0];
const ADD_INVERTED: [f32; 2] = [208.0, 22.0];
const SUMMARY: [f32; 4] = [18.0, 54.0, 24.0, 24.0];

fn layout(kind: Kind) -> Layout {
    match kind {
        Kind::List => Layout {
            size: [214.0, 211.0],
            panel: [3.0, 11.0, 208.0, 87.0],
            inventory: [27.0, 123.0],
            confirm: [181.0, 74.0],
            reset: [152.0, 74.0],
            // deny, allow, respect data, ignore data
            toggles: vec![[18.0, 74.0], [36.0, 74.0], [60.0, 74.0], [78.0, 74.0]],
            horizontal_separator: [3.0, 63.0, 208.0, 1.0],
            vertical_separator: [145.0, 63.0, 1.0, 35.0],
        },
        Kind::Attribute => Layout {
            size: [241.0, 197.0],
            panel: [3.0, 11.0, 235.0, 75.0],
            inventory: [40.0, 109.0],
            confirm: [208.0, 60.0],
            reset: [179.0, 60.0],
            // match any, match all, deny
            toggles: vec![[47.0, 60.0], [65.0, 60.0], [83.0, 60.0]],
            horizontal_separator: [3.0, 47.0, 235.0, 1.0],
            vertical_separator: [202.0, 47.0, 1.0, 39.0],
        },
    }
}

/// The screen's top left corner in a GUI of `gui` pixels.
fn origin(layout: &Layout, gui: [f32; 2]) -> [f32; 2] {
    [((gui[0] - layout.size[0]) / 2.0).floor(), ((gui[1] - layout.size[1]) / 2.0).floor()]
}

/// Where inventory slot `slot` (0 to 8 the hotbar, 9 to 35 the rows above it) is drawn.
fn inventory_slot_at(layout: &Layout, slot: usize) -> [f32; 2] {
    let [x, y] = layout.inventory;
    if slot < 9 {
        [x + slot as f32 * SLOT, y + 58.0]
    } else {
        let index = slot - 9;
        [x + (index % 9) as f32 * SLOT, y + (index / 9) as f32 * SLOT]
    }
}

fn list_slot_at(slot: usize) -> [f32; 2] {
    [LIST_GRID[0] + (slot % 9) as f32 * SLOT, LIST_GRID[1] + (slot / 9) as f32 * SLOT]
}

fn inside(point: [f32; 2], rect: [f32; 4]) -> bool {
    point[0] >= rect[0]
        && point[1] >= rect[1]
        && point[0] < rect[0] + rect[2]
        && point[1] < rect[1] + rect[3]
}

fn square(at: [f32; 2], side: f32) -> [f32; 4] {
    [at[0], at[1], side, side]
}

/// What is under a point of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Inventory(usize),
    ListSlot(usize),
    Reference,
    Toggle(usize),
    Reset,
    Confirm,
    Add,
    AddInverted,
    Selection,
    Summary,
}

/// `point` is relative to the screen's top left corner.
fn target_at(kind: Kind, point: [f32; 2]) -> Option<Target> {
    let layout = layout(kind);
    if let Some(slot) = (0..36).find(|slot| inside(point, square(inventory_slot_at(&layout, *slot), SLOT))) {
        return Some(Target::Inventory(slot));
    }
    if let Some(index) = layout.toggles.iter().position(|at| inside(point, square(*at, BUTTON))) {
        return Some(Target::Toggle(index));
    }
    if inside(point, square(layout.reset, BUTTON)) {
        return Some(Target::Reset);
    }
    if inside(point, square(layout.confirm, BUTTON)) {
        return Some(Target::Confirm);
    }
    match kind {
        Kind::List => (0..filter::LIST_SLOTS)
            .find(|slot| inside(point, square(list_slot_at(*slot), SLOT)))
            .map(Target::ListSlot),
        Kind::Attribute => {
            if inside(point, square(REFERENCE, SLOT)) {
                Some(Target::Reference)
            } else if inside(point, square(ADD, BUTTON)) {
                Some(Target::Add)
            } else if inside(point, square(ADD_INVERTED, BUTTON)) {
                Some(Target::AddInverted)
            } else if inside(point, SELECTION) {
                Some(Target::Selection)
            } else if inside(point, SUMMARY) {
                Some(Target::Summary)
            } else {
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Loading the held filter into a draft
// ---------------------------------------------------------------------------

fn map(tag: Option<Tag>) -> Compound {
    match tag {
        Some(Tag::Compound(map)) => map,
        _ => Compound::new(),
    }
}

fn kept_ghosts(host_ref: &dyn Host, stack: &ItemStack, component: &str) -> Vec<(usize, Ghost)> {
    filter::stored_slots(host_ref, stack, component)
        .into_iter()
        .filter_map(|(slot, item)| {
            let stack = filter::stack_from_template(host_ref, &item)?;
            Some((slot, Ghost { stack, source: GhostSource::Kept(slot) }))
        })
        .collect()
}

fn item_name(host_ref: &dyn ClientHost, id: &str) -> String {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    host_ref
        .translate(&format!("item.{namespace}.{path}"))
        .or_else(|| host_ref.translate(&format!("block.{namespace}.{path}")))
        .unwrap_or_else(|| id.to_owned())
}

fn load(host_ref: &dyn ClientHost, stack: &ItemStack, hand: Hand) -> Option<FilterUi> {
    let id = stack.item().id(host_ref);
    let kind = match id.as_str() {
        "theurgy:list_filter" => Kind::List,
        "theurgy:attribute_filter" => Kind::Attribute,
        _ => return None,
    };
    let mut ui = FilterUi {
        kind,
        hand,
        title: item_name(host_ref, &id),
        deny: false,
        respect: false,
        ghosts: vec![None; filter::LIST_SLOTS],
        mode: 0,
        reference: None,
        rules: Vec::new(),
        candidates: Vec::new(),
        selected: 0,
        carried: None,
        inventory: Vec::new(),
        locked_slot: None,
        closing: false,
    };
    match kind {
        Kind::List => {
            let cfg = map(items::component_tag(host_ref, stack, filter::LIST_CONFIG));
            ui.deny = host::string(&cfg, "mode").as_deref() == Some("deny");
            ui.respect = host::bool(&cfg, "respect_data_components");
            for (slot, ghost) in kept_ghosts(host_ref, stack, filter::LIST_CONTENTS) {
                if let Some(place) = ui.ghosts.get_mut(slot) {
                    *place = Some(ghost);
                }
            }
        }
        Kind::Attribute => {
            let cfg = map(items::component_tag(host_ref, stack, filter::ATTRIBUTE_CONFIG));
            let mode = host::string(&cfg, "mode").unwrap_or_default();
            ui.mode = ATTRIBUTE_MODES.iter().position(|m| *m == mode).unwrap_or(0);
            if let Some(Tag::List(rules)) = cfg.get(&"rules".into()) {
                for (index, rule) in rules.iter().enumerate() {
                    let Tag::Compound(rule) = rule else {
                        continue;
                    };
                    let payload = match rule.get(&"payload".into()) {
                        Some(Tag::Compound(payload)) => payload.clone(),
                        _ => Compound::new(),
                    };
                    ui.rules.push(Rule {
                        kept: Some(index),
                        type_id: host::string(rule, "type").unwrap_or_default(),
                        value: host::string(&payload, "id")
                            .or_else(|| host::string(&payload, "value"))
                            .unwrap_or_default(),
                        inverted: host::bool(rule, "inverted"),
                    });
                }
            }
            ui.reference = kept_ghosts(host_ref, stack, filter::ATTRIBUTE_REFERENCE)
                .into_iter()
                .find(|(slot, _)| *slot == 0)
                .map(|(_, ghost)| ghost);
            ui.candidates = candidates(host_ref, ui.reference.as_ref().map(|ghost| &ghost.stack));
        }
    }
    refresh_inventory(host_ref, &mut ui);
    Some(ui)
}

/// The local player's 36 inventory slots by `Inventory`'s numbers, and the slot the filter is in.
fn refresh_inventory(host_ref: &dyn ClientHost, ui: &mut FilterUi) {
    let mut inventory: Vec<Option<ItemStack>> = vec![None; 36];
    let Some(player) = host_ref.local_player() else {
        ui.inventory = inventory;
        return;
    };
    // The mod's screen is not a container, so the open menu is the player's own inventory menu.
    if let Some(menu) = player.container_menu(host_ref) {
        let count = menu.slot_count(host_ref).unwrap_or(0);
        for index in 0..count {
            let slot = match menu.slot_backing(host_ref, index) {
                Some(MenuSlotBacking::Player(slot)) => Some(slot),
                Some(_) => None,
                // A client is not told what backs a slot: go by the inventory menu's layout.
                None => inventory_menu_slot(count, index),
            };
            if let Some(slot) = slot.filter(|slot| *slot < 36) {
                inventory[slot] = menu.slot(host_ref, index).flatten();
            }
        }
    }
    ui.locked_slot = match ui.hand {
        Hand::Main => player.selected_slot(host_ref),
        Hand::Off => None,
    };
    ui.inventory = inventory;
}

/// The `Inventory` slot behind slot `index` of the player's inventory menu (`InventoryMenu`:
/// 46 slots; 9 to 35 the main rows, 36 to 44 the hotbar). None for the crafting grid, the
/// armour and the off hand, and for a menu of another size.
fn inventory_menu_slot(count: usize, index: usize) -> Option<usize> {
    if count != 46 {
        return None;
    }
    match index {
        9..=35 => Some(index),
        36..=44 => Some(index - 36),
        _ => None,
    }
}

pub fn open(host_ref: &mut dyn ClientHost, stack: &ItemStack, hand: Hand) -> bool {
    let Some(ui) = load(host_ref, stack, hand) else {
        return false;
    };
    host_ref.mod_screen_open("theurgy");
    if let Ok(mut slot) = FILTER_UI.lock() {
        *slot = Some(ui);
        true
    } else {
        host_ref.mod_screen_close("theurgy");
        false
    }
}

pub fn close() {
    if let Ok(mut slot) = FILTER_UI.lock() {
        *slot = None;
    }
}

// ---------------------------------------------------------------------------
// Attribute candidates (`ItemAttributeType.collectCandidates`, in `ItemAttributes` order)
// ---------------------------------------------------------------------------

fn known_item_tags() -> Vec<String> {
    serde_json::from_str::<Vec<String>>(KNOWN_ITEM_TAGS).unwrap_or_default()
}

/// The words of an id's path with capitals, as `StringUtils.capitalize` shows a mod id.
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn candidates(host_ref: &dyn ClientHost, reference: Option<&ItemStack>) -> Vec<Candidate> {
    let Some(stack) = reference else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (id, kind) in StandardKind::ALL {
        if filter::eval_standard(host_ref, &kind, stack) {
            out.push(Candidate { type_id: "codedefinedgui:standard", value: id.to_owned() });
        }
    }
    for tag in known_item_tags() {
        if host::in_item_tag(host_ref, stack.item(), &tag) {
            out.push(Candidate { type_id: "codedefinedgui:in_tag", value: tag });
        }
    }
    let id = stack.item().id(host_ref);
    if let Some((namespace, _)) = id.split_once(':') {
        out.push(Candidate { type_id: "codedefinedgui:added_by", value: namespace.to_owned() });
    }
    if let Some(Tag::Compound(levels)) = items::component_tag(host_ref, stack, "minecraft:enchantments") {
        // 26.3 stores the levels directly; older data nests them under `levels`.
        let levels = match levels.get(&"levels".into()) {
            Some(Tag::Compound(nested)) => nested.clone(),
            _ => levels,
        };
        for key in levels.keys() {
            out.push(Candidate {
                type_id: "codedefinedgui:has_enchantment",
                value: key.to_string_lossy(),
            });
        }
    }
    if let Some(fluid) = filter::stack_fluid_id(host_ref, stack) {
        out.push(Candidate { type_id: "codedefinedgui:has_fluid", value: fluid });
    }
    if let Some(Tag::String(name)) = items::component_tag(host_ref, stack, "minecraft:custom_name") {
        let name = name.to_string_lossy();
        if !name.trim().is_empty() {
            out.push(Candidate { type_id: "codedefinedgui:has_name", value: name });
        }
    }
    out
}

fn translated(key: &str, arguments: Vec<Text>) -> Text {
    Text::translate(key, arguments)
}

/// `ItemAttributeType.describe`: a rule in words.
fn describe(host_ref: &dyn ClientHost, type_id: &str, value: &str, inverted: bool) -> Text {
    let base = match type_id {
        "codedefinedgui:standard" => {
            translated(&format!("codedefinedgui.filter.attribute.standard.{value}"), Vec::new())
        }
        "codedefinedgui:in_tag" => translated(
            "codedefinedgui.filter.attribute.in_tag",
            vec![Text::literal(format!("#{value}"))],
        ),
        "codedefinedgui:added_by" => translated(
            "codedefinedgui.filter.attribute.added_by",
            vec![Text::literal(capitalized(value))],
        ),
        "codedefinedgui:has_enchantment" | "codedefinedgui:enchantment" => {
            let (namespace, path) = value.split_once(':').unwrap_or(("minecraft", value));
            translated(
                "codedefinedgui.filter.attribute.has_enchantment",
                vec![translated(&format!("enchantment.{namespace}.{path}"), Vec::new())],
            )
        }
        "codedefinedgui:has_fluid" | "codedefinedgui:fluid_contents" => translated(
            "codedefinedgui.filter.attribute.has_fluid",
            vec![Text::literal(host_ref.fluid_name(value))],
        ),
        "codedefinedgui:has_name" | "codedefinedgui:item_name" => translated(
            "codedefinedgui.filter.attribute.has_name",
            vec![Text::literal(value.to_owned())],
        ),
        other => Text::literal(other.to_owned()),
    };
    if inverted {
        translated("codedefinedgui.filter.attribute.inverted", vec![base])
    } else {
        base
    }
}

/// `AttributeFilterMenu.canAddSelected`: a candidate already a rule, either way round, is not added again.
fn can_add(rules: &[Rule], candidate: &Candidate) -> bool {
    !rules.iter().any(|rule| rule.type_id == candidate.type_id && rule.value == candidate.value)
}

/// The window of entries a selection tooltip lists around `selected`
/// (`AbstractScrollSelectionWidget.updateTooltip`): at most `visible`, as `start..end`.
fn tooltip_window(selected: usize, count: usize, visible: usize) -> (usize, usize) {
    let start = selected.saturating_sub(visible / 2);
    let end = (start + visible).min(count);
    (start, end)
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn sprite_name(name: &str) -> String {
    format!("{SPRITES}{name}")
}

/// A whole sprite of `size` texels drawn at `at`, tinted.
fn sprite(draw: &mut dyn DrawContext, name: &str, size: [f32; 2], at: [f32; 2], tint: Option<u32>) {
    let _ = draw.sprite(&sprite_name(name), [0.0, 0.0, size[0], size[1]], [at[0], at[1], size[0], size[1]], tint);
}

/// The spans of one axis of a nine-slice: `(texel start, texel length, pixel start, pixel length)`
/// for the leading border, the stretched middle and the trailing border.
fn nine_slice_spans(texture: f32, border: f32, start: f32, length: f32) -> [(f32, f32, f32, f32); 3] {
    let edge = border.min(length / 2.0);
    [
        (0.0, edge, start, edge),
        (border, texture - 2.0 * border, start + edge, length - 2.0 * edge),
        (texture - edge, edge, start + length - edge, edge),
    ]
}

/// A sprite with `border` texel edges stretched over `rect` (`gui.scaling: nine_slice`).
fn nine_slice(draw: &mut dyn DrawContext, name: &str, texture: f32, border: f32, rect: [f32; 4], tint: u32) {
    let name = sprite_name(name);
    for (v, vh, y, h) in nine_slice_spans(texture, border, rect[1], rect[3]) {
        for (u, uw, x, w) in nine_slice_spans(texture, border, rect[0], rect[2]) {
            if w > 0.0 && h > 0.0 {
                let _ = draw.sprite(&name, [u, v, uw, vh], [x, y, w, h], Some(tint));
            }
        }
    }
}

fn panel(draw: &mut dyn DrawContext, rect: [f32; 4], tint: u32) {
    nine_slice(draw, "gui_background", 14.0, 4.0, rect, tint);
}

/// A slot's frame around the item position `at`.
fn slot_frame(draw: &mut dyn DrawContext, at: [f32; 2]) {
    sprite(draw, "inventory_slot", [18.0, 18.0], [at[0] - 1.0, at[1] - 1.0], Some(SLOT_TINT));
}

/// `IconButtonWidget`: pressed while it is the chosen one (or cannot be used), lit under the mouse.
fn icon_button(draw: &mut dyn DrawContext, at: [f32; 2], icon: &str, active: bool, hovered: bool) {
    let (background, tint) = if !active {
        ("button_down", BUTTON_TINT)
    } else if hovered {
        ("button_hover", BUTTON_HOVER_TINT)
    } else {
        ("button", BUTTON_TINT)
    };
    nine_slice(draw, background, 18.0, 2.0, square(at, BUTTON), tint);
    sprite(draw, icon, [16.0, 16.0], [at[0] + 1.0, at[1] + 1.0], None);
}

fn indicator(draw: &mut dyn DrawContext, button: [f32; 2], on: bool) {
    let name = if on { "indicator_green" } else { "indicator" };
    sprite(draw, name, [18.0, 6.0], [button[0], button[1] - 6.0], None);
}

fn styled(text: Text, color: &str, italic: bool) -> Text {
    let mut style = TextStyle::default();
    style.color = Some(color.to_owned());
    if italic {
        style.italic = Some(true);
    }
    text.with_style(style)
}

fn paint_text(draw: &mut dyn DrawContext, text: Text, at: [f32; 2], color: u32) {
    let _ = draw.gui_paint(&[
        GuiDraw::Push(vec![PoseOp::Translate(at[0], at[1])]),
        GuiDraw::Text { text, x: 0, y: 0, color, shadow: false },
        GuiDraw::Pop,
    ]);
}

/// A button's tooltip: its name, then its explanation while Shift is held (else the hint).
fn button_tooltip(key: &str, detailed: bool, shift: bool) -> Vec<Text> {
    let mut lines = vec![translated(&format!("{key}.tooltip"), Vec::new())];
    if detailed {
        lines.push(if shift {
            styled(translated(&format!("{key}.tooltip.shift"), Vec::new()), "gray", false)
        } else {
            translated("codedefinedgui.tooltip.show_extended", Vec::new())
        });
    }
    lines
}

/// `AbstractScrollSelectionWidget.updateTooltip`.
fn selection_tooltip(host_ref: &dyn ClientHost, ui: &FilterUi) -> Vec<Text> {
    let mut lines = vec![styled(
        translated("codedefinedgui.filter.attribute.available", Vec::new()),
        SELECTION_HEADER,
        false,
    )];
    if ui.candidates.is_empty() {
        lines.push(styled(
            translated("codedefinedgui.filter.attribute.no_reference", Vec::new()),
            "gray",
            false,
        ));
        return lines;
    }
    let selected = ui.selected.min(ui.candidates.len() - 1);
    let (start, end) = tooltip_window(selected, ui.candidates.len(), 8);
    let more = || styled(Text::literal("> ..."), "gray", false);
    if start > 0 {
        lines.push(more());
    }
    for index in start..end {
        let candidate = &ui.candidates[index];
        let (prefix, color) = if index == selected { ("-> ", "white") } else { ("> ", "gray") };
        lines.push(styled(
            Text::literal(prefix).append(describe(host_ref, candidate.type_id, &candidate.value, false)),
            color,
            false,
        ));
    }
    if end < ui.candidates.len() {
        lines.push(more());
    }
    lines.push(styled(
        translated("codedefinedgui.filter.attribute.scroll_to_select", Vec::new()),
        "dark_gray",
        true,
    ));
    lines
}

/// `AttributeFilterDefinition.summary`: the mode and the first four rules.
fn summary_tooltip(host_ref: &dyn ClientHost, ui: &FilterUi) -> Vec<Text> {
    if ui.rules.is_empty() {
        return vec![translated("codedefinedgui.filter.attribute.no_rules", Vec::new())];
    }
    let mode = translated(
        &format!("codedefinedgui.filter.attribute.mode.{}", ATTRIBUTE_MODES[ui.mode]),
        Vec::new(),
    );
    let mut lines = vec![styled(
        translated("codedefinedgui.filter.attribute.summary.mode", vec![mode]),
        "gold",
        false,
    )];
    for rule in ui.rules.iter().take(4) {
        lines.push(
            Text::literal("- ").append(describe(host_ref, &rule.type_id, &rule.value, rule.inverted)),
        );
    }
    if ui.rules.len() > 4 {
        lines.push(styled(
            translated(
                "codedefinedgui.filter.summary.more",
                vec![Text::literal((ui.rules.len() - 4).to_string())],
            ),
            "dark_gray",
            false,
        ));
    }
    lines
}

fn stack_tooltip(host_ref: &dyn ClientHost, stack: &ItemStack) -> Vec<Text> {
    vec![Text::literal(item_name(host_ref, &stack.item().id(host_ref)))]
}

/// The toggle buttons of a screen: `(icon, lang key, whether it is the chosen one)`.
fn toggles(ui: &FilterUi) -> Vec<(&'static str, &'static str, bool)> {
    match ui.kind {
        Kind::List => vec![
            ("deny_list", "codedefinedgui.filter.list.mode.deny", ui.deny),
            ("accept_list", "codedefinedgui.filter.list.mode.allow", !ui.deny),
            ("respect_data_components", "codedefinedgui.filter.list.respect_data", ui.respect),
            ("ignore_data_components", "codedefinedgui.filter.list.ignore_data", !ui.respect),
        ],
        Kind::Attribute => vec![
            ("accept_list_or", "codedefinedgui.filter.attribute.mode.match_any", ui.mode == 0),
            ("accept_list_and", "codedefinedgui.filter.attribute.mode.match_all", ui.mode == 1),
            ("deny_list_alt", "codedefinedgui.filter.attribute.mode.deny", ui.mode == 2),
        ],
    }
}

pub fn render(
    host_ref: &dyn ClientHost,
    draw: &mut dyn DrawContext,
    screen: &ScreenView<'_>,
) -> bool {
    if screen.id != "mod" || host_ref.mod_screen() != Some("theurgy") {
        return false;
    }
    let Ok(guard) = FILTER_UI.lock() else {
        return false;
    };
    let Some(ui) = guard.as_ref() else {
        return false;
    };
    let gui = draw.size();
    let layout = layout(ui.kind);
    let [ox, oy] = origin(&layout, gui);
    let at = |point: [f32; 2]| [ox + point[0], oy + point[1]];
    let rect = |r: [f32; 4]| [ox + r[0], oy + r[1], r[2], r[3]];
    let mouse = [screen.mouse[0] - ox, screen.mouse[1] - oy];
    let hovered = target_at(ui.kind, mouse);
    let shift = host_ref.modifiers().shift();
    let mut tooltip: Option<Vec<Text>> = None;

    draw.fill([0.0, 0.0, gui[0], gui[1]], 0xA000_0000);
    // The panel goes under the top bar, which hides its upper border.
    panel(draw, rect(layout.panel), BACKGROUND_TINT);
    let inventory = layout.inventory;
    panel(draw, rect([inventory[0] - 8.0, inventory[1] - 8.0, 176.0, 90.0]), BACKGROUND_TINT);
    panel(draw, rect([0.0, 0.0, layout.size[0], 15.0]), TOP_BAR_TINT);
    let title_x = (layout.size[0] - draw.text_width(&ui.title)) / 2.0;
    draw.text(&ui.title, at([title_x.floor(), 4.0]), BLACK, false);
    draw.fill(rect(layout.horizontal_separator), BLACK);
    draw.fill(rect(layout.vertical_separator), BLACK);

    // Player inventory: the stacks are only shown; the filter's own slot cannot be picked.
    for slot in 0..36 {
        let pos = at(inventory_slot_at(&layout, slot));
        slot_frame(draw, pos);
        if let Some(stack) = ui.inventory.get(slot).and_then(Option::as_ref) {
            draw.gui_draw_item_with(stack, pos, None);
            if hovered == Some(Target::Inventory(slot)) && ui.carried.is_none() {
                tooltip = Some(stack_tooltip(host_ref, stack));
            }
        }
        if ui.locked_slot == Some(slot) {
            draw.fill(square(pos, 16.0), 0x6000_0000);
        } else if hovered == Some(Target::Inventory(slot)) {
            draw.fill(square(pos, 16.0), 0x80FF_FFFF);
        }
    }

    // Mode buttons with their indicators, reset and confirm.
    for (index, (icon, key, chosen)) in toggles(ui).into_iter().enumerate() {
        let pos = layout.toggles[index];
        let over = hovered == Some(Target::Toggle(index));
        icon_button(draw, at(pos), icon, !chosen, over);
        indicator(draw, at(pos), chosen);
        if over {
            tooltip = Some(button_tooltip(key, true, shift));
        }
    }
    icon_button(draw, at(layout.reset), "trash", true, hovered == Some(Target::Reset));
    icon_button(draw, at(layout.confirm), "confirm", true, hovered == Some(Target::Confirm));
    match hovered {
        Some(Target::Reset) => {
            tooltip = Some(button_tooltip("codedefinedgui.filter.button.reset", false, shift));
        }
        Some(Target::Confirm) => {
            tooltip = Some(button_tooltip("codedefinedgui.filter.button.done", false, shift));
        }
        _ => {}
    }

    let mut ghost_slot = |draw: &mut dyn DrawContext, pos: [f32; 2], ghost: Option<&Ghost>, over: bool| {
        let pos = at(pos);
        slot_frame(draw, pos);
        if let Some(ghost) = ghost {
            draw.gui_draw_item_with(&ghost.stack, pos, None);
            if over && ui.carried.is_none() {
                tooltip = Some(stack_tooltip(host_ref, &ghost.stack));
            }
        }
        if over {
            draw.fill(square(pos, 16.0), 0x80FF_FFFF);
        }
    };
    match ui.kind {
        Kind::List => {
            for slot in 0..filter::LIST_SLOTS {
                let ghost = ui.ghosts.get(slot).and_then(Option::as_ref);
                ghost_slot(draw, list_slot_at(slot), ghost, hovered == Some(Target::ListSlot(slot)));
            }
        }
        Kind::Attribute => {
            ghost_slot(draw, REFERENCE, ui.reference.as_ref(), hovered == Some(Target::Reference));
            // The selected candidate, clipped to its box.
            let box_rect = rect(SELECTION);
            let _ = draw.sprite(
                &sprite_name("attribute_filter_selection"),
                [0.0, 0.0, 137.0, 18.0],
                box_rect,
                Some(SLOT_TINT),
            );
            let selected = ui.candidates.get(ui.selected.min(ui.candidates.len().saturating_sub(1)));
            let label = match selected {
                Some(candidate) => describe(host_ref, candidate.type_id, &candidate.value, false),
                None => translated("codedefinedgui.filter.attribute.no_reference", Vec::new()),
            };
            let clip = [box_rect[0] + 6.0, box_rect[1] + 1.0, box_rect[2] - 12.0, box_rect[3] - 2.0];
            if draw.push_clip(clip).is_ok() {
                paint_text(draw, label, [box_rect[0] + 6.0, box_rect[1] + 5.0], SELECTION_TEXT);
                let _ = draw.pop_clip();
            }
            let addable = selected.is_some_and(|candidate| can_add(&ui.rules, candidate));
            icon_button(draw, at(ADD), "add", addable, hovered == Some(Target::Add));
            icon_button(draw, at(ADD_INVERTED), "add_inverted", addable, hovered == Some(Target::AddInverted));
            // The rule summary: a slot-styled box with a name tag and the number of rules.
            let summary = rect(SUMMARY);
            nine_slice(draw, "inventory_slot", 18.0, 1.0, summary, SLOT_TINT);
            if let Some(tag) = host::item_key(host_ref, "minecraft:name_tag") {
                let _ = draw.item(tag, [summary[0] + 4.0, summary[1] + 4.0], 16.0);
            }
            if !ui.rules.is_empty() {
                draw.text(&ui.rules.len().to_string(), [summary[0] + 9.0, summary[1] + 9.0], 0xFFFF_FFFF, false);
            }
            match hovered {
                Some(Target::Selection) => tooltip = Some(selection_tooltip(host_ref, ui)),
                Some(Target::Summary) => tooltip = Some(summary_tooltip(host_ref, ui)),
                Some(Target::Add) => {
                    tooltip = Some(button_tooltip("codedefinedgui.filter.attribute.add", false, shift));
                }
                Some(Target::AddInverted) => {
                    tooltip = Some(button_tooltip(
                        "codedefinedgui.filter.attribute.add_inverted",
                        false,
                        shift,
                    ));
                }
                _ => {}
            }
        }
    }

    // The ghost on the cursor, as a carried stack is drawn.
    if let Some(carried) = ui.carried.as_ref() {
        draw.gui_draw_item_with(&carried.stack, [screen.mouse[0] - 8.0, screen.mouse[1] - 8.0], None);
    } else if let Some(lines) = tooltip {
        let _ = draw.gui_tooltip(&lines, screen.mouse);
    }
    true
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

fn same_stack(a: &ItemStack, b: &ItemStack) -> bool {
    a.item() == b.item() && a.is_stackable_with(b)
}

/// The inventory stack in `slot`, as a ghost of one item.
fn inventory_ghost(ui: &FilterUi, slot: usize) -> Option<Ghost> {
    if ui.locked_slot == Some(slot) {
        return None;
    }
    let mut stack = ui.inventory.get(slot)?.clone()?;
    if stack.count() == 0 {
        return None;
    }
    let _ = stack.set_count(1);
    Some(Ghost { stack, source: GhostSource::Inventory(slot as u32) })
}

fn set_reference(host_ref: &dyn ClientHost, ui: &mut FilterUi, ghost: Option<Ghost>) {
    ui.reference = ghost;
    ui.selected = 0;
    ui.candidates = candidates(host_ref, ui.reference.as_ref().map(|ghost| &ghost.stack));
}

fn add_rule(ui: &mut FilterUi, inverted: bool) {
    let Some(candidate) = ui.candidates.get(ui.selected.min(ui.candidates.len().saturating_sub(1))) else {
        return;
    };
    if !can_add(&ui.rules, candidate) || ui.rules.len() >= filter::MAX_RULES {
        return;
    }
    ui.rules.push(Rule {
        kept: None,
        type_id: candidate.type_id.to_owned(),
        value: candidate.value.clone(),
        inverted,
    });
}

fn ghost_json(ghost: Option<&Ghost>) -> Value {
    match ghost.map(|ghost| ghost.source) {
        Some(GhostSource::Kept(slot)) => json!({ "keep": slot }),
        Some(GhostSource::Inventory(slot)) => json!({ "inv": slot }),
        _ => Value::Null,
    }
}

/// The draft as the server's `commit` action reads it.
fn draft_json(ui: &FilterUi) -> Value {
    match ui.kind {
        Kind::List => json!({
            "mode": if ui.deny { "deny" } else { "allow" },
            "respect": ui.respect,
            "slots": ui.ghosts.iter().map(|ghost| ghost_json(ghost.as_ref())).collect::<Vec<_>>(),
        }),
        Kind::Attribute => json!({
            "mode": ATTRIBUTE_MODES[ui.mode],
            "reference": ghost_json(ui.reference.as_ref()),
            "rules": ui
                .rules
                .iter()
                .map(|rule| match rule.kept {
                    Some(index) => json!({ "keep": index }),
                    None => json!({ "type": rule.type_id, "value": rule.value, "inverted": rule.inverted }),
                })
                .collect::<Vec<_>>(),
        }),
    }
}

fn confirm(ui: &mut FilterUi) {
    if let Ok(mut actions) = UI_ACTIONS.lock() {
        actions.push(PendingEdit {
            hand: ui.hand,
            action: "commit".to_owned(),
            args: draft_json(ui).to_string(),
        });
    }
    ui.closing = true;
}

pub fn click(host_ref: &dyn ClientHost, screen: &ScreenView<'_>, button_index: i32) -> bool {
    if screen.id != "mod" || host_ref.mod_screen() != Some("theurgy") || button_index != 0 {
        return false;
    }
    let Ok(mut guard) = FILTER_UI.lock() else {
        return false;
    };
    let Some(ui) = guard.as_mut() else {
        return false;
    };
    let layout = layout(ui.kind);
    let [ox, oy] = origin(&layout, [screen.rect[2], screen.rect[3]]);
    let point = [screen.mouse[0] - ox, screen.mouse[1] - oy];
    let shift = host_ref.modifiers().shift();
    let Some(target) = target_at(ui.kind, point) else {
        // A click at nothing lets go of the ghost on the cursor.
        ui.carried = None;
        return true;
    };
    match target {
        Target::Inventory(slot) => {
            let ghost = inventory_ghost(ui, slot);
            if !shift {
                ui.carried = ghost;
            } else if let Some(ghost) = ghost {
                // `quickMoveStack`: into the first free ghost slot, unless it is listed already.
                match ui.kind {
                    Kind::List => {
                        let listed = ui
                            .ghosts
                            .iter()
                            .flatten()
                            .any(|existing| same_stack(&existing.stack, &ghost.stack));
                        if !listed && let Some(free) = ui.ghosts.iter_mut().find(|slot| slot.is_none()) {
                            *free = Some(ghost);
                        }
                    }
                    Kind::Attribute => set_reference(host_ref, ui, Some(ghost)),
                }
            }
        }
        // `FilterMenu.clicked`: a ghost slot takes a copy of what is carried, or empties.
        Target::ListSlot(slot) => {
            ui.ghosts[slot] = if shift { None } else { ui.carried.clone() };
        }
        Target::Reference => {
            let ghost = if shift { None } else { ui.carried.clone() };
            set_reference(host_ref, ui, ghost);
        }
        Target::Toggle(index) => match ui.kind {
            Kind::List => match index {
                0 => ui.deny = true,
                1 => ui.deny = false,
                2 => ui.respect = true,
                _ => ui.respect = false,
            },
            Kind::Attribute => ui.mode = index.min(2),
        },
        Target::Reset => match ui.kind {
            Kind::List => ui.ghosts.iter_mut().for_each(|ghost| *ghost = None),
            Kind::Attribute => ui.rules.clear(),
        },
        Target::Confirm => confirm(ui),
        Target::Add => add_rule(ui, false),
        Target::AddInverted => add_rule(ui, true),
        Target::Selection => {
            if !ui.candidates.is_empty() {
                ui.selected = (ui.selected + 1) % ui.candidates.len();
            }
        }
        Target::Summary => {}
    }
    true
}

pub fn scroll(host_ref: &dyn ClientHost, screen: &ScreenView<'_>, delta: f64) -> bool {
    if screen.id != "mod" || host_ref.mod_screen() != Some("theurgy") {
        return false;
    }
    let Ok(mut guard) = FILTER_UI.lock() else {
        return false;
    };
    let Some(ui) = guard.as_mut() else {
        return false;
    };
    let layout = layout(ui.kind);
    let [ox, oy] = origin(&layout, [screen.rect[2], screen.rect[3]]);
    let point = [screen.mouse[0] - ox, screen.mouse[1] - oy];
    if target_at(ui.kind, point) == Some(Target::Selection) && !ui.candidates.is_empty() && delta != 0.0 {
        let count = ui.candidates.len();
        let current = ui.selected.min(count - 1);
        // Scrolling up goes back one entry, down forward one, wrapping.
        ui.selected = if delta > 0.0 { (current + count - 1) % count } else { (current + 1) % count };
    }
    true
}

pub fn char_typed(host_ref: &dyn ClientHost, screen: &ScreenView<'_>, _c: char) -> bool {
    screen.id == "mod"
        && host_ref.mod_screen() == Some("theurgy")
        && FILTER_UI.lock().is_ok_and(|slot| slot.is_some())
}

/// Keys are the host's: Escape closes the screen, which discards the draft.
pub fn key_pressed(_host_ref: &dyn ClientHost, _screen: &ScreenView<'_>, _key: &str) -> bool {
    false
}

pub fn take_actions() -> Vec<PendingEdit> {
    UI_ACTIONS
        .lock()
        .map(|mut actions| std::mem::take(&mut *actions))
        .unwrap_or_default()
}

/// Each client tick: follow the inventory, and close the screen once confirmed or once the
/// screen was closed some other way (`FilterMenu.stillValid`: the filter left the hand).
pub fn tick(host_ref: &mut dyn ClientHost) {
    let close_screen = {
        let Ok(mut slot) = FILTER_UI.lock() else {
            return;
        };
        let Some(ui) = slot.as_mut() else {
            return;
        };
        let open = host_ref.mod_screen() == Some("theurgy");
        let held = host_ref
            .local_player()
            .and_then(|player| host_ref.living(player.entity()))
            .and_then(|living| living.hand_item(host_ref, ui.hand))
            .map(|stack| stack.item().id(host_ref));
        let expected = match ui.kind {
            Kind::List => "theurgy:list_filter",
            Kind::Attribute => "theurgy:attribute_filter",
        };
        if ui.closing || !open || held.as_deref() != Some(expected) {
            *slot = None;
            open
        } else {
            refresh_inventory(host_ref, ui);
            false
        }
    };
    if close_screen {
        host_ref.mod_screen_close("theurgy");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_boxes_exclude_the_far_edge() {
        assert!(inside([4.0, 4.0], [4.0, 4.0, 10.0, 10.0]));
        assert!(!inside([14.0, 4.0], [4.0, 4.0, 10.0, 10.0]));
    }

    #[test]
    fn every_part_fits_its_screen_and_none_overlap() {
        for kind in [Kind::List, Kind::Attribute] {
            let layout = layout(kind);
            let mut rects: Vec<[f32; 4]> = (0..36).map(|slot| square(inventory_slot_at(&layout, slot), SLOT)).collect();
            rects.extend(layout.toggles.iter().map(|at| square(*at, BUTTON)));
            rects.push(square(layout.reset, BUTTON));
            rects.push(square(layout.confirm, BUTTON));
            match kind {
                Kind::List => rects.extend((0..filter::LIST_SLOTS).map(|slot| square(list_slot_at(slot), SLOT))),
                Kind::Attribute => {
                    rects.extend([square(REFERENCE, SLOT), square(ADD, BUTTON), square(ADD_INVERTED, BUTTON), SELECTION, SUMMARY]);
                }
            }
            for (index, a) in rects.iter().enumerate() {
                assert!(a[0] >= 0.0 && a[1] >= 0.0, "{kind:?} {a:?}");
                assert!(a[0] + a[2] <= layout.size[0] && a[1] + a[3] <= layout.size[1], "{kind:?} {a:?}");
                for b in &rects[index + 1..] {
                    let apart = a[0] + a[2] <= b[0] || b[0] + b[2] <= a[0] || a[1] + a[3] <= b[1] || b[1] + b[3] <= a[1];
                    assert!(apart, "{kind:?}: {a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn inventory_slots_follow_the_inventory_numbers() {
        let layout = layout(Kind::List);
        // The hotbar is the bottom row, slot 9 the top left of the three rows above it.
        assert_eq!(inventory_slot_at(&layout, 0), [27.0, 181.0]);
        assert_eq!(inventory_slot_at(&layout, 8), [171.0, 181.0]);
        assert_eq!(inventory_slot_at(&layout, 9), [27.0, 123.0]);
        assert_eq!(inventory_slot_at(&layout, 35), [171.0, 159.0]);
        assert_eq!(target_at(Kind::List, [28.0, 182.0]), Some(Target::Inventory(0)));
        assert_eq!(target_at(Kind::List, [26.0, 24.0]), Some(Target::ListSlot(0)));
        assert_eq!(target_at(Kind::List, [25.0 + 8.0 * 18.0 + 1.0, 23.0 + 19.0]), Some(Target::ListSlot(17)));
        assert_eq!(target_at(Kind::Attribute, [20.0, 24.0]), Some(Target::Reference));
        assert_eq!(target_at(Kind::Attribute, [100.0, 30.0]), Some(Target::Selection));
        assert_eq!(target_at(Kind::Attribute, [1.0, 1.0]), None);
    }

    #[test]
    fn the_inventory_menu_maps_onto_inventory_slots() {
        assert_eq!(inventory_menu_slot(46, 0), None);
        assert_eq!(inventory_menu_slot(46, 8), None);
        assert_eq!(inventory_menu_slot(46, 9), Some(9));
        assert_eq!(inventory_menu_slot(46, 35), Some(35));
        assert_eq!(inventory_menu_slot(46, 36), Some(0));
        assert_eq!(inventory_menu_slot(46, 44), Some(8));
        assert_eq!(inventory_menu_slot(46, 45), None);
        assert_eq!(inventory_menu_slot(63, 40), None);
    }

    #[test]
    fn nine_slice_keeps_its_borders_and_stretches_the_middle() {
        let spans = nine_slice_spans(14.0, 4.0, 10.0, 100.0);
        assert_eq!(spans[0], (0.0, 4.0, 10.0, 4.0));
        assert_eq!(spans[1], (4.0, 6.0, 14.0, 92.0));
        assert_eq!(spans[2], (10.0, 4.0, 106.0, 4.0));
        // A box thinner than two borders splits them evenly and has no middle.
        let thin = nine_slice_spans(14.0, 4.0, 0.0, 6.0);
        assert_eq!(thin[0].3 + thin[2].3, 6.0);
        assert_eq!(thin[1].3, 0.0);
    }

    #[test]
    fn a_candidate_is_added_once_whichever_way_round() {
        let candidate = Candidate { type_id: "codedefinedgui:in_tag", value: "c:ingots".into() };
        let mut rules = Vec::new();
        assert!(can_add(&rules, &candidate));
        rules.push(Rule { kept: None, type_id: candidate.type_id.into(), value: candidate.value.clone(), inverted: true });
        assert!(!can_add(&rules, &candidate));
        let other = Candidate { type_id: "codedefinedgui:added_by", value: "c:ingots".into() };
        assert!(can_add(&rules, &other));
    }

    #[test]
    fn selection_tooltip_lists_eight_entries_around_the_selected_one() {
        assert_eq!(tooltip_window(0, 3, 8), (0, 3));
        assert_eq!(tooltip_window(0, 20, 8), (0, 8));
        assert_eq!(tooltip_window(10, 20, 8), (6, 14));
        assert_eq!(tooltip_window(19, 20, 8), (15, 20));
    }

    #[test]
    fn the_known_item_tags_are_a_list_of_tag_names() {
        let tags = known_item_tags();
        assert!(tags.len() > 100);
        assert!(tags.iter().any(|tag| tag == "c:ingots"));
        assert!(tags.iter().any(|tag| tag == "minecraft:planks"));
        assert!(tags.iter().any(|tag| tag.starts_with("theurgy:")));
    }

    #[test]
    fn a_draft_only_names_where_its_ghosts_come_from() {
        assert_eq!(ghost_json(None), Value::Null);
        assert_eq!(capitalized("theurgy"), "Theurgy");
    }
}
