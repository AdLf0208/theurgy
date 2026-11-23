// SPDX-License-Identifier: MIT
//! The alchemical sulfur and niter jars as a special item model (`theurgy:alchemical_derivative`,
//! upstream's `AlchemicalDerivativeRenderer`): the jar, its tier's label frame, the label and
//! the source item on it, in every display context — in a slot, in the hand, on the ground.

use mod_api::serde_json::{self, Value};
use mod_api::{
    ClientHost, Identifier, ItemStack, PackType, RegisterSpecialModelRendererEvent,
    SpecialModelPart, SpecialModelRenderer, Tag,
};
use std::sync::Arc;

use crate::{client, config, host};

pub const MODEL_TYPE: &str = "theurgy:alchemical_derivative";

const DEFAULT_JAR: &str = "theurgy:item/empty_jar_icon";
/// The jar drawn when the source item is not: its label carries scribbles instead.
const LABELED_JAR: &str = "theurgy:item/empty_jar_labeled_icon";
const LABEL: &str = "theurgy:item/jar_label_icon";

const PIXEL: f32 = 1.0 / 16.0;

/// A part scaled by `scale` about the model's centre and then moved by `offset`: a row-major
/// 3×4 transform in the special model's space, whose origin is the model's corner.
pub fn about_centre(scale: [f32; 3], offset: [f32; 3]) -> [f32; 12] {
    let mut out = [0.0; 12];
    for axis in 0..3 {
        out[axis * 4 + axis] = scale[axis];
        out[axis * 4 + 3] = 0.5 - 0.5 * scale[axis] + offset[axis];
    }
    out
}

/// The label frame and the label: flattened and laid half a pixel forward, on the jar's front.
pub fn overlay_transform() -> [f32; 12] {
    about_centre([1.0, 1.0, 0.01], [0.0, 0.0, PIXEL * 0.5])
}

/// The source item on the label. Upstream's chain: forward 0.6 pixel, scale 0.36, down 3.2
/// pixels (in the scaled space), then 0.74 across and flat.
pub fn source_transform() -> [f32; 12] {
    let scale = 0.36;
    about_centre(
        [scale * 0.74, scale * 0.74, scale * 0.01],
        [0.0, scale * -PIXEL * 3.2, PIXEL * 0.6],
    )
}

/// The plain models an item definition (`assets/<ns>/items/<path>.json`'s `model`) draws for a
/// stack with nothing special about it: the model of a `minecraft:model`, every model of a
/// composite, the fallback of a select or range dispatch and the false branch of a condition.
pub fn definition_models(node: &Value, out: &mut Vec<String>) {
    let kind = node.get("type").and_then(Value::as_str).unwrap_or("");
    let child = |key: &str, out: &mut Vec<String>| {
        if let Some(child) = node.get(key) {
            definition_models(child, out);
        }
    };
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "model" => {
            if let Some(model) = node.get("model").and_then(Value::as_str) {
                out.push(model.to_owned());
            }
        }
        "composite" => {
            for model in node.get("models").and_then(Value::as_array).into_iter().flatten() {
                definition_models(model, out);
            }
        }
        "condition" => child("on_false", out),
        "select" | "range_dispatch" => {
            if node.get("fallback").is_some() {
                child("fallback", out);
            } else {
                let first = ["cases", "entries"]
                    .into_iter()
                    .filter_map(|key| node.get(key).and_then(Value::as_array))
                    .find_map(|list| list.first())
                    .and_then(|entry| entry.get("model"));
                if let Some(model) = first {
                    definition_models(model, out);
                }
            }
        }
        _ => {}
    }
}

/// `ns:path` as the resource `ns:<folder>/path.json`.
fn resource_id(id: &str, folder: &str) -> Option<Identifier> {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    Identifier::parse(&format!("{namespace}:{folder}/{path}.json")).ok()
}

/// The model files item `item` is drawn with, as far as its definition names plain ones that
/// the packs have. Empty for an item drawn by code, so the jar is never refused for it.
fn item_models(host_ref: &dyn ClientHost, item: &str) -> Vec<String> {
    let Some(definition) = resource_id(item, "items")
        .and_then(|id| host_ref.resource(PackType::ClientResources, &id))
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return Vec::new();
    };
    let mut models = Vec::new();
    if let Some(model) = definition.get("model") {
        definition_models(model, &mut models);
    }
    models.retain(|model| {
        resource_id(model, "models")
            .is_some_and(|id| host_ref.resource(PackType::ClientResources, &id).is_some())
    });
    models
}

struct Derivative;

impl SpecialModelRenderer for Derivative {
    fn extract(
        &self,
        host_ref: &dyn ClientHost,
        stack: &ItemStack,
        config: &Tag,
    ) -> Result<Vec<SpecialModelPart>, String> {
        let jar = match config {
            Tag::Compound(map) => host::string(map, "jar"),
            _ => None,
        }
        .unwrap_or_else(|| DEFAULT_JAR.to_owned());
        let item_id = stack.item().id(host_ref);
        let render_source = config::get().render_sulfur_source_item;
        let mut parts = vec![SpecialModelPart::new(if render_source { jar } else { LABELED_JAR.to_owned() })];
        if let Some(tier) = client::derivative_tier(host_ref, stack, &item_id) {
            parts.push(
                SpecialModelPart::new(format!("theurgy:item/jar_label_frame_{tier}_icon"))
                    .with_transform(overlay_transform()),
            );
        }
        if render_source {
            parts.push(SpecialModelPart::new(LABEL).with_transform(overlay_transform()));
            if let Some(source) = client::derivative_source(host_ref, stack) {
                for model in item_models(host_ref, &source) {
                    parts.push(SpecialModelPart::new(model).with_transform(source_transform()));
                }
            }
        }
        Ok(parts)
    }
}

pub fn register(event: &mut RegisterSpecialModelRendererEvent<'_>) {
    event.renderers.register(MODEL_TYPE.to_owned(), Arc::new(Derivative));
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::serde_json::json;

    fn apply(transform: [f32; 12], point: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|row| {
            (0..3).map(|k| transform[row * 4 + k] * point[k]).sum::<f32>() + transform[row * 4 + 3]
        })
    }

    #[test]
    fn overlays_lie_flat_on_the_front_of_the_jar() {
        let overlay = overlay_transform();
        // The corners keep their place across the model; only the depth collapses.
        assert_eq!(apply(overlay, [0.0, 0.0, 0.5])[..2], [0.0, 0.0]);
        assert_eq!(apply(overlay, [1.0, 1.0, 0.5])[..2], [1.0, 1.0]);
        // A generated item model is one pixel thick about z = 0.5: its front is at 8.5 pixels.
        let front = apply(overlay, [0.5, 0.5, 8.5 * PIXEL])[2];
        let back = apply(overlay, [0.5, 0.5, 7.5 * PIXEL])[2];
        assert!((front - back) < 0.02 * PIXEL);
        assert!(front >= 8.5 * PIXEL && back > 8.0 * PIXEL, "{back} {front}");
    }

    #[test]
    fn the_source_item_sits_small_on_the_label_in_front_of_it() {
        let source = source_transform();
        let centre = apply(source, [0.5, 0.5, 0.5]);
        assert!((centre[0] - 0.5).abs() < 1.0e-6);
        // 0.36 x 3.2 pixels below the middle, and in front of the overlays.
        assert!((centre[1] - (0.5 - 0.36 * 3.2 * PIXEL)).abs() < 1.0e-6);
        assert!(centre[2] > apply(overlay_transform(), [0.5, 0.5, 0.5])[2]);
        let width = apply(source, [1.0, 0.5, 0.5])[0] - apply(source, [0.0, 0.5, 0.5])[0];
        assert!((width - 0.36 * 0.74).abs() < 1.0e-6);
    }

    #[test]
    fn an_item_definition_names_its_plain_models() {
        let mut out = Vec::new();
        definition_models(&json!({"type": "minecraft:model", "model": "minecraft:item/iron_ingot"}), &mut out);
        assert_eq!(out, ["minecraft:item/iron_ingot"]);

        out.clear();
        definition_models(
            &json!({"type": "composite", "models": [
                {"type": "model", "model": "a:item/one"},
                {"type": "minecraft:condition", "property": "minecraft:broken",
                 "on_true": {"type": "model", "model": "a:item/broken"},
                 "on_false": {"type": "model", "model": "a:item/two"}}
            ]}),
            &mut out,
        );
        assert_eq!(out, ["a:item/one", "a:item/two"]);

        out.clear();
        definition_models(
            &json!({"type": "minecraft:select", "cases": [{"when": "x", "model": {"type": "model", "model": "a:item/case"}}],
                    "fallback": {"type": "model", "model": "a:item/fallback"}}),
            &mut out,
        );
        assert_eq!(out, ["a:item/fallback"]);

        out.clear();
        definition_models(
            &json!({"type": "minecraft:range_dispatch", "entries": [{"threshold": 0, "model": {"type": "model", "model": "a:item/first"}}]}),
            &mut out,
        );
        assert_eq!(out, ["a:item/first"]);

        // An item drawn by code has no plain model to show.
        out.clear();
        definition_models(&json!({"type": "minecraft:special", "base": "a:item/base", "model": {"type": "minecraft:chest"}}), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn ids_become_resource_names() {
        assert_eq!(resource_id("minecraft:iron_ingot", "items").map(|id| id.to_string()).as_deref(), Some("minecraft:items/iron_ingot.json"));
        assert_eq!(resource_id("theurgy:item/jar_label_icon", "models").map(|id| id.to_string()).as_deref(), Some("theurgy:models/item/jar_label_icon.json"));
    }
}
