// SPDX-License-Identifier: MIT
//! Item & fluid list/attribute filters (`ListFilter`, `AttributeFilter`),
//! all 14 standard rules + 4 special rules + tag matching, and server-side
//! filter editing (`/theurgy_filter` and client filter screen payloads).

use mod_api::serde_json::Value;
use mod_api::{
    Command, CommandArgumentValue, Compound, Flow, Hand, Host, HostTransaction, ItemPlace,
    ItemStack, RunModCommandEvent, ServerHost, Tag, Text,
};

use crate::{host, items, recipes};

pub const LIST_CONTENTS: &str = "codedefinedgui:list_filter_contents";
pub const LIST_CONFIG: &str = "codedefinedgui:list_filter_config";
pub const ATTRIBUTE_CONFIG: &str = "codedefinedgui:attribute_filter_config";
pub const ATTRIBUTE_REFERENCE: &str = "codedefinedgui:attribute_filter_reference";

/// List filter rule parsed from a `theurgy:list_filter` item's components.
#[derive(Clone, Debug, Default)]
pub struct ListFilter {
    pub deny: bool,
    pub respect: bool,
    pub entries: Vec<ItemStack>,
}

impl ListFilter {
    pub fn from_stack(host_ref: &dyn Host, filter: Option<&ItemStack>) -> Self {
        let mut out = Self::default();
        let Some(filter) = filter else {
            return out;
        };
        if filter.item().id(host_ref) != "theurgy:list_filter" {
            return out;
        }
        if let Some(Tag::Compound(cfg)) = items::component_tag(host_ref, filter, LIST_CONFIG) {
            if let Some(Tag::String(mode)) = cfg.get(&"mode".into()) {
                out.deny = mode.to_string_lossy() == "deny";
            }
            out.respect = host::bool(&cfg, "respect_data_components");
        }
        let contents = items::component_tag(host_ref, filter, LIST_CONTENTS);
        let slots: Vec<Tag> = match contents {
            Some(Tag::List(list)) => list,
            Some(Tag::Compound(map)) => match map.get(&"slots".into()) {
                Some(Tag::List(list)) => list.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        for slot in slots {
            let Tag::Compound(slot_map) = &slot else {
                continue;
            };
            let item_tag = slot_map.get(&"item".into()).unwrap_or(&slot);
            if let Some(stack) = stack_from_template(host_ref, item_tag) {
                out.entries.push(stack);
            }
        }
        out
    }

    pub fn allows_item(&self, stack: &ItemStack) -> bool {
        if self.entries.is_empty() {
            return true;
        }
        let matched = self.entries.iter().any(|entry| {
            entry.item() == stack.item()
                && (!self.respect || entry.is_stackable_with(stack))
        });
        if self.deny { !matched } else { matched }
    }

    pub fn allows_fluid(&self, host_ref: &dyn Host, fluid_id: &str) -> bool {
        if self.entries.is_empty() {
            return true;
        }
        let matched = self
            .entries
            .iter()
            .any(|entry| stack_fluid_id(host_ref, entry).as_deref() == Some(fluid_id));
        if self.deny { !matched } else { matched }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeMode {
    MatchAny,
    MatchAll,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StandardKind {
    Placeable,
    Consumable,
    FluidContainer,
    Enchanted,
    Renamed,
    Damaged,
    BadlyDamaged,
    NotStackable,
    Equipable,
    FurnaceFuel,
    Smeltable,
    Smokable,
    Blastable,
    Compostable,
}

impl StandardKind {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.to_ascii_lowercase().as_str() {
            "placeable" => Self::Placeable,
            "consumable" => Self::Consumable,
            "fluid_container" => Self::FluidContainer,
            "enchanted" => Self::Enchanted,
            "renamed" => Self::Renamed,
            "damaged" => Self::Damaged,
            "badly_damaged" => Self::BadlyDamaged,
            "not_stackable" => Self::NotStackable,
            "equipable" => Self::Equipable,
            "furnace_fuel" => Self::FurnaceFuel,
            "smeltable" => Self::Smeltable,
            "smokable" => Self::Smokable,
            "blastable" => Self::Blastable,
            "compostable" => Self::Compostable,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttributeRule {
    InTag { tag: String, inverted: bool },
    Standard { kind: StandardKind, inverted: bool },
    AddedBy { namespace: String, inverted: bool },
    Enchantment { id: String, inverted: bool },
    FluidContents { id: String, inverted: bool },
    ItemName { value: String, inverted: bool },
    Unknown,
}

#[derive(Clone, Debug)]
pub struct AttributeFilter {
    pub mode: AttributeMode,
    pub rules: Vec<AttributeRule>,
}

impl Default for AttributeFilter {
    fn default() -> Self {
        Self {
            mode: AttributeMode::MatchAny,
            rules: Vec::new(),
        }
    }
}

fn payload_str(payload: &Compound, key: &str) -> Option<String> {
    match payload.get(&key.into()) {
        Some(Tag::String(v)) => Some(v.to_string_lossy()),
        _ => None,
    }
}

impl AttributeFilter {
    pub fn from_config_nbt(config: Option<&Compound>) -> Self {
        let mut out = Self::default();
        let Some(config) = config else {
            return out;
        };
        if let Some(Tag::String(mode)) = config.get(&"mode".into()) {
            out.mode = match mode.to_string_lossy().as_str() {
                "match_all" => AttributeMode::MatchAll,
                "deny" => AttributeMode::Deny,
                _ => AttributeMode::MatchAny,
            };
        }
        let Some(Tag::List(rules)) = config.get(&"rules".into()) else {
            return out;
        };
        for entry in rules {
            let Tag::Compound(rule_map) = entry else {
                out.rules.push(AttributeRule::Unknown);
                continue;
            };
            let inverted = host::bool(rule_map, "inverted");
            let Some(Tag::String(kind_tag)) = rule_map.get(&"type".into()) else {
                out.rules.push(AttributeRule::Unknown);
                continue;
            };
            let kind = kind_tag.to_string_lossy();
            let Some(Tag::Compound(payload)) = rule_map.get(&"payload".into()) else {
                out.rules.push(AttributeRule::Unknown);
                continue;
            };
            let parsed = match kind.as_str() {
                "codedefinedgui:in_tag" => payload_str(payload, "id")
                    .map(|tag| AttributeRule::InTag { tag, inverted }),
                "codedefinedgui:standard" => payload_str(payload, "value")
                    .and_then(|v| StandardKind::parse(&v))
                    .map(|kind| AttributeRule::Standard { kind, inverted }),
                "codedefinedgui:added_by" => payload_str(payload, "id")
                    .map(|namespace| AttributeRule::AddedBy { namespace, inverted }),
                "codedefinedgui:enchantment" => payload_str(payload, "id")
                    .map(|id| AttributeRule::Enchantment { id, inverted }),
                "codedefinedgui:fluid_contents" => payload_str(payload, "id")
                    .map(|id| AttributeRule::FluidContents { id, inverted }),
                "codedefinedgui:item_name" => payload_str(payload, "value")
                    .map(|value| AttributeRule::ItemName { value, inverted }),
                _ => None,
            };
            out.rules.push(parsed.unwrap_or(AttributeRule::Unknown));
        }
        out
    }

    pub fn from_stack(host_ref: &dyn Host, filter: Option<&ItemStack>) -> Self {
        let Some(filter) = filter else {
            return Self::default();
        };
        if filter.item().id(host_ref) != "theurgy:attribute_filter" {
            return Self::default();
        }
        match items::component_tag(host_ref, filter, ATTRIBUTE_CONFIG) {
            Some(Tag::Compound(cfg)) => Self::from_config_nbt(Some(&cfg)),
            _ => Self::default(),
        }
    }

    pub fn is_valid(&self) -> bool {
        !self
            .rules
            .iter()
            .any(|rule| matches!(rule, AttributeRule::Unknown))
    }

    pub fn allows_item(&self, host_ref: &dyn Host, stack: &ItemStack) -> bool {
        if self.rules.is_empty() {
            return true;
        }
        let mut any = false;
        let mut all = true;
        for rule in &self.rules {
            let matched = eval_rule(host_ref, rule, stack);
            any |= matched;
            all &= matched;
        }
        match self.mode {
            AttributeMode::MatchAny => any,
            AttributeMode::MatchAll => all,
            AttributeMode::Deny => !any,
        }
    }
}

pub fn stack_fluid_id(host_ref: &dyn Host, stack: &ItemStack) -> Option<String> {
    let item_id = stack.item().id(host_ref);
    match item_id.as_str() {
        "minecraft:water_bucket" => return Some("minecraft:water".to_owned()),
        "minecraft:lava_bucket" => return Some("minecraft:lava".to_owned()),
        "theurgy:sal_ammoniac_bucket" => return Some("theurgy:sal_ammoniac".to_owned()),
        "minecraft:bucket" => return None,
        _ => {}
    }
    let Some(Tag::Compound(custom)) =
        items::component_tag(host_ref, stack, "minecraft:custom_data")
    else {
        return None;
    };
    let Some(Tag::Compound(fluid)) = custom.get(&"horizon_fluid".into()) else {
        return None;
    };
    if host::int(fluid, "Amount", 0) <= 0 {
        return None;
    }
    host::string(fluid, "Fluid")
}

fn eval_rule(host_ref: &dyn Host, rule: &AttributeRule, stack: &ItemStack) -> bool {
    match rule {
        AttributeRule::InTag { tag, inverted } => {
            host::in_item_tag(host_ref, stack.item(), tag) ^ *inverted
        }
        AttributeRule::Standard { kind, inverted } => {
            eval_standard(host_ref, kind, stack) ^ *inverted
        }
        AttributeRule::AddedBy {
            namespace,
            inverted,
        } => {
            let id = stack.item().id(host_ref);
            let ns = id.split(':').next().unwrap_or("minecraft");
            (ns == namespace) ^ *inverted
        }
        AttributeRule::Enchantment { id, inverted } => {
            let has = matches!(
                items::component_tag(host_ref, stack, "minecraft:enchantments"),
                Some(Tag::Compound(levels)) if levels.contains_key(&id.as_str().into())
            );
            has ^ *inverted
        }
        AttributeRule::FluidContents { id, inverted } => {
            (stack_fluid_id(host_ref, stack).as_deref() == Some(id.as_str())) ^ *inverted
        }
        AttributeRule::ItemName { value, inverted } => {
            let matched = match items::component_tag(host_ref, stack, "minecraft:item_name") {
                Some(Tag::String(name)) => name.to_string_lossy() == *value,
                _ => value.is_empty(),
            };
            matched ^ *inverted
        }
        AttributeRule::Unknown => false,
    }
}

fn eval_standard(host_ref: &dyn Host, kind: &StandardKind, stack: &ItemStack) -> bool {
    let key = stack.item();
    match kind {
        StandardKind::Placeable => host_ref.item_block(key).is_some(),
        StandardKind::Consumable => {
            items::component_tag(host_ref, stack, "minecraft:food").is_some()
                || items::component_tag(host_ref, stack, "minecraft:consumable").is_some()
        }
        StandardKind::FluidContainer => {
            let id = key.id(host_ref);
            matches!(
                id.as_str(),
                "minecraft:bucket"
                    | "minecraft:water_bucket"
                    | "minecraft:lava_bucket"
                    | "minecraft:milk_bucket"
                    | "theurgy:sal_ammoniac_bucket"
            ) || stack_fluid_id(host_ref, stack).is_some()
        }
        StandardKind::Enchanted => matches!(
            items::component_tag(host_ref, stack, "minecraft:enchantments"),
            Some(Tag::Compound(levels)) if !levels.is_empty()
        ),
        StandardKind::Renamed => {
            stack
                .components()
                .is_some_and(|c| c.as_nbt().contains_key(&"minecraft:custom_name".into()))
        }
        StandardKind::Damaged => stack.damage() > 0,
        StandardKind::BadlyDamaged => host_ref
            .max_damage(key)
            .is_some_and(|max| max > 0 && (f64::from(stack.damage()) / f64::from(max)) > 0.75),
        StandardKind::NotStackable => host_ref.max_stack_size(key) == Some(1),
        StandardKind::Equipable => {
            items::component_tag(host_ref, stack, "minecraft:equippable").is_some()
        }
        StandardKind::FurnaceFuel => host_ref.item_burn_time(key).unwrap_or(0) > 0,
        StandardKind::Smeltable => recipes::matches_cooking(host_ref, "minecraft:smelting", stack),
        StandardKind::Smokable => recipes::matches_cooking(host_ref, "minecraft:smoking", stack),
        StandardKind::Blastable => recipes::matches_cooking(host_ref, "minecraft:blasting", stack),
        StandardKind::Compostable => host_ref.item_compost_chance(key).is_some(),
    }
}

/// Combined filter predicate from an optional saved filter item.
pub fn allows_item(host_ref: &dyn Host, filter: Option<&ItemStack>, stack: &ItemStack) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let list = ListFilter::from_stack(host_ref, Some(filter));
    let attr = AttributeFilter::from_stack(host_ref, Some(filter));
    list.allows_item(stack) && attr.allows_item(host_ref, stack)
}

pub fn allows_fluid(host_ref: &dyn Host, filter: Option<&ItemStack>, fluid_id: &str) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    ListFilter::from_stack(host_ref, Some(filter)).allows_fluid(host_ref, fluid_id)
}

// ---------------------------------------------------------------------------
// Stack <-> NBT template serialization (FilterBehaviour.writeNetwork)
// ---------------------------------------------------------------------------

const FILTER_SAVED_COMPONENTS: [&str; 12] = [
    "minecraft:custom_name",
    "minecraft:enchantments",
    "minecraft:custom_data",
    "theurgy:filter_items",
    "theurgy:filter_is_deny_list",
    "theurgy:filter_respects_data_components",
    "theurgy:filter_mode",
    "theurgy:filter_attributes",
    LIST_CONTENTS,
    LIST_CONFIG,
    ATTRIBUTE_REFERENCE,
    ATTRIBUTE_CONFIG,
];

pub fn stack_to_template(host_ref: &dyn Host, stack: &ItemStack) -> Tag {
    let mut record = Compound::new();
    let id = stack.item().id(host_ref);
    record.insert("id".into(), Tag::String(id.as_str().into()));
    if stack.count() != 1 {
        record.insert("count".into(), Tag::Int(stack.count() as i32));
    }
    let mut components = Compound::new();
    if stack.damage() != 0 {
        components.insert("minecraft:damage".into(), Tag::Int(stack.damage() as i32));
    }
    if let Some(patch) = stack.components().map(|c| c.as_nbt()) {
        for key in FILTER_SAVED_COMPONENTS {
            if let Some(tag) = patch.get(&key.into()) {
                components.insert(key.into(), tag.clone());
            }
        }
    }
    if !components.is_empty() {
        record.insert("components".into(), Tag::Compound(components));
    }
    Tag::Compound(record)
}

pub fn stack_from_template(host_ref: &dyn Host, tag: &Tag) -> Option<ItemStack> {
    let Tag::Compound(record) = tag else {
        return None;
    };
    let id = host::string(record, "id")?;
    if id.is_empty() {
        return None;
    }
    let count = host::int(record, "count", 1);
    if count <= 0 {
        return None;
    }
    let mut stack = host::make_stack(host_ref, &id, count as u32)?;
    if let Some(Tag::Compound(components)) = record.get(&"components".into()) {
        if let Some(Tag::Int(dmg)) = components.get(&"minecraft:damage".into())
            && *dmg > 0
        {
            let _ = stack.set_damage(*dmg as u32);
        }
        let mut patch = Compound::new();
        for (k, v) in components {
            if k.to_string_lossy() != "minecraft:damage" {
                patch.insert(k.clone(), v.clone());
            }
        }
        if !patch.is_empty() {
            let _ = stack.apply_components(&patch);
        }
    }
    Some(stack)
}

// ---------------------------------------------------------------------------
// Server-authoritative filter editing (`/theurgy_filter` and client packets)
// ---------------------------------------------------------------------------

pub fn json_tag(value: &Value) -> Option<Tag> {
    Some(match value {
        Value::Null => Tag::End,
        Value::Bool(v) => Tag::Byte(i8::from(*v)),
        Value::Number(v) => {
            if let Some(n) = v.as_i64().and_then(|n| i32::try_from(n).ok()) {
                Tag::Int(n)
            } else {
                Tag::Double(v.as_f64()?.to_bits())
            }
        }
        Value::String(v) => Tag::String(v.as_str().into()),
        Value::Array(vs) => Tag::List(vs.iter().map(json_tag).collect::<Option<_>>()?),
        Value::Object(vs) => Tag::Compound(
            vs.iter()
                .map(|(k, v)| Some((k.as_str().into(), json_tag(v)?)))
                .collect::<Option<_>>()?,
        ),
    })
}

fn compound(tag: Option<Tag>) -> Compound {
    match tag {
        Some(Tag::Compound(map)) => map,
        _ => Compound::new(),
    }
}

fn list(tag: Option<&Tag>) -> Vec<Tag> {
    match tag {
        Some(Tag::List(values)) => values.clone(),
        _ => Vec::new(),
    }
}

fn write_filter(
    host_ref: &mut dyn ServerHost,
    player_id: u128,
    hand: Hand,
    stack: ItemStack,
) -> bool {
    if stack.item().id(&*host_ref) == "theurgy:attribute_filter"
        && !AttributeFilter::from_stack(&*host_ref, Some(&stack)).is_valid()
    {
        return false;
    }
    let Some(player) = host_ref.player(player_id) else {
        return false;
    };
    let slot = match hand {
        Hand::Main => player.selected_slot(&*host_ref).unwrap_or(0) as u32,
        Hand::Off => 40,
    };
    host_ref
        .transaction(|host_ref, _tx| {
            player
                .set_inventory_slot(host_ref, slot, Some(stack))
                .map_err(|_| ())
        })
        .is_ok()
}

pub fn apply_action(
    host_ref: &mut dyn ServerHost,
    player_id: u128,
    hand: Hand,
    action: &str,
    rest: &str,
) -> bool {
    let place = ItemPlace::PlayerHand {
        player: player_id,
        hand,
    };
    let Some(filter) = place.stack(host_ref) else {
        return false;
    };
    let item_id = filter.item().id(&*host_ref);
    match item_id.as_str() {
        "theurgy:list_filter" => list_action(host_ref, player_id, hand, filter, action, rest),
        "theurgy:attribute_filter" => {
            attribute_action(host_ref, player_id, hand, filter, action, rest)
        }
        _ => false,
    }
}

fn list_action(
    host_ref: &mut dyn ServerHost,
    player_id: u128,
    hand: Hand,
    mut filter: ItemStack,
    action: &str,
    rest: &str,
) -> bool {
    let mut cfg = compound(items::component_tag(&*host_ref, &filter, LIST_CONFIG));
    let mut contents = list(items::component_tag(&*host_ref, &filter, LIST_CONTENTS).as_ref());
    let ok = match action {
        "mode" if matches!(rest, "allow" | "deny") => {
            cfg.insert("mode".into(), Tag::String(rest.into()));
            true
        }
        "components" if matches!(rest, "true" | "false") => {
            cfg.insert(
                "respect_data_components".into(),
                Tag::Byte(i8::from(rest == "true")),
            );
            true
        }
        "clear" => {
            contents.clear();
            true
        }
        "add" => {
            if contents.len() >= 27 {
                return false;
            }
            let Some(player) = host_ref.player(player_id) else {
                return false;
            };
            let inv = player.inventory_slots(&*host_ref).unwrap_or_default();
            let chosen = if rest == "auto" {
                inv.into_iter().find_map(|(_, st)| {
                    let id = st.item().id(&*host_ref);
                    (st.count() > 0
                        && !matches!(
                            id.as_str(),
                            "theurgy:list_filter" | "theurgy:attribute_filter"
                        ))
                    .then_some(st)
                })
            } else {
                let wanted = rest.parse::<u32>().ok();
                wanted.and_then(|s| inv.into_iter().find(|(idx, _)| *idx == s).map(|(_, st)| st))
            };
            let Some(mut source) = chosen else {
                return false;
            };
            let src_id = source.item().id(&*host_ref);
            if source.count() == 0
                || matches!(
                    src_id.as_str(),
                    "theurgy:list_filter" | "theurgy:attribute_filter"
                )
            {
                return false;
            }
            let _ = source.set_count(1);
            contents.push(stack_to_template(&*host_ref, &source));
            true
        }
        "remove" => {
            let Ok(index) = rest.parse::<usize>() else {
                return false;
            };
            if index < contents.len() {
                contents.remove(index);
                true
            } else {
                false
            }
        }
        _ => false,
    };
    if !ok {
        return false;
    }
    items::set_stack_component(&mut filter, LIST_CONFIG, Tag::Compound(cfg));
    items::set_stack_component(&mut filter, LIST_CONTENTS, Tag::List(contents));
    write_filter(host_ref, player_id, hand, filter)
}

fn attribute_action(
    host_ref: &mut dyn ServerHost,
    player_id: u128,
    hand: Hand,
    mut filter: ItemStack,
    action: &str,
    rest: &str,
) -> bool {
    let mut cfg = compound(items::component_tag(&*host_ref, &filter, ATTRIBUTE_CONFIG));
    let mut rules = list(cfg.get(&"rules".into()));
    let ok = match action {
        "mode" if matches!(rest, "match_any" | "match_all" | "deny") => {
            cfg.insert("mode".into(), Tag::String(rest.into()));
            true
        }
        "clear" => {
            rules.clear();
            true
        }
        "rule_renamed" => {
            let mut payload = Compound::new();
            payload.insert("value".into(), Tag::String("renamed".into()));
            let mut rule = Compound::new();
            rule.insert("type".into(), Tag::String("codedefinedgui:standard".into()));
            rule.insert("payload".into(), Tag::Compound(payload));
            rule.insert("inverted".into(), Tag::Byte(0));
            rules.push(Tag::Compound(rule));
            true
        }
        "rule_tag" => {
            if rest.is_empty() || rest == "auto" {
                return false;
            }
            let mut payload = Compound::new();
            payload.insert("id".into(), Tag::String(rest.into()));
            let mut rule = Compound::new();
            rule.insert("type".into(), Tag::String("codedefinedgui:in_tag".into()));
            rule.insert("payload".into(), Tag::Compound(payload));
            rule.insert("inverted".into(), Tag::Byte(0));
            rules.push(Tag::Compound(rule));
            true
        }
        "rule_add" => {
            let Some((kind, json)) = rest.split_once(' ') else {
                return false;
            };
            if !kind.starts_with("codedefinedgui:") {
                return false;
            }
            let Ok(value) = mod_api::serde_json::from_str::<Value>(json) else {
                return false;
            };
            if !value.is_object() {
                return false;
            }
            let Some(Tag::Compound(payload)) = json_tag(&value) else {
                return false;
            };
            let mut rule = Compound::new();
            rule.insert("type".into(), Tag::String(kind.into()));
            rule.insert("payload".into(), Tag::Compound(payload));
            rule.insert("inverted".into(), Tag::Byte(0));
            rules.push(Tag::Compound(rule));
            true
        }
        "rule_invert" => {
            let Ok(index) = rest.parse::<usize>() else {
                return false;
            };
            let Some(Tag::Compound(rule)) = rules.get_mut(index) else {
                return false;
            };
            let inv = host::bool(rule, "inverted");
            rule.insert("inverted".into(), Tag::Byte(i8::from(!inv)));
            true
        }
        "rule_remove" => {
            let Ok(index) = rest.parse::<usize>() else {
                return false;
            };
            if index < rules.len() {
                rules.remove(index);
                true
            } else {
                false
            }
        }
        _ => false,
    };
    if !ok {
        return false;
    }
    cfg.insert("rules".into(), Tag::List(rules));
    items::set_stack_component(&mut filter, ATTRIBUTE_CONFIG, Tag::Compound(cfg));
    write_filter(host_ref, player_id, hand, filter)
}

pub fn register(commands: &mut Vec<Command>) {
    commands.push(Command::new(
        "theurgy_filter <hand:word> <action:word> [args:text]",
    ));
}

pub fn run(host_ref: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
    if event.cancelled
        || event.command.literals.first().map(String::as_str) != Some("theurgy_filter")
    {
        return Flow::Continue;
    }
    let hand_str = event
        .command
        .argument("hand")
        .and_then(|v| match v {
            CommandArgumentValue::String(s) => Some(s.as_str()),
            _ => None,
        })
        .unwrap_or("");
    let hand = match hand_str {
        "main" => Hand::Main,
        "off" => Hand::Off,
        _ => {
            let _ = host_ref
                .commands()
                .feedback(Text::literal("theurgy_filter: hand must be main or off"));
            return Flow::Cancel;
        }
    };
    let action = event
        .command
        .argument("action")
        .and_then(|v| match v {
            CommandArgumentValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let rest = event
        .command
        .argument("args")
        .and_then(|v| match v {
            CommandArgumentValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let Some(player) = host_ref.players().acting_player() else {
        return Flow::Cancel;
    };
    let Some(player_id) = player.entity().uuid(host_ref) else {
        return Flow::Cancel;
    };
    if !apply_action(host_ref, player_id, hand, &action, &rest) {
        let _ = host_ref.commands().feedback(Text::literal(
            "theurgy_filter: rejected action or invalid resulting configuration",
        ));
    }
    Flow::Cancel
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::serde_json::json;

    #[test]
    fn json_payloads_become_bounded_nbt() {
        let value = json!({"value": "renamed", "nested": [1, true]});
        let tag = json_tag(&value).expect("tag");
        let Tag::Compound(map) = tag else {
            panic!("compound expected");
        };
        assert!(matches!(map.get(&"value".into()), Some(Tag::String(_))));
        assert!(matches!(map.get(&"nested".into()), Some(Tag::List(_))));
    }

    #[test]
    fn standard_attribute_names_parse_case_insensitively() {
        for name in [
            "placeable",
            "CONSUMABLE",
            "fluid_container",
            "enchanted",
            "renamed",
            "damaged",
            "badly_damaged",
            "not_stackable",
            "equipable",
            "furnace_fuel",
            "smeltable",
            "smokable",
            "blastable",
            "compostable",
        ] {
            assert!(StandardKind::parse(name).is_some(), "{name}");
        }
        assert!(StandardKind::parse("no_such_rule").is_none());
    }

    #[test]
    fn attribute_filter_config_validates_known_and_rejects_unknown_rules() {
        let mut cfg = Compound::new();
        cfg.insert("mode".into(), Tag::String("match_all".into()));
        let mut r1 = Compound::new();
        r1.insert("type".into(), Tag::String("codedefinedgui:standard".into()));
        let mut p1 = Compound::new();
        p1.insert("value".into(), Tag::String("renamed".into()));
        r1.insert("payload".into(), Tag::Compound(p1));
        r1.insert("inverted".into(), Tag::Byte(1));
        cfg.insert("rules".into(), Tag::List(vec![Tag::Compound(r1)]));

        let parsed = AttributeFilter::from_config_nbt(Some(&cfg));
        assert_eq!(parsed.mode, AttributeMode::MatchAll);
        assert!(parsed.is_valid());
        assert_eq!(
            parsed.rules,
            vec![AttributeRule::Standard {
                kind: StandardKind::Renamed,
                inverted: true,
            }]
        );

        let mut bad_rule = Compound::new();
        bad_rule.insert("type".into(), Tag::String("codedefinedgui:bogus".into()));
        bad_rule.insert("payload".into(), Tag::Compound(Compound::new()));
        cfg.insert("rules".into(), Tag::List(vec![Tag::Compound(bad_rule)]));
        assert!(!AttributeFilter::from_config_nbt(Some(&cfg)).is_valid());
    }
}
