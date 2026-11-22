// SPDX-License-Identifier: MIT
//! Read-only native recipe viewer for Theurgy's data-driven recipes.

use std::sync::Mutex;

use mod_api::serde_json::Value;
use mod_api::{ClientHost, DrawContext, ItemStack, ScreenView};

use crate::engine::recipes::{fluid_part, ingredient_parts};
use crate::recipes;

const KINDS: &[&str] = &[
    "theurgy:calcination",
    "theurgy:distillation",
    "theurgy:liquefaction",
    "theurgy:fermentation",
    "theurgy:digestion",
    "theurgy:incubation",
    "theurgy:reformation",
    "theurgy:catalysation",
    "theurgy:accumulation",
];

struct RecipeView {
    kind: &'static str,
    id: String,
    result: Option<ItemStack>,
    time: i64,
    inputs: Vec<String>,
    fluid: Option<String>,
}

struct RecipeUi {
    recipes: Vec<RecipeView>,
    selected: Option<usize>,
    kind: usize,
    page: usize,
}

static RECIPE_UI: Mutex<Option<RecipeUi>> = Mutex::new(None);

pub fn close() {
    if let Ok(mut slot) = RECIPE_UI.lock() {
        *slot = None;
    }
}

pub fn open(host_ref: &mut dyn ClientHost) -> bool {
    let mut rows = Vec::new();
    for &kind in KINDS {
        for recipe in host_ref.recipes_of_type(kind).iter() {
            let result = recipe
                .json
                .get("result")
                .and_then(|v| recipes::parse_result(host_ref, v));
            let time = recipe
                .json
                .get("time")
                .and_then(Value::as_i64)
                .unwrap_or(100);
            let inputs = ingredient_parts(kind, &recipe.json)
                .into_iter()
                .map(|(ingredient, count)| format!("{count}x {ingredient}"))
                .collect();
            let fluid = fluid_part(kind, &recipe.json)
                .map(|(fluid, amount)| format!("{amount} mB {fluid}"));
            rows.push(RecipeView {
                kind,
                id: recipe.id.clone(),
                result,
                time,
                inputs,
                fluid,
            });
        }
    }
    rows.sort_by(|a, b| a.kind.cmp(b.kind).then(a.id.cmp(&b.id)));
    if rows.is_empty() {
        return false;
    }
    host_ref.mod_screen_open("theurgy");
    if let Ok(mut slot) = RECIPE_UI.lock() {
        *slot = Some(RecipeUi {
            recipes: rows,
            selected: None,
            kind: 0,
            page: 0,
        });
        true
    } else {
        host_ref.mod_screen_close("theurgy");
        false
    }
}

fn panel(width: f32, height: f32) -> [f32; 4] {
    let w = 360.0f32.min((width - 16.0).max(240.0));
    let h = 240.0f32.min((height - 16.0).max(180.0));
    [((width - w) / 2.0).max(4.0), ((height - h) / 2.0).max(4.0), w, h]
}

fn inside(point: [f32; 2], rect: [f32; 4]) -> bool {
    point[0] >= rect[0]
        && point[1] >= rect[1]
        && point[0] < rect[0] + rect[2]
        && point[1] < rect[1] + rect[3]
}

fn filtered(ui: &RecipeUi) -> Vec<usize> {
    ui.recipes
        .iter()
        .enumerate()
        .filter_map(|(index, recipe)| {
            (ui.kind == 0
                || ui.kind
                    == KINDS
                        .iter()
                        .position(|kind| *kind == recipe.kind)
                        .unwrap_or(usize::MAX)
                        + 1)
            .then_some(index)
        })
        .collect()
}

pub fn render(
    host_ref: &dyn ClientHost,
    draw: &mut dyn DrawContext,
    screen: &ScreenView<'_>,
) -> bool {
    if screen.id != "mod" || host_ref.mod_screen() != Some("theurgy") {
        return false;
    }
    let Ok(guard) = RECIPE_UI.lock() else {
        return false;
    };
    let Some(ui) = guard.as_ref() else {
        return false;
    };
    let [width, height] = draw.size();
    let rect = panel(width, height);
    draw.fill([0.0, 0.0, width, height], 0xA000_0000);
    draw.fill(rect, 0xFF18_251F);
    draw.fill(
        [rect[0] + 3.0, rect[1] + 3.0, rect[2] - 6.0, rect[3] - 6.0],
        0xFF26_3A30,
    );
    draw.text(
        "Theurgy Recipes",
        [rect[0] + 9.0, rect[1] + 8.0],
        0xFFE7_D59A,
        false,
    );
    draw.text(
        if ui.kind == 0 {
            "All types"
        } else {
            KINDS[ui.kind - 1]
        },
        [rect[0] + 9.0, rect[1] + 25.0],
        0xFFA9_C9B0,
        false,
    );
    if let Some(selected) = ui.selected {
        let Some(recipe) = ui.recipes.get(selected) else {
            return false;
        };
        draw.text("< Back", [rect[0] + 9.0, rect[1] + 44.0], 0xFFFF_FFFF, false);
        draw.text(
            &recipe.id,
            [rect[0] + 60.0, rect[1] + 44.0],
            0xFFE7_D59A,
            false,
        );
        if let Some(result) = &recipe.result {
            let _ = draw.item_stack(result, [rect[0] + 10.0, rect[1] + 64.0], 24.0);
            let name = result.item().id(host_ref);
            draw.text(
                &format!("{}x {name}", result.count()),
                [rect[0] + 42.0, rect[1] + 72.0],
                0xFFFF_FFFF,
                false,
            );
        }
        draw.text(
            &format!("{} ticks", recipe.time),
            [rect[0] + 10.0, rect[1] + 96.0],
            0xFFA9_C9B0,
            false,
        );
        for (index, input) in recipe.inputs.iter().take(8).enumerate() {
            draw.text(
                input,
                [rect[0] + 10.0, rect[1] + 114.0 + index as f32 * 10.0],
                0xFFFF_FFFF,
                false,
            );
        }
        if let Some(fluid) = &recipe.fluid {
            draw.text(
                fluid,
                [rect[0] + 10.0, rect[1] + rect[3] - 38.0],
                0xFFA9_C9B0,
                false,
            );
        }
        return true;
    }
    let list = filtered(ui);
    for (row, index) in list.iter().skip(ui.page * 8).take(8).enumerate() {
        let Some(recipe) = ui.recipes.get(*index) else {
            continue;
        };
        let y = rect[1] + 40.0 + row as f32 * 20.0;
        draw.fill([rect[0] + 8.0, y, rect[2] - 16.0, 18.0], 0xFF34_483A);
        if let Some(result) = &recipe.result {
            let _ = draw.item_stack(result, [rect[0] + 10.0, y + 1.0], 16.0);
        }
        draw.text(&recipe.id, [rect[0] + 32.0, y + 5.0], 0xFFFF_FFFF, false);
    }
    let pages = list.len().div_ceil(8).max(1);
    draw.text(
        &format!("Page {}/{}   (click type to cycle)", ui.page + 1, pages),
        [rect[0] + 9.0, rect[1] + rect[3] - 19.0],
        0xFFE7_D59A,
        false,
    );
    true
}

pub fn click(host_ref: &dyn ClientHost, screen: &ScreenView<'_>, button: i32) -> bool {
    if screen.id != "mod" || host_ref.mod_screen() != Some("theurgy") || button != 0 {
        return false;
    }
    let Ok(mut guard) = RECIPE_UI.lock() else {
        return false;
    };
    let Some(ui) = guard.as_mut() else {
        return false;
    };
    let rect = panel(screen.rect[2], screen.rect[3]);
    if inside(screen.mouse, [rect[0] + 8.0, rect[1] + 20.0, 180.0, 17.0]) {
        ui.kind = (ui.kind + 1) % (KINDS.len() + 1);
        ui.selected = None;
        ui.page = 0;
        return true;
    }
    if ui.selected.is_some()
        && inside(screen.mouse, [rect[0] + 8.0, rect[1] + 40.0, 44.0, 16.0])
    {
        ui.selected = None;
        return true;
    }
    if ui.selected.is_none() {
        let list = filtered(ui);
        for (row, index) in list.iter().skip(ui.page * 8).take(8).enumerate() {
            let y = rect[1] + 40.0 + row as f32 * 20.0;
            if inside(screen.mouse, [rect[0] + 8.0, y, rect[2] - 16.0, 18.0]) {
                ui.selected = Some(*index);
                return true;
            }
        }
    }
    if screen.mouse[1] >= rect[1] + rect[3] - 24.0 {
        let pages = filtered(ui).len().div_ceil(8).max(1);
        if screen.mouse[0] < rect[0] + rect[2] / 2.0 {
            ui.page = ui.page.saturating_sub(1);
        } else {
            ui.page = (ui.page + 1).min(pages - 1);
        }
    }
    true
}

pub fn scroll(host_ref: &dyn ClientHost, screen: &ScreenView<'_>, delta: f64) -> bool {
    if screen.id != "mod" || host_ref.mod_screen() != Some("theurgy") {
        return false;
    }
    let Ok(mut guard) = RECIPE_UI.lock() else {
        return false;
    };
    let Some(ui) = guard.as_mut() else {
        return false;
    };
    if ui.selected.is_some() {
        return true;
    }
    let pages = filtered(ui).len().div_ceil(8).max(1);
    ui.page = if delta > 0.0 {
        ui.page.saturating_sub(1)
    } else {
        (ui.page + 1).min(pages - 1)
    };
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_has_strict_far_edge() {
        assert!(inside([2.0, 2.0], [2.0, 2.0, 5.0, 5.0]));
        assert!(!inside([7.0, 2.0], [2.0, 2.0, 5.0, 5.0]));
    }
}
