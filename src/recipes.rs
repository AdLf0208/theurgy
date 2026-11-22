// SPDX-License-Identifier: MIT
//! Reload-stable server recipe cache for all nine Theurgy custom recipe types.
//!
//! Populated on `tags_updated` (server side) and cleared on `server_about_to_start`
//! and `server_stopped`, following `docs/modding/src/how-to/items/reload-stable-recipes.md`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use mod_api::serde_json::Value;
use mod_api::{Host, Ingredient, ItemStack};

use crate::engine::recipes::{fluid_part, ingredient_parts};

/// All custom recipe types declared by Theurgy (bare paths for `Registrar::register_recipe_type`).
pub const RECIPE_TYPES: &[&str] = &[
    "accumulation",
    "calcination",
    "catalysation",
    "digestion",
    "distillation",
    "fermentation",
    "incubation",
    "liquefaction",
    "reformation",
];

#[derive(Clone, Debug)]
pub struct CachedIngredient {
    pub raw: Value,
    pub parsed: Option<Ingredient>,
    pub count: u32,
}

#[derive(Clone, Debug)]
pub struct CachedRecipe {
    pub id: String,
    pub json: Arc<Value>,
    pub time: i32,
    pub inputs: Vec<CachedIngredient>,
    pub fluid: Option<(String, i32)>,
    pub result: Option<ItemStack>,
}

#[derive(Default)]
struct Cache {
    by_type: HashMap<&'static str, Arc<[CachedRecipe]>>,
}

static CACHE: OnceLock<RwLock<Cache>> = OnceLock::new();

fn cache() -> &'static RwLock<Cache> {
    CACHE.get_or_init(|| RwLock::new(Cache::default()))
}

pub fn clear() {
    if let Ok(mut guard) = cache().write() {
        guard.by_type.clear();
    }
}

fn recipe_time(json: &Value, kind: &str) -> i32 {
    let default = if matches!(kind, "theurgy:fermentation" | "theurgy:digestion") {
        200
    } else {
        100
    };
    json.get("time")
        .and_then(Value::as_i64)
        .unwrap_or(default)
        .clamp(0, i64::from(i32::MAX)) as i32
}

pub fn parse_ingredient(host: &dyn Host, value: &Value) -> Option<Ingredient> {
    let text = value.to_string();
    host.recipe_ingredient(&text).ok().flatten()
}

pub fn parse_result(host: &dyn Host, value: &Value) -> Option<ItemStack> {
    let text = value.to_string();
    host.recipe_result(&text).ok().flatten()
}

/// Rebuild the whole recipe cache from the server's published recipes and tags.
pub fn publish(host: &dyn Host) {
    let kinds: [&'static str; 9] = [
        "theurgy:accumulation",
        "theurgy:calcination",
        "theurgy:catalysation",
        "theurgy:digestion",
        "theurgy:distillation",
        "theurgy:fermentation",
        "theurgy:incubation",
        "theurgy:liquefaction",
        "theurgy:reformation",
    ];
    let mut next = HashMap::with_capacity(kinds.len());
    for kind in kinds {
        let rows: Vec<CachedRecipe> = host
            .recipes_of_type(kind)
            .iter()
            .map(|recipe| {
                let inputs = ingredient_parts(kind, &recipe.json)
                    .into_iter()
                    .map(|(raw, count)| CachedIngredient {
                        parsed: parse_ingredient(host, raw),
                        raw: raw.clone(),
                        count,
                    })
                    .collect();
                let fluid = fluid_part(kind, &recipe.json)
                    .map(|(fluid_id, amount)| (fluid_id.to_owned(), amount));
                let result = recipe
                    .json
                    .get("result")
                    .and_then(|value| parse_result(host, value));
                CachedRecipe {
                    id: recipe.id.clone(),
                    json: Arc::clone(&recipe.json),
                    time: recipe_time(&recipe.json, kind),
                    inputs,
                    fluid,
                    result,
                }
            })
            .collect();
        next.insert(kind, Arc::from(rows));
    }
    if let Ok(mut guard) = cache().write() {
        guard.by_type = next;
    }
}

/// Snapshot of cached recipes for `kind` (e.g. `"theurgy:calcination"`).
pub fn of_type(kind: &str) -> Arc<[CachedRecipe]> {
    if let Ok(guard) = cache().read()
        && let Some(list) = guard.by_type.get(kind)
    {
        return Arc::clone(list);
    }
    Arc::from([])
}

/// Live match test for a cached ingredient against a live `ItemStack`.
pub fn matches_cached(host: &dyn Host, ing: &CachedIngredient, stack: &ItemStack) -> bool {
    ing.parsed
        .as_ref()
        .is_some_and(|parsed| parsed.test(host, stack))
}

/// Live match test for an arbitrary JSON ingredient value against a live `ItemStack`.
pub fn matches_value(host: &dyn Host, value: &Value, stack: &ItemStack) -> bool {
    parse_ingredient(host, value).is_some_and(|parsed| parsed.test(host, stack))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::serde_json::json;

    #[test]
    fn recipe_time_defaults_match_machine_types() {
        assert_eq!(recipe_time(&json!({}), "theurgy:calcination"), 100);
        assert_eq!(recipe_time(&json!({}), "theurgy:fermentation"), 200);
        assert_eq!(recipe_time(&json!({}), "theurgy:digestion"), 200);
        assert_eq!(recipe_time(&json!({"time": 40}), "theurgy:calcination"), 40);
        assert_eq!(recipe_time(&json!({"time": -10}), "theurgy:calcination"), 0);
    }

    #[test]
    fn empty_cache_returns_empty_slice() {
        clear();
        assert!(of_type("theurgy:calcination").is_empty());
    }
}
