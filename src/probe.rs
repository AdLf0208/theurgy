// SPDX-License-Identifier: MIT
//! Operator-only structured block-entity probe commands used by the
//! Java↔Rust differential harness and e2e checks.

use mod_api::serde_json::{self, Value};
use mod_api::{
    Command, CommandArgumentValue, Flow, Hand, Host, ItemPlace, LevelRef, RunModCommandEvent,
    ServerHost, Tag, Text,
};

use crate::{host, interactions, items, logistics};

pub fn tag_json(tag: &Tag) -> Value {
    match tag {
        Tag::End => Value::Null,
        Tag::Byte(v) => serde_json::json!(*v),
        Tag::Short(v) => serde_json::json!(*v),
        Tag::Int(v) => serde_json::json!(*v),
        Tag::Long(v) => serde_json::json!(*v),
        Tag::Float(bits) => serde_json::Number::from_f64(f64::from(f32::from_bits(*bits)))
            .map_or(Value::Null, Value::Number),
        Tag::Double(bits) => serde_json::Number::from_f64(f64::from_bits(*bits))
            .map_or(Value::Null, Value::Number),
        Tag::ByteArray(v) => serde_json::json!(v),
        Tag::String(v) => serde_json::json!(v.to_string_lossy()),
        Tag::List(v) => Value::Array(v.iter().map(tag_json).collect()),
        Tag::Compound(v) => Value::Object(
            v.iter()
                .map(|(k, val)| (k.to_string_lossy(), tag_json(val)))
                .collect(),
        ),
        Tag::IntArray(v) => serde_json::json!(v),
        Tag::LongArray(v) => serde_json::json!(v),
    }
}

pub fn register(commands: &mut Vec<Command>) {
    commands.push(Command::new("theurgy_probe <pos:block_pos>"));
    commands.push(Command::new(
        "theurgy_state <pos:block_pos> <key:word> <value:word>",
    ));
    commands.push(Command::new("theurgy_interact <pos:block_pos> <face:word>"));
    commands.push(Command::new("theurgy_proxy_read <pos:block_pos>"));
    commands.push(Command::new(
        "theurgy_proxy_drain <pos:block_pos> <amount:integer>",
    ));
}

fn feedback(host_ref: &mut dyn ServerHost, text: String) {
    let _ = host_ref.commands().feedback(Text::literal(text));
}

fn probe_state(host_ref: &dyn Host, level: LevelRef, dim: &str, pos: [i32; 3]) -> Value {
    let state = level.block_state(host_ref, pos);
    let serialized = state
        .and_then(|s| host_ref.block_state_string(s))
        .unwrap_or_else(|| "minecraft:air".to_owned());
    let block = serialized
        .split('[')
        .next()
        .unwrap_or("minecraft:air")
        .to_owned();
    let mut properties = serde_json::Map::new();
    if let (Some(start), Some(end)) = (serialized.find('['), serialized.rfind(']'))
        && end > start
    {
        for pair in serialized[start + 1..end].split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                properties.insert(k.to_owned(), serde_json::json!(v));
            }
        }
    }
    let (nbt, items, data) = if let Some(be) = host_ref.block_entity(level, pos) {
        let nbt = be
            .data(host_ref)
            .unwrap_or_default()
            .iter()
            .map(|(k, v)| (k.to_string_lossy(), tag_json(v)))
            .collect::<serde_json::Map<_, _>>();
        let mut items = Vec::new();
        for slot in 0..8u32 {
            if let Ok(Some(stack)) = be.item_slot(host_ref, slot) {
                items.push(serde_json::json!({
                    "Slot": slot,
                    "id": stack.item().id(host_ref),
                    "count": stack.count(),
                    "damage": stack.damage(),
                }));
            }
        }
        let data = be.data_slots(host_ref).unwrap_or_default();
        (nbt, items, data)
    } else {
        (serde_json::Map::new(), Vec::new(), Vec::new())
    };
    let mercury_flux_capacity = match block.as_str() {
        "theurgy:mercury_catalyst" => Some(50_000),
        "theurgy:mercury_capacitor" => Some(500_000),
        "theurgy:caloric_flux_emitter"
        | "theurgy:logistics_mercury_flux_connector"
        | "theurgy:mercury_flux_emitter"
        | "theurgy:sulfuric_flux_emitter" => Some(1_000),
        _ => None,
    };
    serde_json::json!({
        "pos": pos,
        "block": block,
        "properties": properties,
        "nbt": nbt,
        "items": items,
        "data": data,
        "capabilities": {
            "mercury_flux_capacity": mercury_flux_capacity,
            "insert_targets": logistics::probe_insert_targets(host_ref, level, dim, pos),
            "nexus_partners": logistics::probe_nexus_partners(host_ref, level, dim, pos),
        },
    })
}

pub fn run(host_ref: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
    if event.cancelled {
        return Flow::Continue;
    }
    let Some(root) = event.command.literals.first().map(String::as_str) else {
        return Flow::Continue;
    };
    if !matches!(
        root,
        "theurgy_probe"
            | "theurgy_state"
            | "theurgy_interact"
            | "theurgy_proxy_read"
            | "theurgy_proxy_drain"
    ) {
        return Flow::Continue;
    }
    if event.source.permission().level() < 4 {
        feedback(
            host_ref,
            r#"{"error":"operator level 4 required"}"#.to_owned(),
        );
        return Flow::Cancel;
    }
    let dim = event
        .command
        .dimension
        .id(&*host_ref)
        .unwrap_or_else(|| host_ref.dimension().to_owned());
    let level = host_ref.level(&dim).or_else(|| host::level_in_place(host_ref));
    let Some(level) = level else {
        feedback(host_ref, r#"{"error":"no level"}"#.to_owned());
        return Flow::Cancel;
    };
    let pos = event
        .command
        .argument("pos")
        .and_then(|v| match v {
            CommandArgumentValue::BlockPos(p) => Some(*p),
            _ => None,
        });
    let Some(pos) = pos else {
        feedback(host_ref, r#"{"error":"missing pos"}"#.to_owned());
        return Flow::Cancel;
    };
    match root {
        "theurgy_probe" => {
            let reply = probe_state(&*host_ref, level, &dim, pos);
            feedback(host_ref, reply.to_string());
            Flow::Cancel
        }
        "theurgy_state" => {
            let key = event.command.argument("key").and_then(|v| match v {
                CommandArgumentValue::String(w) => Some(w.clone()),
                _ => None,
            });
            let value = event.command.argument("value").and_then(|v| match v {
                CommandArgumentValue::String(w) => Some(w.clone()),
                _ => None,
            });
            if let (Some(key), Some(value)) = (key, value) {
                let changed = host_ref.set_block_property_with(pos, &key, &value, 3);
                feedback(
                    host_ref,
                    serde_json::json!({"changed": changed}).to_string(),
                );
            } else {
                feedback(
                    host_ref,
                    r#"{"error":"usage: theurgy_state <x y z> <key> <value>"}"#.to_owned(),
                );
            }
            Flow::Cancel
        }
        "theurgy_interact" => {
            let face_name = event.command.argument("face").and_then(|v| match v {
                CommandArgumentValue::String(w) => Some(w.clone()),
                _ => None,
            });
            let face = match face_name.as_deref() {
                Some("down") => [0, -1, 0],
                Some("up") => [0, 1, 0],
                Some("north") => [0, 0, -1],
                Some("south") => [0, 0, 1],
                Some("west") => [-1, 0, 0],
                Some("east") => [1, 0, 0],
                _ => {
                    feedback(host_ref, r#"{"error":"invalid face"}"#.to_owned());
                    return Flow::Cancel;
                }
            };
            let Some(player) = host_ref.players().acting_player() else {
                feedback(host_ref, r#"{"error":"player required"}"#.to_owned());
                return Flow::Cancel;
            };
            let Some(player_uuid) = player.entity().uuid(host_ref) else {
                feedback(host_ref, r#"{"error":"player uuid missing"}"#.to_owned());
                return Flow::Cancel;
            };
            let held = ItemPlace::PlayerHand {
                player: player_uuid,
                hand: Hand::Main,
            }
            .stack(host_ref)
            .filter(|s| s.count() > 0);
            let mut canceled = items::right_click_block_direct(
                host_ref,
                player_uuid,
                Hand::Main,
                level,
                &dim,
                pos,
                face,
                held.as_ref(),
            );
            if !canceled {
                let res = if held.is_some() {
                    interactions::interact_with_item(
                        host_ref,
                        level,
                        player_uuid,
                        Hand::Main,
                        pos,
                        face,
                        held.as_ref(),
                    )
                } else {
                    interactions::interact_empty_hand(host_ref, level, player_uuid, pos)
                };
                canceled = res.is_some();
            }
            let state = probe_state(&*host_ref, level, &dim, pos);
            feedback(
                host_ref,
                serde_json::json!({"ok": true, "canceled": canceled, "state": state}).to_string(),
            );
            Flow::Cancel
        }
        "theurgy_proxy_read" => {
            let result = logistics::proxy_read_fluid(host_ref, level, &dim, pos);
            feedback(
                host_ref,
                serde_json::json!({"fluid": result}).to_string(),
            );
            Flow::Cancel
        }
        "theurgy_proxy_drain" => {
            let amount = event
                .command
                .argument("amount")
                .and_then(|v| match v {
                    CommandArgumentValue::Integer(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(0);
            let drained = logistics::proxy_drain_fluid(host_ref, level, &dim, pos, amount);
            feedback(
                host_ref,
                serde_json::json!({"drained": drained}).to_string(),
            );
            Flow::Cancel
        }
        _ => Flow::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::Compound;

    #[test]
    fn nbt_projection_preserves_numbers_lists_and_compounds() {
        let mut compound = Compound::new();
        compound.insert("progress".into(), Tag::Short(17));
        compound.insert(
            "Items".into(),
            Tag::List(vec![Tag::String("minecraft:apple".into())]),
        );
        let value = tag_json(&Tag::Compound(compound));
        assert_eq!(value["progress"], 17);
        assert_eq!(value["Items"][0], "minecraft:apple");
    }
}
