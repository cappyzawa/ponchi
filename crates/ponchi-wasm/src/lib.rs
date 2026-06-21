//! WASM bindings for `ponchi-core`.
//!
//! Exposes a single entry point, [`render_svg`], that turns a scene JSON string
//! into a hand-drawn SVG document string. All work happens in `ponchi-core`,
//! which has no platform dependencies; this crate only bridges it to JavaScript.
//!
//! No network access is performed: the browser renders the returned SVG, so the
//! local-only invariant holds even in the WASM build.

use wasm_bindgen::prelude::*;

/// Render a scene JSON string to a hand-drawn SVG document string.
///
/// Returns the SVG text on success, or a `JsValue` carrying the error message
/// (invalid JSON, schema violation, etc.) on failure.
#[wasm_bindgen]
pub fn render_svg(scene_json: &str) -> Result<String, JsValue> {
    let scene = ponchi_core::input::parse_and_resolve(scene_json)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(ponchi_core::render::render_scene_with_font(
        &scene, "Yomogi",
    ))
}
