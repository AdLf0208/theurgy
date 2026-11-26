// SPDX-License-Identifier: MIT
//! `/theurgy info|recipe|recipes` readout commands backed by live world state
//! and the reload-stable recipe cache.

use mod_api::{
    Command, CommandArgument, CommandArgumentValue, CommandSource, Compound, Flow, Host,
    ItemStack, LevelRef, RunModCommandEvent, ServerHost, Text,
};

use crate::{host, recipes, registry};

/// Permission level matching vanilla ops (`LEVEL_ADMINS`).
pub const LEVEL_ADMINS: u8 = 2;
/// Reach in blocks within which a non-operator player may inspect an apparatus.
pub const INSPECT_RANGE: f64 = 8.0;

pub fn declare(commands: &mut Vec<Command>) {
    commands.push(Command::new("theurgy info <pos:block_pos>"));
    commands.push(Command::new("theurgy recipe <item:item>"));
    commands.push(Command::new("theurgy recipes"));
}

pub fn may_inspect(is_operator: bool, same_level: bool, distance: f64) -> bool {
    is_operator || (same_level && distance <= INSPECT_RANGE)
}

fn is_theurgy(literals: &[String]) -> bool {
    literals.first().is_some_and(|word| word == "theurgy")
}

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

fn may_read(host_ref: &dyn ServerHost, source: &CommandSource, level: LevelRef, pos: [i32; 3]) -> bool {
    let op = source.permission().level() >= LEVEL_ADMINS;
    match source {
        CommandSource::Player { player, .. } => {
            let same_level = player.level(host_ref) == Some(level);
            let distance = player
                .entity()
                .position(host_ref)
                .map(|at| {
                    let block = [
                        f64::from(pos[0]) + 0.5,
                        f64::from(pos[1]) + 0.5,
                        f64::from(pos[2]) + 0.5,
                    ];
                    (0..3)
                        .map(|axis| (at[axis] - block[axis]).powi(2))
                        .sum::<f64>()
                        .sqrt()
                })
                .unwrap_or(f64::MAX);
            may_inspect(op, same_level, distance)
        }
        _ => op,
    }
}

fn slot_text(host_ref: &dyn Host, stack: Option<&ItemStack>) -> String {
    match stack {
        Some(stack) => format!("{} x{}", stack.item().id(host_ref), stack.count()),
        None => "empty".to_owned(),
    }
}

fn feedback(host_ref: &mut dyn ServerHost, line: impl Into<String>) {
    let _ = host_ref.commands().feedback(Text::literal(line.into()));
}

fn readout(host_ref: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3]) -> String {
    let Some(entity) = host_ref.block_entity(level, pos) else {
        return format!("{pos:?} is not a mod block entity.");
    };
    let kind = entity.kind(&*host_ref).map(|k| k.id(&*host_ref));
    let Some(id) = kind.as_deref() else {
        return format!("{pos:?} is not a mod block entity.");
    };
    if !registry::is_be(id) {
        return format!("{pos:?} is a {id}, not a Theurgy apparatus.");
    }
    let data = entity.data(&*host_ref).unwrap_or_else(Compound::new);
    let slots = entity.data_slots(&*host_ref).unwrap_or_default();
    let now = level.game_time(&*host_ref).unwrap_or(0);
    let lit = level
        .block_property(&*host_ref, pos, "lit")
        .unwrap_or_else(|| "-".to_owned());

    match id {
        "theurgy:pyromantic_brazier" => {
            let fuel = entity.item_slot(&*host_ref, 0).ok().flatten();
            format!(
                "Pyromantic Brazier at {pos:?}: lit={lit}, fuel {}, remaining {} ticks, t={now}",
                slot_text(&*host_ref, fuel.as_ref()),
                host::int(&data, "remainingLitTime", 0),
            )
        }
        "theurgy:calcination_oven"
        | "theurgy:distiller"
        | "theurgy:liquefaction_cauldron" => {
            let input = entity.item_slot(&*host_ref, 0).ok().flatten();
            let output = entity.item_slot(&*host_ref, 1).ok().flatten();
            let heat_cache = slots.get(6).copied().unwrap_or(0);
            format!(
                "{id} at {pos:?}: lit={lit}, input {}, output {}, progress {}/{}, heatCache {heat_cache}, t={now}",
                slot_text(&*host_ref, input.as_ref()),
                slot_text(&*host_ref, output.as_ref()),
                host::int(&data, "progress", 0),
                host::int(&data, "totalTime", 0),
            )
        }
        "theurgy:fermentation_vat" | "theurgy:digestion_vat" => {
            let open = level
                .block_property(&*host_ref, pos, "open")
                .unwrap_or_else(|| "?".to_owned());
            let output = entity.item_slot(&*host_ref, 3).ok().flatten();
            let tank = host::tank_from_nbt(&data, 0, 10_000);
            format!(
                "{id} at {pos:?}: open={open}, output {}, fluid {:?} {}mB, progress {}/{}, t={now}",
                slot_text(&*host_ref, output.as_ref()),
                tank.fluid,
                tank.amount,
                host::int(&data, "progress", 0),
                host::int(&data, "totalTime", 0),
            )
        }
        _ => {
            let energy = host::int(&data, "mercuryFlux", 0);
            format!("{id} at {pos:?}: progress {}/{}, flux {energy}, t={now}",
                host::int(&data, "progress", 0),
                host::int(&data, "totalTime", 0),
            )
        }
    }
}

pub fn run(host_ref: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
    if event.cancelled || !is_theurgy(&event.command.literals) {
        return Flow::Continue;
    }
    match event.command.literals.get(1).map(String::as_str) {
        Some("info") => {
            let Some(pos) = info_position(&event.command.arguments) else {
                feedback(host_ref, "usage: /theurgy info <x> <y> <z>");
                return Flow::Cancel;
            };
            let level = event
                .command
                .dimension
                .id(&*host_ref)
                .and_then(|id| host_ref.level(&id));
            if let Some(level) = level
                && !may_read(&*host_ref, event.source, level, pos)
            {
                feedback(
                    host_ref,
                    format!(
                        "You must be an operator or within {:.0} blocks of the apparatus.",
                        INSPECT_RANGE
                    ),
                );
                return Flow::Cancel;
            }
            let line = match level {
                Some(level) => readout(host_ref, level, pos),
                None => format!("{pos:?} is not a mod block entity."),
            };
            feedback(host_ref, line);
            Flow::Cancel
        }
        Some("recipe") => {
            let Some(CommandArgumentValue::Item(stack)) = event.command.argument("item").cloned()
            else {
                feedback(host_ref, "usage: /theurgy recipe <item>");
                return Flow::Cancel;
            };
            let item = stack.item().id(&*host_ref);
            let calc = recipes::of_type("theurgy:calcination");
            let found = calc.iter().find(|r| {
                r.inputs
                    .first()
                    .is_some_and(|ing| recipes::matches_cached(&*host_ref, ing, &stack))
            });
            let line = match found {
                Some(r) => {
                    let count = r.inputs.first().map_or(1, |i| i.count);
                    let out = r
                        .result
                        .as_ref()
                        .map(|s| format!("{} x{}", s.item().id(&*host_ref), s.count()))
                        .unwrap_or_else(|| "?".to_owned());
                    format!("{item} calcines to {out} for {count} of it in {} ticks.", r.time)
                }
                None => format!("{item} has no calcination recipe."),
            };
            feedback(host_ref, line);
            Flow::Cancel
        }
        Some("recipes") => {
            let mut parts = Vec::new();
            let mut total = 0usize;
            for name in recipes::RECIPE_TYPES {
                let kind = format!("theurgy:{name}");
                let n = recipes::of_type(&kind).len();
                total += n;
                parts.push(format!("{name}={n}"));
            }
            feedback(
                host_ref,
                format!("{total} Theurgy recipes loaded ({}).", parts.join(", ")),
            );
            Flow::Cancel
        }
        _ => {
            feedback(host_ref, "usage: /theurgy info|recipe|recipes ...");
            Flow::Cancel
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readout_needs_operator_or_reach() {
        assert!(may_inspect(true, false, 1000.0));
        assert!(may_inspect(false, true, 8.0));
        assert!(!may_inspect(false, true, 8.1));
        assert!(!may_inspect(false, false, 1.0));
    }

    #[test]
    fn only_the_theurgy_root_is_ours() {
        let words = |ws: &[&str]| ws.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        assert!(is_theurgy(&words(&["theurgy", "info"])));
        assert!(!is_theurgy(&words(&["other", "info"])));
        assert!(!is_theurgy(&[]));
    }

    #[test]
    fn info_position_is_one_block_pos() {
        let given = [CommandArgument::new(
            "pos".to_owned(),
            CommandArgumentValue::BlockPos([200, 102, 0]),
        )];
        assert_eq!(info_position(&given), Some([200, 102, 0]));
        let wrong = [CommandArgument::new(
            "pos".to_owned(),
            CommandArgumentValue::Integer(1),
        )];
        assert_eq!(info_position(&wrong), None);
    }
}
