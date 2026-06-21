//! Scene schema (the agent-facing "mouth"): a declarative description of a
//! diagram. Designed so an agent can write minimal JSON thanks to serde
//! defaults, while `validate` rejects anything that would break rendering or
//! the local-only invariant.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;

/// Schema version string expected in [`Scene::schema_version`].
pub const SCHEMA_VERSION: &str = "ponchi.scene/v0";

/// Upper bounds that keep a malicious or buggy scene from exhausting memory.
/// A huge canvas could slip past the HTTP body-size limit yet still blow up
/// when allocating the pixmap, so the canvas is bounded explicitly.
pub const MAX_CANVAS_DIM: u32 = 8192;
const MAX_NODES: usize = 2000;
const MAX_EDGES: usize = 4000;
const MAX_ZONES: usize = 500;
const MAX_TEXTS: usize = 2000;
const MAX_WAYPOINTS: usize = 64;
const MAX_STRING_LEN: usize = 4096;

/// A point in canvas coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// Node shape kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    Rect,
    Ellipse,
    Diamond,
}

/// Connection port on a node boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Port {
    N,
    E,
    S,
    W,
}

/// Edge rendering style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeStyle {
    #[default]
    Arrow,
    Line,
}

/// Semantic role of a node; drives the color palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Input,
    Process,
    External,
    Persistence,
    Transformation,
}

/// Canvas dimensions and background color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub w: u32,
    pub h: u32,
    #[serde(default = "default_background")]
    pub background: String,
}

/// Default canvas background color (`#ffffff`), shared with the input schema.
pub fn default_background() -> String {
    "#ffffff".to_string()
}

/// A diagram node (box/ellipse/diamond) with an optional semantic role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub shape: Shape,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub role: Option<Role>,
}

impl Node {
    /// Center point of the node's bounding box.
    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.w / 2.0,
            y: self.y + self.h / 2.0,
        }
    }
}

/// An edge connecting two nodes by id, optionally via waypoints, with an
/// optional label positioned along the path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
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
    pub waypoints: Vec<Point>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "default_label_pos")]
    pub label_pos: f32,
    #[serde(default)]
    pub label_dx: f32,
    #[serde(default)]
    pub label_dy: f32,
    /// Explicit, fully-resolved boundary-to-boundary route (including both
    /// endpoints). Populated only by the auto-layout resolver: when present,
    /// the renderer draws this polyline verbatim instead of recomputing
    /// endpoints from node boundaries (avoids double-correcting the layout
    /// engine's geometry). Not part of the input schema (`#[serde(skip)]`).
    #[serde(skip)]
    pub route: Option<Vec<Point>>,
    /// Explicit label anchor (auto-layout: the engine-computed label position).
    /// When present, overrides the `label_pos`/`label_dx`/`label_dy` placement.
    #[serde(skip)]
    pub label_anchor: Option<Point>,
}

fn default_label_pos() -> f32 {
    0.5
}

/// A labelled rectangular grouping drawn behind nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A free-standing text annotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Text {
    pub id: String,
    pub text: String,
    pub x: f32,
    pub y: f32,
}

/// The full declarative scene.
///
/// Element ids are unique within their own kind (nodes, edges, zones, texts),
/// not globally; edges reference node ids only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub schema_version: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub seed: u64,
    pub canvas: Canvas,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub zones: Vec<Zone>,
    #[serde(default)]
    pub texts: Vec<Text>,
}

/// Validation error for a scene that cannot be safely rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError(pub String);

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scene validation failed: {}", self.0)
    }
}

impl std::error::Error for ValidationError {}

impl Scene {
    /// Parse and validate a scene from a JSON string.
    pub fn from_json(s: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let scene: Scene = serde_json::from_str(s)?;
        scene.validate()?;
        Ok(scene)
    }

    /// Validate structural and safety invariants that serde alone cannot
    /// enforce: schema version, id uniqueness, edge references, finite/bounded
    /// coordinates, canvas size, element counts, string lengths, and color
    /// format. Keeping this strict protects the local-only invariant (no raw
    /// strings leak unescaped into the SVG) and bounds memory use.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError(format!(
                "unexpected schema_version {:?}, expected {:?}",
                self.schema_version, SCHEMA_VERSION
            )));
        }

        // Canvas bounds: a positive, capped pixmap allocation.
        if self.canvas.w == 0 || self.canvas.h == 0 {
            return Err(ValidationError("canvas dimensions must be > 0".into()));
        }
        if self.canvas.w > MAX_CANVAS_DIM || self.canvas.h > MAX_CANVAS_DIM {
            return Err(ValidationError(format!(
                "canvas dimension exceeds max {MAX_CANVAS_DIM}"
            )));
        }
        validate_color(&self.canvas.background, "canvas.background")?;

        // Element-count caps.
        check_count(self.nodes.len(), MAX_NODES, "nodes")?;
        check_count(self.edges.len(), MAX_EDGES, "edges")?;
        check_count(self.zones.len(), MAX_ZONES, "zones")?;
        check_count(self.texts.len(), MAX_TEXTS, "texts")?;

        // Id uniqueness across nodes (edges reference these) and within each
        // other element kind. Node ids form the edge address space.
        let mut node_ids: HashSet<&str> = HashSet::new();
        for n in &self.nodes {
            validate_id(&n.id, "node")?;
            if !node_ids.insert(n.id.as_str()) {
                return Err(ValidationError(format!("duplicate node id {:?}", n.id)));
            }
            validate_string(&n.label, "node.label")?;
            validate_finite_rect(n.x, n.y, n.w, n.h, &n.id)?;
        }

        let mut edge_ids: HashSet<&str> = HashSet::new();
        for e in &self.edges {
            validate_id(&e.id, "edge")?;
            if !edge_ids.insert(e.id.as_str()) {
                return Err(ValidationError(format!("duplicate edge id {:?}", e.id)));
            }
            if !node_ids.contains(e.from.as_str()) {
                return Err(ValidationError(format!(
                    "edge {:?} references unknown from node {:?}",
                    e.id, e.from
                )));
            }
            if !node_ids.contains(e.to.as_str()) {
                return Err(ValidationError(format!(
                    "edge {:?} references unknown to node {:?}",
                    e.id, e.to
                )));
            }
            if let Some(label) = &e.label {
                validate_string(label, "edge.label")?;
            }
            check_count(e.waypoints.len(), MAX_WAYPOINTS, "edge.waypoints")?;
            for (i, wp) in e.waypoints.iter().enumerate() {
                validate_finite(wp.x, &format!("edge {:?} waypoint {i} x", e.id))?;
                validate_finite(wp.y, &format!("edge {:?} waypoint {i} y", e.id))?;
            }
            if !e.label_pos.is_finite() {
                return Err(ValidationError(format!(
                    "edge {:?} label_pos not finite",
                    e.id
                )));
            }
            validate_finite(e.label_dx, &format!("edge {:?} label_dx", e.id))?;
            validate_finite(e.label_dy, &format!("edge {:?} label_dy", e.id))?;
            // Auto-layout-populated geometry (skipped by serde, so only ever
            // set internally) must still be finite to keep rendering safe.
            if let Some(route) = &e.route {
                // A degenerate route (empty or single point) cannot be drawn as
                // a polyline; the renderer would silently drop the edge and its
                // label. Reject it so the resolver can never emit one.
                if route.len() < 2 {
                    return Err(ValidationError(format!(
                        "edge {:?} route must have at least 2 points, got {}",
                        e.id,
                        route.len()
                    )));
                }
                check_count(route.len(), MAX_WAYPOINTS, "edge.route")?;
                for (i, p) in route.iter().enumerate() {
                    validate_finite(p.x, &format!("edge {:?} route {i} x", e.id))?;
                    validate_finite(p.y, &format!("edge {:?} route {i} y", e.id))?;
                }
            }
            if let Some(anchor) = &e.label_anchor {
                validate_finite(anchor.x, &format!("edge {:?} label_anchor x", e.id))?;
                validate_finite(anchor.y, &format!("edge {:?} label_anchor y", e.id))?;
            }
        }

        let mut zone_ids: HashSet<&str> = HashSet::new();
        for z in &self.zones {
            validate_id(&z.id, "zone")?;
            if !zone_ids.insert(z.id.as_str()) {
                return Err(ValidationError(format!("duplicate zone id {:?}", z.id)));
            }
            validate_string(&z.label, "zone.label")?;
            validate_finite_rect(z.x, z.y, z.w, z.h, &z.id)?;
        }

        let mut text_ids: HashSet<&str> = HashSet::new();
        for t in &self.texts {
            validate_id(&t.id, "text")?;
            if !text_ids.insert(t.id.as_str()) {
                return Err(ValidationError(format!("duplicate text id {:?}", t.id)));
            }
            validate_string(&t.text, "text.text")?;
            validate_finite(t.x, &format!("text {:?} x", t.id))?;
            validate_finite(t.y, &format!("text {:?} y", t.id))?;
        }

        validate_string(&self.title, "title")?;
        Ok(())
    }
}

fn check_count(n: usize, max: usize, what: &str) -> Result<(), ValidationError> {
    if n > max {
        return Err(ValidationError(format!("too many {what}: {n} > {max}")));
    }
    Ok(())
}

fn validate_id(id: &str, kind: &str) -> Result<(), ValidationError> {
    if id.is_empty() {
        return Err(ValidationError(format!("{kind} id must not be empty")));
    }
    if id.len() > MAX_STRING_LEN {
        return Err(ValidationError(format!("{kind} id too long")));
    }
    Ok(())
}

fn validate_string(s: &str, what: &str) -> Result<(), ValidationError> {
    if s.len() > MAX_STRING_LEN {
        return Err(ValidationError(format!(
            "{what} too long ({} bytes)",
            s.len()
        )));
    }
    Ok(())
}

fn validate_finite(v: f32, what: &str) -> Result<(), ValidationError> {
    if !v.is_finite() {
        return Err(ValidationError(format!("{what} must be finite")));
    }
    Ok(())
}

/// Validate a rectangle's coordinates are finite and its size is strictly
/// positive (a degenerate `w<=0` or `h<=0` box has no sensible boundary or
/// fill, and would render as nothing or NaN).
fn validate_finite_rect(x: f32, y: f32, w: f32, h: f32, id: &str) -> Result<(), ValidationError> {
    validate_finite(x, &format!("{id} x"))?;
    validate_finite(y, &format!("{id} y"))?;
    validate_finite(w, &format!("{id} w"))?;
    validate_finite(h, &format!("{id} h"))?;
    if w <= 0.0 || h <= 0.0 {
        return Err(ValidationError(format!("{id} w/h must be > 0")));
    }
    Ok(())
}

/// Accept only `#rgb`, `#rrggbb`, or `#rrggbbaa` hex colors. This keeps raw,
/// attacker-controlled strings out of SVG attributes (defends the local-only
/// invariant: no `url(...)` or external refs can slip through a color field).
pub fn validate_color(c: &str, what: &str) -> Result<(), ValidationError> {
    let ok = c.strip_prefix('#').is_some_and(|hex| {
        matches!(hex.len(), 3 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
    });
    if ok {
        Ok(())
    } else {
        Err(ValidationError(format!(
            "{what} must be a #hex color, got {c:?}"
        )))
    }
}

/// Map a node role to `(stroke, fill)` hex colors. Unspecified roles get a
/// neutral palette.
pub fn role_palette(role: Option<Role>) -> (&'static str, &'static str) {
    match role {
        None => ("#3a3a3a", "#f6f6f6"),
        Some(Role::Input) => ("#1f6feb", "#dbeafe"),
        Some(Role::Process) => ("#9a6700", "#fff4d6"),
        Some(Role::External) => ("#6e40c9", "#ede7f6"),
        Some(Role::Persistence) => ("#1a7f37", "#dcfce7"),
        Some(Role::Transformation) => ("#cf222e", "#ffe3e3"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_scene() -> Scene {
        Scene {
            schema_version: SCHEMA_VERSION.to_string(),
            title: "t".into(),
            seed: 1,
            canvas: Canvas {
                w: 400,
                h: 300,
                background: "#ffffff".into(),
            },
            nodes: vec![Node {
                id: "a".into(),
                label: "A".into(),
                shape: Shape::Rect,
                x: 10.0,
                y: 10.0,
                w: 100.0,
                h: 50.0,
                role: Some(Role::Input),
            }],
            edges: vec![],
            zones: vec![],
            texts: vec![],
        }
    }

    #[test]
    fn serde_roundtrip() {
        let scene = minimal_scene();
        let json = serde_json::to_string(&scene).unwrap();
        let back: Scene = serde_json::from_str(&json).unwrap();
        assert_eq!(scene, back);
    }

    #[test]
    fn minimal_json_uses_defaults() {
        // Only required fields; defaults fill the rest.
        let json = r#"{
            "schema_version": "ponchi.scene/v0",
            "canvas": { "w": 100, "h": 100 },
            "nodes": [ { "id": "n1", "shape": "rect", "x": 0, "y": 0, "w": 10, "h": 10 } ]
        }"#;
        let scene = Scene::from_json(json).unwrap();
        assert_eq!(scene.canvas.background, "#ffffff");
        assert_eq!(scene.seed, 0);
        assert!(scene.nodes[0].role.is_none());
    }

    #[test]
    fn rejects_bad_schema_version() {
        let mut s = minimal_scene();
        s.schema_version = "nope".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_node_id() {
        let mut s = minimal_scene();
        let mut dup = s.nodes[0].clone();
        dup.label = "B".into();
        s.nodes.push(dup);
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_dangling_edge() {
        let mut s = minimal_scene();
        s.edges.push(Edge {
            id: "e1".into(),
            from: "a".into(),
            to: "ghost".into(),
            from_port: None,
            to_port: None,
            style: EdgeStyle::Arrow,
            dashed: false,
            waypoints: vec![],
            label: None,
            label_pos: 0.5,
            label_dx: 0.0,
            label_dy: 0.0,
            route: None,
            label_anchor: None,
        });
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_huge_canvas() {
        let mut s = minimal_scene();
        s.canvas.w = MAX_CANVAS_DIM + 1;
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        // A typo'd top-level key (e.g. "noddes") must error, not silently
        // produce an empty diagram.
        let json = r#"{
            "schema_version": "ponchi.scene/v0",
            "canvas": { "w": 100, "h": 100 },
            "noddes": []
        }"#;
        assert!(Scene::from_json(json).is_err());
        // Unknown nested key too.
        let json2 = r#"{
            "schema_version": "ponchi.scene/v0",
            "canvas": { "w": 100, "h": 100 },
            "nodes": [ { "id": "n1", "shape": "rect", "x": 0, "y": 0, "w": 10, "h": 10, "colour": "red" } ]
        }"#;
        assert!(Scene::from_json(json2).is_err());
    }

    #[test]
    fn rejects_degenerate_node_size() {
        let mut s = minimal_scene();
        s.nodes[0].w = 0.0;
        assert!(s.validate().is_err());
        let mut s2 = minimal_scene();
        s2.nodes[0].h = -1.0;
        assert!(s2.validate().is_err());
    }

    #[test]
    fn rejects_non_finite_label_offset() {
        let mut s = minimal_scene();
        s.edges.push(Edge {
            id: "e1".into(),
            from: "a".into(),
            to: "a".into(),
            from_port: None,
            to_port: None,
            style: EdgeStyle::Arrow,
            dashed: false,
            waypoints: vec![],
            label: Some("x".into()),
            label_pos: 0.5,
            label_dx: f32::NAN,
            label_dy: 0.0,
            route: None,
            label_anchor: None,
        });
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_degenerate_route() {
        // An internally-populated route with fewer than 2 points cannot be
        // drawn and must be rejected (defends the renderer's early return).
        let mut single = minimal_scene();
        single.edges.push(Edge {
            id: "e1".into(),
            from: "a".into(),
            to: "a".into(),
            from_port: None,
            to_port: None,
            style: EdgeStyle::Arrow,
            dashed: false,
            waypoints: vec![],
            label: None,
            label_pos: 0.5,
            label_dx: 0.0,
            label_dy: 0.0,
            route: Some(vec![Point { x: 1.0, y: 2.0 }]),
            label_anchor: None,
        });
        assert!(single.validate().is_err());

        // Two points is the minimum acceptable route.
        let mut ok = minimal_scene();
        ok.edges.push(Edge {
            id: "e1".into(),
            from: "a".into(),
            to: "a".into(),
            from_port: None,
            to_port: None,
            style: EdgeStyle::Arrow,
            dashed: false,
            waypoints: vec![],
            label: None,
            label_pos: 0.5,
            label_dx: 0.0,
            label_dy: 0.0,
            route: Some(vec![Point { x: 1.0, y: 2.0 }, Point { x: 3.0, y: 4.0 }]),
            label_anchor: None,
        });
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_color() {
        assert!(validate_color("red", "x").is_err());
        assert!(validate_color("#12g", "x").is_err());
        assert!(validate_color("url(#x)", "x").is_err());
        assert!(validate_color("#fff", "x").is_ok());
        assert!(validate_color("#ffffff", "x").is_ok());
        assert!(validate_color("#ffffffff", "x").is_ok());
    }

    #[test]
    fn role_palette_distinct_and_neutral_default() {
        let neutral = role_palette(None);
        let input = role_palette(Some(Role::Input));
        assert_ne!(neutral, input);
        // Every role yields a valid hex pair.
        for r in [
            None,
            Some(Role::Input),
            Some(Role::Process),
            Some(Role::External),
            Some(Role::Persistence),
            Some(Role::Transformation),
        ] {
            let (s, f) = role_palette(r);
            assert!(validate_color(s, "stroke").is_ok());
            assert!(validate_color(f, "fill").is_ok());
        }
    }
}
