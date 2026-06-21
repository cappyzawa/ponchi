//! Input schema (the agent-facing "mouth") and resolution into a render-ready
//! [`Scene`].
//!
//! The render-ready [`Scene`] requires every node to carry explicit geometry
//! (`x`/`y`/`w`/`h`). To let an agent write a diagram without placing anything,
//! the input is parsed into geometry-optional types ([`InputScene`]) and then
//! *resolved*:
//!
//! - No `layout` block -> manual mode: geometry is required, exactly as before
//!   (full backward compatibility with existing scenes).
//! - `layout.mode = "auto"` -> the `dagre` engine places nodes and routes edges
//!   from labels and relationships alone; supplying `x`/`y` is an error (the
//!   intent would be ambiguous, so it is rejected rather than silently ignored).
//!
//! In auto mode only `nodes` and `edges` are placed. `zones` and `texts` remain
//! manual overlays and still require coordinates (a documented v1 limitation).

use crate::layout::{self, Direction, LayoutEdge, LayoutNode, LayoutParams};
use crate::scene::{
    Canvas, Edge, EdgeStyle, MAX_CANVAS_DIM, Node, Port, Role, Scene, Shape, Text, ValidationError,
    Zone, default_background,
};
use serde::Deserialize;
use std::collections::HashSet;

/// Layout mode selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutMode {
    /// Engine places nodes and routes edges.
    Auto,
}

/// Layout engine selector. Only the layered (Sugiyama/dagre) engine exists in
/// v1; the field is accepted for forward compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutEngine {
    #[default]
    Layered,
}

/// Layout flow direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LayoutDirection {
    /// Left to right.
    #[default]
    Lr,
    /// Top to bottom.
    Tb,
}

impl From<LayoutDirection> for Direction {
    fn from(d: LayoutDirection) -> Self {
        match d {
            LayoutDirection::Lr => Direction::Lr,
            LayoutDirection::Tb => Direction::Tb,
        }
    }
}

/// How the canvas size is chosen in auto mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Grow the canvas to fit the layout bounding box plus margin.
    #[default]
    Grow,
    /// Use the explicit `canvas`; error if the layout does not fit.
    Fixed,
}

/// The `layout` block that switches a scene from manual to auto placement.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub mode: LayoutMode,
    #[serde(default)]
    pub engine: LayoutEngine,
    #[serde(default)]
    pub direction: LayoutDirection,
    #[serde(default = "default_node_sep")]
    pub node_sep: f64,
    #[serde(default = "default_rank_sep")]
    pub rank_sep: f64,
    #[serde(default = "default_margin")]
    pub margin: f64,
    #[serde(default)]
    pub fit: Fit,
}

fn default_node_sep() -> f64 {
    55.0
}
fn default_rank_sep() -> f64 {
    90.0
}
fn default_margin() -> f64 {
    30.0
}

/// A node as written by the agent: geometry is optional. In manual mode
/// `x`/`y`/`w`/`h` are required; in auto mode `x`/`y` must be absent and
/// `w`/`h` are optional overrides on the auto-sized box.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputNode {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub shape: Shape,
    #[serde(default)]
    pub x: Option<f32>,
    #[serde(default)]
    pub y: Option<f32>,
    #[serde(default)]
    pub w: Option<f32>,
    #[serde(default)]
    pub h: Option<f32>,
    #[serde(default)]
    pub role: Option<Role>,
}

/// An edge as written by the agent. In auto mode, manual routing/label-offset
/// fields (`from_port`/`to_port`/`waypoints`/`label_pos`/`label_dx`/`label_dy`)
/// are ignored: the engine decides ports, route, and label position.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub from_port: Option<Port>,
    #[serde(default)]
    pub to_port: Option<Port>,
    #[serde(default)]
    pub style: EdgeStyle,
    #[serde(default)]
    pub dashed: bool,
    #[serde(default)]
    pub waypoints: Vec<crate::scene::Point>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "default_label_pos")]
    pub label_pos: f32,
    #[serde(default)]
    pub label_dx: f32,
    #[serde(default)]
    pub label_dy: f32,
}

fn default_label_pos() -> f32 {
    0.5
}

/// Optional explicit canvas. In manual mode it is required; in auto mode with
/// `fit = "grow"` it may be omitted (the size is computed from the layout).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputCanvas {
    pub w: u32,
    pub h: u32,
    #[serde(default = "default_background")]
    pub background: String,
}

/// The agent-facing scene: like [`Scene`] but geometry-optional and with an
/// optional `layout` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputScene {
    pub schema_version: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub layout: Option<Layout>,
    #[serde(default)]
    pub canvas: Option<InputCanvas>,
    #[serde(default)]
    pub nodes: Vec<InputNode>,
    #[serde(default)]
    pub edges: Vec<InputEdge>,
    #[serde(default)]
    pub zones: Vec<Zone>,
    #[serde(default)]
    pub texts: Vec<Text>,
}

impl InputScene {
    /// Parse an input scene from JSON without resolving it.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

/// Parse an input scene from JSON and resolve it into a validated, render-ready
/// [`Scene`]. This is the single entry point used by the CLI and the server.
pub fn parse_and_resolve(s: &str) -> Result<Scene, Box<dyn std::error::Error>> {
    let input = InputScene::from_json(s)?;
    let scene = resolve_scene(input)?;
    Ok(scene)
}

/// Resolve an [`InputScene`] into a validated, render-ready [`Scene`].
pub fn resolve_scene(input: InputScene) -> Result<Scene, ValidationError> {
    let scene = match &input.layout {
        None => resolve_manual(input)?,
        Some(_) => resolve_auto(input)?,
    };
    scene.validate()?;
    Ok(scene)
}

/// Manual mode: geometry is mandatory, matching the legacy `Scene` exactly.
///
/// The `schema_version` is not checked here: `resolve_scene` always calls
/// [`Scene::validate`] on the result, which is the single place that enforces
/// it (avoids triplicating the same check).
fn resolve_manual(input: InputScene) -> Result<Scene, ValidationError> {
    let canvas = input
        .canvas
        .ok_or_else(|| ValidationError("manual mode requires a canvas".into()))?;

    let mut nodes = Vec::with_capacity(input.nodes.len());
    for n in input.nodes {
        let (Some(x), Some(y), Some(w), Some(h)) = (n.x, n.y, n.w, n.h) else {
            return Err(ValidationError(format!(
                "node {:?} requires x, y, w, h in manual mode (add a layout block for auto placement)",
                n.id
            )));
        };
        nodes.push(Node {
            id: n.id,
            label: n.label,
            shape: n.shape,
            x,
            y,
            w,
            h,
            role: n.role,
        });
    }

    let edges = input
        .edges
        .into_iter()
        .map(|e| Edge {
            id: e.id,
            from: e.from,
            to: e.to,
            from_port: e.from_port,
            to_port: e.to_port,
            style: e.style,
            dashed: e.dashed,
            waypoints: e.waypoints,
            label: e.label,
            label_pos: e.label_pos,
            label_dx: e.label_dx,
            label_dy: e.label_dy,
            route: None,
            label_anchor: None,
        })
        .collect();

    Ok(Scene {
        schema_version: input.schema_version,
        title: input.title,
        seed: input.seed,
        canvas: Canvas {
            w: canvas.w,
            h: canvas.h,
            background: canvas.background,
        },
        nodes,
        edges,
        zones: input.zones,
        texts: input.texts,
    })
}

/// Auto mode: place nodes and route edges with the layout engine. Coordinates
/// on nodes are rejected; `zones`/`texts` are kept as manual overlays.
///
/// The `schema_version` is validated once by `resolve_scene` via
/// [`Scene::validate`], so it is not re-checked here.
fn resolve_auto(input: InputScene) -> Result<Scene, ValidationError> {
    let layout_cfg = input.layout.expect("resolve_auto called with a layout");
    validate_layout_params(&layout_cfg)?;

    // Reject explicit node coordinates outright: in auto mode the engine owns
    // placement, so an `x`/`y` would be silently dropped. Fail loudly instead.
    for n in &input.nodes {
        if n.x.is_some() || n.y.is_some() {
            return Err(ValidationError(format!(
                "node {:?} has x/y but layout.mode is auto; remove coordinates (the engine places nodes) or drop the layout block for manual mode",
                n.id
            )));
        }
    }

    // Validate the graph before feeding dagre. dagre silently invents a
    // phantom zero-size node for an edge endpoint it has never seen, producing
    // a dangling arrow; and duplicate node ids would later fail id-uniqueness
    // anyway. Reject both up front with a clear message.
    let mut ids: HashSet<&str> = HashSet::with_capacity(input.nodes.len());
    for n in &input.nodes {
        if !ids.insert(n.id.as_str()) {
            return Err(ValidationError(format!("duplicate node id {:?}", n.id)));
        }
    }
    for e in &input.edges {
        if !ids.contains(e.from.as_str()) {
            return Err(ValidationError(format!(
                "edge {:?} references unknown from node {:?}",
                e.id, e.from
            )));
        }
        if !ids.contains(e.to.as_str()) {
            return Err(ValidationError(format!(
                "edge {:?} references unknown to node {:?}",
                e.id, e.to
            )));
        }
    }

    let lnodes: Vec<LayoutNode> = input
        .nodes
        .iter()
        .map(|n| LayoutNode {
            id: n.id.clone(),
            label: n.label.clone(),
            shape: n.shape,
            role: n.role,
            w: n.w,
            h: n.h,
        })
        .collect();
    let ledges: Vec<LayoutEdge> = input
        .edges
        .iter()
        .map(|e| LayoutEdge {
            id: e.id.clone(),
            from: e.from.clone(),
            to: e.to.clone(),
            label: e.label.clone(),
            dashed: e.dashed,
            style: e.style,
        })
        .collect();

    let params = LayoutParams {
        direction: layout_cfg.direction.into(),
        node_sep: layout_cfg.node_sep,
        rank_sep: layout_cfg.rank_sep,
        margin: layout_cfg.margin,
    };
    let laid = layout::run(&lnodes, &ledges, params);

    // Canvas sizing. Margin is added once past the far extent of the content
    // (dagre already reserves `margin` on the top/left via marginx/marginy, so
    // this yields a symmetric bottom/right margin).
    //
    // `fixed` measures the far extent from the laid-out nodes/edges only: the
    // user picked the canvas, so manual `texts`/`zones` spilling over is their
    // responsibility and is not an error. `grow` additionally includes the
    // manual `texts`/`zones` overlays so the auto-expanded canvas does not clip
    // annotations placed beside or below the graph.
    let canvas = match layout_cfg.fit {
        // grow: the canvas is derived from the content, so any `canvas.w`/
        // `canvas.h` in the input is ignored; only its `background` is kept.
        Fit::Grow => {
            let mut far_x = laid.max_x;
            let mut far_y = laid.max_y;
            for z in &input.zones {
                far_x = far_x.max(z.x + z.w);
                far_y = far_y.max(z.y + z.h);
            }
            for t in &input.texts {
                let (right, bottom) = crate::render::free_text_extent(t);
                far_x = far_x.max(right);
                far_y = far_y.max(bottom);
            }
            let needed_w = (far_x + layout_cfg.margin as f32).ceil();
            let needed_h = (far_y + layout_cfg.margin as f32).ceil();
            Canvas {
                w: (needed_w.max(1.0) as u32).min(MAX_CANVAS_DIM),
                h: (needed_h.max(1.0) as u32).min(MAX_CANVAS_DIM),
                background: input
                    .canvas
                    .map(|c| c.background)
                    .unwrap_or_else(default_background),
            }
        }
        Fit::Fixed => {
            let needed_w = (laid.max_x + layout_cfg.margin as f32).ceil();
            let needed_h = (laid.max_y + layout_cfg.margin as f32).ceil();
            let c = input
                .canvas
                .ok_or_else(|| ValidationError("fit \"fixed\" requires a canvas".into()))?;
            if needed_w > c.w as f32 || needed_h > c.h as f32 {
                return Err(ValidationError(format!(
                    "layout ({needed_w}x{needed_h}) does not fit fixed canvas ({}x{}); use fit \"grow\" or enlarge the canvas",
                    c.w, c.h
                )));
            }
            Canvas {
                w: c.w,
                h: c.h,
                background: c.background,
            }
        }
    };

    Ok(Scene {
        schema_version: input.schema_version,
        title: input.title,
        seed: input.seed,
        canvas,
        nodes: laid.nodes,
        edges: laid.edges,
        zones: input.zones,
        texts: input.texts,
    })
}

/// Generous upper bound on layout spacing params; well past any sane diagram
/// but small enough that the resulting canvas stays under [`MAX_CANVAS_DIM`].
const MAX_LAYOUT_SPACING: f64 = 4096.0;

/// Reject non-finite or out-of-range layout spacing. A NaN would otherwise
/// propagate into the bbox and collapse the canvas to 1x1; a negative value
/// would pull content off-canvas.
fn validate_layout_params(layout: &Layout) -> Result<(), ValidationError> {
    for (name, v) in [
        ("node_sep", layout.node_sep),
        ("rank_sep", layout.rank_sep),
        ("margin", layout.margin),
    ] {
        if !v.is_finite() {
            return Err(ValidationError(format!("layout.{name} must be finite")));
        }
        if v < 0.0 {
            return Err(ValidationError(format!(
                "layout.{name} must be >= 0, got {v}"
            )));
        }
        if v > MAX_LAYOUT_SPACING {
            return Err(ValidationError(format!(
                "layout.{name} ({v}) exceeds max {MAX_LAYOUT_SPACING}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: &str = crate::scene::SCHEMA_VERSION;

    #[test]
    fn manual_mode_backward_compatible() {
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "canvas": {{ "w": 200, "h": 200 }},
                "nodes": [ {{ "id": "a", "shape": "rect", "x": 0, "y": 0, "w": 50, "h": 30 }} ]
            }}"#
        );
        let scene = parse_and_resolve(&json).unwrap();
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(scene.nodes[0].x, 0.0);
        assert!(scene.edges.is_empty());
    }

    #[test]
    fn manual_mode_missing_geometry_is_error() {
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "canvas": {{ "w": 200, "h": 200 }},
                "nodes": [ {{ "id": "a", "shape": "rect" }} ]
            }}"#
        );
        assert!(parse_and_resolve(&json).is_err());
    }

    #[test]
    fn auto_mode_places_nodes_without_coords() {
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto", "direction": "LR" }},
                "nodes": [
                    {{ "id": "a", "label": "A", "shape": "rect" }},
                    {{ "id": "b", "label": "B", "shape": "rect" }}
                ],
                "edges": [ {{ "id": "e1", "from": "a", "to": "b", "label": "go" }} ]
            }}"#
        );
        let scene = parse_and_resolve(&json).unwrap();
        assert_eq!(scene.nodes.len(), 2);
        // Engine assigned positive sizes and a route.
        assert!(scene.nodes.iter().all(|n| n.w > 0.0 && n.h > 0.0));
        assert!(scene.edges[0].route.as_ref().is_some_and(|r| r.len() >= 2));
        // Canvas was grown to fit.
        assert!(scene.canvas.w > 0 && scene.canvas.h > 0);
    }

    #[test]
    fn auto_mode_rejects_explicit_coords() {
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto" }},
                "nodes": [ {{ "id": "a", "label": "A", "shape": "rect", "x": 10, "y": 20 }} ]
            }}"#
        );
        let err = parse_and_resolve(&json).unwrap_err();
        assert!(err.to_string().contains("auto"), "got {err}");
    }

    #[test]
    fn auto_mode_w_h_override_allowed() {
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto" }},
                "nodes": [ {{ "id": "a", "label": "A", "shape": "rect", "w": 200, "h": 120 }} ]
            }}"#
        );
        let scene = parse_and_resolve(&json).unwrap();
        assert_eq!(scene.nodes[0].w, 200.0);
        assert_eq!(scene.nodes[0].h, 120.0);
    }

    #[test]
    fn auto_mode_shuffle_invariant() {
        let mk = |order: &[(&str, &str)]| {
            let nodes = r#"
                { "id": "a", "label": "A", "shape": "rect" },
                { "id": "b", "label": "B", "shape": "rect" },
                { "id": "c", "label": "C", "shape": "rect" }"#;
            let edges: Vec<String> = order
                .iter()
                .enumerate()
                .map(|(i, (f, t))| format!(r#"{{ "id": "e{i}", "from": "{f}", "to": "{t}" }}"#))
                .collect();
            format!(
                r#"{{
                    "schema_version": "{V}",
                    "layout": {{ "mode": "auto" }},
                    "nodes": [ {nodes} ],
                    "edges": [ {} ]
                }}"#,
                edges.join(",")
            )
        };
        let s1 = parse_and_resolve(&mk(&[("a", "b"), ("b", "c"), ("a", "c")])).unwrap();
        let s2 = parse_and_resolve(&mk(&[("a", "c"), ("b", "c"), ("a", "b")])).unwrap();
        let coords = |s: &Scene| {
            let mut v: Vec<_> = s
                .nodes
                .iter()
                .map(|n| (n.id.clone(), n.x as i64, n.y as i64))
                .collect();
            v.sort();
            v
        };
        assert_eq!(coords(&s1), coords(&s2));
    }

    #[test]
    fn fixed_fit_too_small_is_error() {
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto", "fit": "fixed" }},
                "canvas": {{ "w": 10, "h": 10 }},
                "nodes": [
                    {{ "id": "a", "label": "AAAAAA", "shape": "rect" }},
                    {{ "id": "b", "label": "BBBBBB", "shape": "rect" }}
                ],
                "edges": [ {{ "id": "e1", "from": "a", "to": "b" }} ]
            }}"#
        );
        assert!(parse_and_resolve(&json).is_err());
    }

    #[test]
    fn unknown_field_rejected() {
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto" }},
                "noddes": []
            }}"#
        );
        assert!(parse_and_resolve(&json).is_err());
    }

    #[test]
    fn auto_mode_rejects_unknown_edge_endpoint() {
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto" }},
                "nodes": [ {{ "id": "a", "label": "A", "shape": "rect" }} ],
                "edges": [ {{ "id": "e1", "from": "a", "to": "ghost" }} ]
            }}"#
        );
        let err = parse_and_resolve(&json).unwrap_err();
        assert!(err.to_string().contains("ghost"), "got {err}");
    }

    #[test]
    fn auto_mode_rejects_duplicate_node_id() {
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto" }},
                "nodes": [
                    {{ "id": "a", "label": "A", "shape": "rect" }},
                    {{ "id": "a", "label": "B", "shape": "rect" }}
                ]
            }}"#
        );
        let err = parse_and_resolve(&json).unwrap_err();
        assert!(err.to_string().contains("duplicate"), "got {err}");
    }

    #[test]
    fn auto_mode_rejects_nan_and_negative_layout_params() {
        // Negative spacing.
        let neg = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto", "node_sep": -5 }},
                "nodes": [ {{ "id": "a", "label": "A", "shape": "rect" }} ]
            }}"#
        );
        assert!(parse_and_resolve(&neg).is_err());
        // Absurdly large margin (over the cap).
        let huge = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto", "margin": 1000000 }},
                "nodes": [ {{ "id": "a", "label": "A", "shape": "rect" }} ]
            }}"#
        );
        assert!(parse_and_resolve(&huge).is_err());
    }

    #[test]
    fn parallel_edges_both_resolve() {
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto" }},
                "nodes": [
                    {{ "id": "a", "label": "A", "shape": "rect" }},
                    {{ "id": "b", "label": "B", "shape": "rect" }}
                ],
                "edges": [
                    {{ "id": "e1", "from": "a", "to": "b", "label": "first" }},
                    {{ "id": "e2", "from": "a", "to": "b", "label": "second" }}
                ]
            }}"#
        );
        let scene = parse_and_resolve(&json).unwrap();
        assert_eq!(scene.edges.len(), 2);
        let labels: Vec<_> = scene.edges.iter().filter_map(|e| e.label.clone()).collect();
        assert!(labels.contains(&"first".to_string()));
        assert!(labels.contains(&"second".to_string()));
    }

    // Helper: same two-node auto scene, with an optional overlay block.
    fn auto_two_nodes(overlay: &str) -> String {
        format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto", "fit": "grow" }},
                "nodes": [
                    {{ "id": "a", "label": "A", "shape": "rect" }},
                    {{ "id": "b", "label": "B", "shape": "rect" }}
                ],
                "edges": [ {{ "id": "e1", "from": "a", "to": "b" }} ]{overlay}
            }}"#
        )
    }

    #[test]
    fn grow_canvas_includes_text_below_graph() {
        let base = parse_and_resolve(&auto_two_nodes("")).unwrap();
        // Place a text far below the graph; the canvas must grow to cover it.
        let far_y = base.canvas.h as f32 + 500.0;
        let overlay = format!(
            r#", "texts": [ {{ "id": "note", "text": "annotation", "x": 10, "y": {far_y} }} ]"#
        );
        let grown = parse_and_resolve(&auto_two_nodes(&overlay)).unwrap();
        assert!(
            grown.canvas.h > base.canvas.h,
            "canvas should grow to fit text below the graph: base={}, grown={}",
            base.canvas.h,
            grown.canvas.h
        );
        // The text's drawn bottom must fit inside the grown canvas.
        let (_, bottom) = crate::render::free_text_extent(&grown.texts[0]);
        assert!(
            bottom <= grown.canvas.h as f32,
            "text bottom {bottom} must fit canvas h {}",
            grown.canvas.h
        );
    }

    #[test]
    fn grow_canvas_includes_zone_below_graph() {
        let base = parse_and_resolve(&auto_two_nodes("")).unwrap();
        let far_y = base.canvas.h as f32 + 300.0;
        let overlay = format!(
            r#", "zones": [ {{ "id": "z", "label": "grp", "x": 10, "y": {far_y}, "w": 100, "h": 80 }} ]"#
        );
        let grown = parse_and_resolve(&auto_two_nodes(&overlay)).unwrap();
        assert!(
            grown.canvas.h > base.canvas.h,
            "canvas should grow for zone"
        );
        let z = &grown.zones[0];
        assert!(
            z.y + z.h <= grown.canvas.h as f32,
            "zone bottom must fit canvas"
        );
    }

    #[test]
    fn fixed_fit_ignores_text_overflow() {
        // fit:fixed must NOT error just because a manual text spills past the
        // canvas; only the laid-out graph is measured for the fit check.
        let json = format!(
            r#"{{
                "schema_version": "{V}",
                "layout": {{ "mode": "auto", "fit": "fixed" }},
                "canvas": {{ "w": 4000, "h": 4000 }},
                "nodes": [
                    {{ "id": "a", "label": "A", "shape": "rect" }},
                    {{ "id": "b", "label": "B", "shape": "rect" }}
                ],
                "edges": [ {{ "id": "e1", "from": "a", "to": "b" }} ],
                "texts": [ {{ "id": "note", "text": "far", "x": 10, "y": 9000 }} ]
            }}"#
        );
        // 9000 > canvas 4000, but fixed ignores texts -> ok.
        assert!(parse_and_resolve(&json).is_ok());
    }
}
