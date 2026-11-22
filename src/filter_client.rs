// SPDX-License-Identifier: MIT
//! Native list/attribute filter screen. Rendering is client-local; every edit
//! is sent via a typed C2S payload (`FilterEditPayload`) to the server.

use std::sync::Mutex;

use mod_api::{ClientHost, Compound, DrawContext, Hand, Host, ItemStack, ScreenView, Tag};

use crate::{filter, host, items};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    List,
    Attribute,
}

struct RuleView {
    label: String,
    inverted: bool,
}

struct FilterUi {
    kind: Kind,
    hand: Hand,
    mode: String,
    respect: bool,
    entries: Vec<ItemStack>,
    rules: Vec<RuleView>,
    page: usize,
    refresh_ticks: u32,
    editing: bool,
    input: String,
}

#[derive(Clone, Debug)]
pub struct PendingEdit {
    pub hand: Hand,
    pub action: String,
    pub args: String,
}

static FILTER_UI: Mutex<Option<FilterUi>> = Mutex::new(None);
static UI_ACTIONS: Mutex<Vec<PendingEdit>> = Mutex::new(Vec::new());

fn map(tag: Option<Tag>) -> Compound {
    match tag {
        Some(Tag::Compound(map)) => map,
        _ => Compound::new(),
    }
}

fn text_field(map: &Compound, key: &str, fallback: &str) -> String {
    host::string(map, key).unwrap_or_else(|| fallback.to_owned())
}

fn load(host_ref: &dyn Host, stack: &ItemStack, hand: Hand, page: usize) -> Option<FilterUi> {
    let id = stack.item().id(host_ref);
    let kind = match id.as_str() {
        "theurgy:list_filter" => Kind::List,
        "theurgy:attribute_filter" => Kind::Attribute,
        _ => return None,
    };
    let (mode, respect, entries, rules) = if kind == Kind::List {
        let cfg = map(items::component_tag(host_ref, stack, filter::LIST_CONFIG));
        let mut entries = Vec::new();
        if let Some(Tag::List(values)) = items::component_tag(host_ref, stack, filter::LIST_CONTENTS)
        {
            for value in &values {
                if let Some(st) = filter::stack_from_template(host_ref, value) {
                    entries.push(st);
                }
            }
        }
        (
            text_field(&cfg, "mode", "allow"),
            host::bool(&cfg, "respect_data_components"),
            entries,
            Vec::new(),
        )
    } else {
        let cfg = map(items::component_tag(host_ref, stack, filter::ATTRIBUTE_CONFIG));
        let mut rules = Vec::new();
        if let Some(Tag::List(values)) = cfg.get(&"rules".into()) {
            for value in values {
                let Tag::Compound(rule) = value else {
                    continue;
                };
                let rk = text_field(rule, "type", "unknown");
                let payload = match rule.get(&"payload".into()) {
                    Some(Tag::Compound(p)) => p.clone(),
                    _ => Compound::new(),
                };
                let label = if rk == "codedefinedgui:in_tag" {
                    text_field(&payload, "id", "tag")
                } else {
                    text_field(&payload, "value", &rk)
                };
                rules.push(RuleView {
                    label,
                    inverted: host::bool(rule, "inverted"),
                });
            }
        }
        (
            text_field(&cfg, "mode", "match_any"),
            false,
            Vec::new(),
            rules,
        )
    };
    Some(FilterUi {
        kind,
        hand,
        mode,
        respect,
        entries,
        rules,
        page,
        refresh_ticks: 0,
        editing: false,
        input: String::new(),
    })
}

pub fn open(host_ref: &mut dyn ClientHost, stack: &ItemStack, hand: Hand) -> bool {
    let Some(ui) = load(host_ref, stack, hand, 0) else {
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

fn button(draw: &mut dyn DrawContext, rect: [f32; 4], label: &str, active: bool) {
    draw.fill(rect, if active { 0xFF65_7B48 } else { 0xFF34_483A });
    draw.text(label, [rect[0] + 4.0, rect[1] + 4.0], 0xFFFF_FFFF, false);
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
    let [width, height] = draw.size();
    let rect = panel(width, height);
    if ui.editing {
        draw.fill([0.0, 0.0, width, height], 0xA000_0000);
        draw.fill(rect, 0xFF18_251F);
        draw.text(
            "Advanced rule: <kind> <JSON payload>",
            [rect[0] + 10.0, rect[1] + 12.0],
            0xFFE7_D59A,
            false,
        );
        draw.text(
            r#"Example: codedefinedgui:standard {"value":"renamed"}"#,
            [rect[0] + 10.0, rect[1] + 28.0],
            0xFFA9_C9B0,
            false,
        );
        draw.fill(
            [rect[0] + 10.0, rect[1] + 52.0, rect[2] - 20.0, 22.0],
            0xFF10_1010,
        );
        draw.text(&ui.input, [rect[0] + 14.0, rect[1] + 58.0], 0xFFFF_FFFF, false);
        draw.text(
            "Enter: apply   Escape: cancel",
            [rect[0] + 10.0, rect[1] + rect[3] - 24.0],
            0xFFFF_FFFF,
            false,
        );
        return true;
    }
    draw.fill([0.0, 0.0, width, height], 0xA000_0000);
    draw.fill(rect, 0xFF18_251F);
    draw.fill(
        [rect[0] + 3.0, rect[1] + 3.0, rect[2] - 6.0, rect[3] - 6.0],
        0xFF26_3A30,
    );
    let title = if ui.kind == Kind::List {
        "List Filter"
    } else {
        "Attribute Filter"
    };
    draw.text(title, [rect[0] + 9.0, rect[1] + 8.0], 0xFFE7_D59A, false);
    if ui.kind == Kind::List {
        for (index, (mode, label)) in [("allow", "Allow"), ("deny", "Deny")].into_iter().enumerate()
        {
            button(
                draw,
                [rect[0] + 8.0 + index as f32 * 60.0, rect[1] + 23.0, 56.0, 15.0],
                label,
                ui.mode == mode,
            );
        }
        button(
            draw,
            [rect[0] + 134.0, rect[1] + 23.0, 138.0, 15.0],
            "Match components",
            ui.respect,
        );
        for (index, stack) in ui.entries.iter().skip(ui.page * 18).take(18).enumerate() {
            let x = rect[0] + 10.0 + (index % 9) as f32 * 20.0;
            let y = rect[1] + 48.0 + (index / 9) as f32 * 20.0;
            draw.fill([x - 1.0, y - 1.0, 18.0, 18.0], 0xFF4A_5C50);
            let _ = draw.item_stack(stack, [x, y], 16.0);
        }
        button(
            draw,
            [rect[0] + 8.0, rect[1] + rect[3] - 42.0, 106.0, 15.0],
            "Add held item",
            false,
        );
        button(
            draw,
            [rect[0] + 120.0, rect[1] + rect[3] - 42.0, 64.0, 15.0],
            "Clear",
            false,
        );
    } else {
        for (index, (mode, label)) in [
            ("match_any", "Any"),
            ("match_all", "All"),
            ("deny", "Deny"),
        ]
        .into_iter()
        .enumerate()
        {
            button(
                draw,
                [rect[0] + 8.0 + index as f32 * 60.0, rect[1] + 23.0, 56.0, 15.0],
                label,
                ui.mode == mode,
            );
        }
        for (row, rule) in ui.rules.iter().skip(ui.page * 7).take(7).enumerate() {
            let y = rect[1] + 45.0 + row as f32 * 18.0;
            draw.fill([rect[0] + 8.0, y, rect[2] - 16.0, 16.0], 0xFF34_483A);
            let prefix = if rule.inverted { "! " } else { "" };
            draw.text(
                &format!("{prefix}{}", rule.label),
                [rect[0] + 12.0, y + 4.0],
                0xFFFF_FFFF,
                false,
            );
            draw.text(
                "invert",
                [rect[0] + rect[2] - 105.0, y + 4.0],
                0xFFE7_D59A,
                false,
            );
            draw.text(
                "remove",
                [rect[0] + rect[2] - 53.0, y + 4.0],
                0xFFE7_D59A,
                false,
            );
        }
        button(
            draw,
            [rect[0] + 8.0, rect[1] + rect[3] - 42.0, 92.0, 15.0],
            "Add renamed",
            false,
        );
        button(
            draw,
            [rect[0] + 106.0, rect[1] + rect[3] - 42.0, 88.0, 15.0],
            "Add item tag",
            false,
        );
        button(
            draw,
            [rect[0] + 200.0, rect[1] + rect[3] - 42.0, 64.0, 15.0],
            "Clear",
            false,
        );
        button(
            draw,
            [rect[0] + 270.0, rect[1] + rect[3] - 42.0, 78.0, 15.0],
            "Advanced",
            false,
        );
    }
    let count = if ui.kind == Kind::List {
        ui.entries.len()
    } else {
        ui.rules.len()
    };
    let page_size = if ui.kind == Kind::List { 18 } else { 7 };
    draw.text(
        &format!(
            "{} rules   Page {}/{}",
            count,
            ui.page + 1,
            count.div_ceil(page_size).max(1)
        ),
        [rect[0] + 9.0, rect[1] + rect[3] - 19.0],
        0xFFE7_D59A,
        false,
    );
    true
}

fn queue(ui: &mut FilterUi, action: &str, args: impl Into<String>) {
    if let Ok(mut actions) = UI_ACTIONS.lock() {
        actions.push(PendingEdit {
            hand: ui.hand,
            action: action.to_owned(),
            args: args.into(),
        });
        ui.refresh_ticks = 4;
    }
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
    let rect = panel(screen.rect[2], screen.rect[3]);
    let point = screen.mouse;
    if ui.kind == Kind::List {
        for (index, mode) in ["allow", "deny"].into_iter().enumerate() {
            if inside(
                point,
                [rect[0] + 8.0 + index as f32 * 60.0, rect[1] + 23.0, 56.0, 15.0],
            ) {
                queue(ui, "mode", mode);
                return true;
            }
        }
        if inside(point, [rect[0] + 134.0, rect[1] + 23.0, 138.0, 15.0]) {
            queue(ui, "components", (!ui.respect).to_string());
            return true;
        }
        for (index, _) in ui.entries.iter().skip(ui.page * 18).take(18).enumerate() {
            let x = rect[0] + 10.0 + (index % 9) as f32 * 20.0;
            let y = rect[1] + 48.0 + (index / 9) as f32 * 20.0;
            if inside(point, [x - 1.0, y - 1.0, 18.0, 18.0]) {
                queue(ui, "remove", (ui.page * 18 + index).to_string());
                return true;
            }
        }
        if inside(point, [rect[0] + 8.0, rect[1] + rect[3] - 42.0, 106.0, 15.0]) {
            queue(ui, "add", "auto");
        } else if inside(point, [rect[0] + 120.0, rect[1] + rect[3] - 42.0, 64.0, 15.0]) {
            queue(ui, "clear", "");
        }
    } else {
        for (index, mode) in ["match_any", "match_all", "deny"].into_iter().enumerate() {
            if inside(
                point,
                [rect[0] + 8.0 + index as f32 * 60.0, rect[1] + 23.0, 56.0, 15.0],
            ) {
                queue(ui, "mode", mode);
                return true;
            }
        }
        for (row, _) in ui.rules.iter().skip(ui.page * 7).take(7).enumerate() {
            let y = rect[1] + 45.0 + row as f32 * 18.0;
            if !inside(point, [rect[0] + 8.0, y, rect[2] - 16.0, 16.0]) {
                continue;
            }
            let index = ui.page * 7 + row;
            if point[0] >= rect[0] + rect[2] - 60.0 {
                queue(ui, "rule_remove", index.to_string());
            } else if point[0] >= rect[0] + rect[2] - 112.0 {
                queue(ui, "rule_invert", index.to_string());
            }
            return true;
        }
        if inside(point, [rect[0] + 8.0, rect[1] + rect[3] - 42.0, 92.0, 15.0]) {
            queue(ui, "rule_renamed", "");
        } else if inside(point, [rect[0] + 106.0, rect[1] + rect[3] - 42.0, 88.0, 15.0]) {
            queue(ui, "rule_tag", "c:ingots");
        } else if inside(point, [rect[0] + 200.0, rect[1] + rect[3] - 42.0, 64.0, 15.0]) {
            queue(ui, "clear", "");
        } else if inside(point, [rect[0] + 270.0, rect[1] + rect[3] - 42.0, 78.0, 15.0]) {
            ui.editing = true;
            ui.input.clear();
        }
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
    let (count, size) = if ui.kind == Kind::List {
        (ui.entries.len(), 18)
    } else {
        (ui.rules.len(), 7)
    };
    let pages = count.div_ceil(size).max(1);
    ui.page = if delta > 0.0 {
        ui.page.saturating_sub(1)
    } else {
        (ui.page + 1).min(pages - 1)
    };
    true
}

pub fn char_typed(host_ref: &dyn ClientHost, screen: &ScreenView<'_>, c: char) -> bool {
    if screen.id != "mod" || host_ref.mod_screen() != Some("theurgy") {
        return false;
    }
    let Ok(mut guard) = FILTER_UI.lock() else {
        return false;
    };
    let Some(ui) = guard.as_mut() else {
        return false;
    };
    if !ui.editing {
        return false;
    }
    if ui.input.len() < 256 && !c.is_control() {
        ui.input.push(c);
    }
    true
}

pub fn key_pressed(host_ref: &dyn ClientHost, screen: &ScreenView<'_>, key: &str) -> bool {
    if screen.id != "mod" || host_ref.mod_screen() != Some("theurgy") {
        return false;
    }
    let Ok(mut guard) = FILTER_UI.lock() else {
        return false;
    };
    let Some(ui) = guard.as_mut() else {
        return false;
    };
    if !ui.editing {
        return false;
    }
    match key {
        "key.keyboard.escape" => {
            ui.editing = false;
            ui.input.clear();
        }
        "key.keyboard.backspace" => {
            ui.input.pop();
        }
        "key.keyboard.enter" | "key.keyboard.keypad_enter" => {
            if ui.input.split_once(' ').is_some() {
                let action = ui.input.trim().to_owned();
                queue(ui, "rule_add", action);
                ui.editing = false;
                ui.input.clear();
            }
        }
        _ => {}
    }
    true
}

pub fn take_actions() -> Vec<PendingEdit> {
    UI_ACTIONS
        .lock()
        .map(|mut actions| std::mem::take(&mut *actions))
        .unwrap_or_default()
}

pub fn tick_refresh(host_ref: &dyn ClientHost) {
    let refresh = FILTER_UI.lock().ok().and_then(|mut slot| {
        let ui = slot.as_mut()?;
        if ui.refresh_ticks == 0 {
            return None;
        }
        ui.refresh_ticks -= 1;
        (ui.refresh_ticks == 0).then_some((ui.hand, ui.page))
    });
    let Some((hand, page)) = refresh else {
        return;
    };
    let Some(player) = host_ref.local_player() else {
        return;
    };
    let held = host_ref
        .living(player.entity())
        .and_then(|living| living.hand_item(host_ref, hand));
    let updated = held.and_then(|stack| load(host_ref, &stack, hand, page));
    if let Ok(mut slot) = FILTER_UI.lock() {
        *slot = updated;
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
}
