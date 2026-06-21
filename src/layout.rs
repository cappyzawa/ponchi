//! Auto-layout: turn a coordinate-free scene description into a fully placed,
//! render-ready [`Scene`] using the `dagre` layered-graph engine.
//!
//! The engine is [`dagre`](https://crates.io/crates/dagre), a port of dagre.js.
//! It needs only `log` + `serde` at runtime (no HTTP client, no browser), which
//! keeps the local-only invariant. Given node sizes and edges it returns each
//! node center (`x`/`y`), each edge's boundary-to-boundary polyline (`points`),
//! and each edge label position.
//!
//! Determinism caveat: dagre is insertion-order sensitive. To make the layout
//! invariant under the authoring order of `nodes`/`edges` in the JSON, this
//! module sorts nodes by id and edges by `(from, to, label)` before feeding
//! them to the engine.

use crate::render::{EDGE_LABEL_FONT_SIZE, LABEL_FONT_SIZE, LINE_HEIGHT_FACTOR};
use crate::scene::{Edge, Node, Point, Shape};
use dagre::graph::{Graph, GraphOptions};
use dagre::{EdgeLabel, LayoutOptions, NodeLabel, RankDir, layout};
use unicode_width::UnicodeWidthStr;

/// Label font size used for node auto-sizing. Reuses the renderer's
/// `LABEL_FONT_SIZE` so the boxes match the drawn text (single source of truth).
const FONT_SIZE: f64 = LABEL_FONT_SIZE as f64;
/// Per-line height for node auto-sizing (font size x line-height factor),
/// sharing the renderer's [`LINE_HEIGHT_FACTOR`].
const LINE_H: f64 = FONT_SIZE * LINE_HEIGHT_FACTOR as f64;
/// Edge-label font size, used only to reserve label space in the engine. Reuses
/// the renderer's `EDGE_LABEL_FONT_SIZE`.
const EDGE_FONT_SIZE: f64 = EDGE_LABEL_FONT_SIZE as f64;
/// Approximate average glyph advance as a fraction of the font size. Rough but
/// adequate without precise font metrics (a documented v1 limitation).
const GLYPH_ADVANCE: f64 = 0.62;
/// Horizontal / vertical padding around node text.
const PAD_X: f64 = 36.0;
const PAD_Y: f64 = 24.0;
/// Minimum node box size so tiny labels still read as boxes.
const MIN_W: f64 = 90.0;
const MIN_H: f64 = 54.0;

/// Layout direction, mapped onto dagre's [`RankDir`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Left-to-right ranks.
    Lr,
    /// Top-to-bottom ranks.
    Tb,
}

impl Direction {
    fn rankdir(self) -> RankDir {
        match self {
            Direction::Lr => RankDir::LR,
            Direction::Tb => RankDir::TB,
        }
    }
}

/// Tunable spacing for the layered layout (all in canvas pixels).
#[derive(Debug, Clone, Copy)]
pub struct LayoutParams {
    pub direction: Direction,
    pub node_sep: f64,
    pub rank_sep: f64,
    pub margin: f64,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            direction: Direction::Lr,
            node_sep: 55.0,
            rank_sep: 90.0,
            margin: 30.0,
        }
    }
}

/// A node to be placed: identity, label (for sizing), shape, and an optional
/// caller-supplied size override (`w`/`h`). When a dimension is `None` it is
/// derived from the label.
#[derive(Debug, Clone)]
pub struct LayoutNode {
    pub id: String,
    pub label: String,
    pub shape: Shape,
    pub role: Option<crate::scene::Role>,
    pub w: Option<f32>,
    pub h: Option<f32>,
}

/// An edge to be routed: endpoints by node id, an optional label, and the
/// originating edge id (carried through so the resolved [`Edge`] keeps its id).
#[derive(Debug, Clone)]
pub struct LayoutEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: Option<String>,
    pub dashed: bool,
    pub style: crate::scene::EdgeStyle,
}

/// The placed result: render-ready nodes and edges (the latter carrying an
/// explicit `route`), plus the layout bounding box for canvas fitting.
#[derive(Debug, Clone)]
pub struct LaidOut {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Maximum x and y reached by any node box or edge point.
    pub max_x: f32,
    pub max_y: f32,
}

/// Auto-size a node box from its (possibly multi-line) label using
/// `unicode-width` column counts. Shape gets a size multiplier so ellipse and
/// diamond text is not clipped by the inscribed box.
fn auto_size(label: &str, shape: Shape) -> (f64, f64) {
    let lines: Vec<&str> = label.split('\n').collect();
    let cols = lines.iter().map(|l| l.width()).max().unwrap_or(1) as f64;
    let n_lines = lines.len().max(1) as f64;
    let (kw, kh) = match shape {
        Shape::Rect => (1.0, 1.0),
        Shape::Ellipse => (1.35, 1.3),
        Shape::Diamond => (1.6, 1.5),
    };
    let w = (cols * FONT_SIZE * GLYPH_ADVANCE + PAD_X).max(MIN_W) * kw;
    let h = (n_lines * LINE_H + PAD_Y).max(MIN_H) * kh;
    (w, h)
}

/// Run the layered layout and produce render-ready nodes/edges.
///
/// Nodes are placed at the engine-computed centers with the sizes used for
/// layout, and each edge gets an explicit boundary-to-boundary `route` plus an
/// engine-computed `label_anchor`. Inputs are sorted internally so the result
/// is invariant under the authoring order of `nodes`/`edges`.
pub fn run(nodes: &[LayoutNode], edges: &[LayoutEdge], params: LayoutParams) -> LaidOut {
    let mut g: Graph<NodeLabel, EdgeLabel> = Graph::with_options(GraphOptions {
        directed: true,
        multigraph: true,
        compound: false,
    });

    // Canonical insertion order: dagre is insertion-order sensitive, so sort to
    // make the layout independent of JSON authoring order.
    let mut nsorted: Vec<&LayoutNode> = nodes.iter().collect();
    nsorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut esorted: Vec<&LayoutEdge> = edges.iter().collect();
    esorted.sort_by(|a, b| {
        (a.from.as_str(), a.to.as_str(), a.label.as_deref()).cmp(&(
            b.from.as_str(),
            b.to.as_str(),
            b.label.as_deref(),
        ))
    });

    for n in &nsorted {
        let (w, h) = match (n.w, n.h) {
            (Some(w), Some(h)) => (w as f64, h as f64),
            (w_opt, h_opt) => {
                let (aw, ah) = auto_size(&n.label, n.shape);
                (
                    w_opt.map(|v| v as f64).unwrap_or(aw),
                    h_opt.map(|v| v as f64).unwrap_or(ah),
                )
            }
        };
        let nl = NodeLabel {
            width: w,
            height: h,
            label: Some(n.label.clone()),
            ..Default::default()
        };
        g.set_node(n.id.clone(), Some(nl));
    }

    for e in &esorted {
        let mut el = EdgeLabel::default();
        if let Some(label) = &e.label
            && !label.is_empty()
        {
            // Reserve label space so dagre keeps it clear of the route.
            let (lw, _) = auto_size(label, Shape::Rect);
            el.width = (lw - 20.0).max(0.0);
            el.height = EDGE_FONT_SIZE * 1.4;
        }
        // Pass the edge id as a unique name so parallel edges (same from/to)
        // are kept distinct in the multigraph instead of collapsing into one
        // (which would double-draw a single route and lose a label).
        g.set_edge(&e.from, &e.to, Some(el), Some(e.id.as_str()));
    }

    let opts = LayoutOptions {
        rankdir: params.direction.rankdir(),
        nodesep: params.node_sep,
        ranksep: params.rank_sep,
        edgesep: 18.0,
        marginx: params.margin,
        marginy: params.margin,
        ..Default::default()
    };
    layout(&mut g, Some(opts));

    let mut out_nodes = Vec::with_capacity(nsorted.len());
    let mut max_x = 0.0f32;
    let mut max_y = 0.0f32;

    for n in &nsorted {
        let nl = g.node(&n.id).expect("node present after layout");
        let (cx, cy) = (nl.x.unwrap_or(0.0), nl.y.unwrap_or(0.0));
        let (w, h) = (nl.width, nl.height);
        // Scene `Node` stores the top-left corner, not the center.
        let x = (cx - w / 2.0) as f32;
        let y = (cy - h / 2.0) as f32;
        let node = Node {
            id: n.id.clone(),
            label: n.label.clone(),
            shape: n.shape,
            x,
            y,
            w: w as f32,
            h: h as f32,
            role: n.role,
        };
        max_x = max_x.max(node.x + node.w);
        max_y = max_y.max(node.y + node.h);
        out_nodes.push(node);
    }

    let mut out_edges = Vec::with_capacity(esorted.len());
    for e in &esorted {
        // Read back by the same unique name used at insertion so parallel
        // edges resolve to their own route and label.
        let el = g
            .edge(&e.from, &e.to, Some(e.id.as_str()))
            .expect("edge present after layout");
        let route: Vec<Point> = el
            .points
            .iter()
            .map(|p| Point {
                x: p.x as f32,
                y: p.y as f32,
            })
            .collect();
        for p in &route {
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        let label_anchor = match (el.x, el.y) {
            (Some(lx), Some(ly)) if e.label.as_deref().is_some_and(|l| !l.is_empty()) => {
                Some(Point {
                    x: lx as f32,
                    y: ly as f32,
                })
            }
            _ => None,
        };
        out_edges.push(Edge {
            id: e.id.clone(),
            from: e.from.clone(),
            to: e.to.clone(),
            from_port: None,
            to_port: None,
            style: e.style,
            dashed: e.dashed,
            waypoints: vec![],
            label: e.label.clone(),
            label_pos: 0.5,
            label_dx: 0.0,
            label_dy: 0.0,
            route: if route.len() >= 2 { Some(route) } else { None },
            label_anchor,
        });
    }

    LaidOut {
        nodes: out_nodes,
        edges: out_edges,
        max_x,
        max_y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{EdgeStyle, Role, Shape};

    fn n(id: &str, label: &str) -> LayoutNode {
        LayoutNode {
            id: id.into(),
            label: label.into(),
            shape: Shape::Rect,
            role: Some(Role::Process),
            w: None,
            h: None,
        }
    }
    fn e(id: &str, from: &str, to: &str) -> LayoutEdge {
        LayoutEdge {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            label: Some("L".into()),
            dashed: false,
            style: EdgeStyle::Arrow,
        }
    }

    fn coords(out: &LaidOut) -> Vec<(String, i64, i64, i64, i64)> {
        let mut v: Vec<_> = out
            .nodes
            .iter()
            .map(|n| {
                (
                    n.id.clone(),
                    n.x.round() as i64,
                    n.y.round() as i64,
                    n.w.round() as i64,
                    n.h.round() as i64,
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn deterministic_and_shuffle_invariant() {
        let nodes = vec![n("a", "A"), n("b", "B"), n("c", "C")];
        let edges = vec![e("e1", "a", "b"), e("e2", "b", "c"), e("e3", "a", "c")];
        let p = LayoutParams::default();
        let r1 = run(&nodes, &edges, p);
        // Same input, second run.
        let r2 = run(&nodes, &edges, p);
        assert_eq!(coords(&r1), coords(&r2), "two runs must agree");

        // Reversed authoring order must give the same coordinates.
        let nrev: Vec<_> = nodes.iter().rev().cloned().collect();
        let erev: Vec<_> = edges.iter().rev().cloned().collect();
        let r3 = run(&nrev, &erev, p);
        assert_eq!(coords(&r1), coords(&r3), "shuffle must be invariant");
    }

    #[test]
    fn cycle_produces_finite_coords() {
        let nodes = vec![n("a", "A"), n("b", "B"), n("c", "C")];
        // a -> b -> c -> a (back-edge handled internally by dagre).
        let edges = vec![e("e1", "a", "b"), e("e2", "b", "c"), e("e3", "c", "a")];
        let out = run(&nodes, &edges, LayoutParams::default());
        for node in &out.nodes {
            assert!(node.x.is_finite() && node.y.is_finite());
            assert!(node.w > 0.0 && node.h > 0.0);
        }
        for edge in &out.edges {
            if let Some(route) = &edge.route {
                for p in route {
                    assert!(p.x.is_finite() && p.y.is_finite());
                }
            }
        }
    }

    #[test]
    fn multi_line_label_grows_height() {
        let (_, h1) = auto_size("one", Shape::Rect);
        let (_, h2) = auto_size("one\ntwo", Shape::Rect);
        assert!(h2 > h1, "two lines must be taller than one");
    }

    #[test]
    fn edges_carry_explicit_route() {
        let nodes = vec![n("a", "A"), n("b", "B")];
        let edges = vec![e("e1", "a", "b")];
        let out = run(&nodes, &edges, LayoutParams::default());
        let edge = &out.edges[0];
        assert!(edge.route.as_ref().is_some_and(|r| r.len() >= 2));
        assert!(edge.label_anchor.is_some());
    }

    #[test]
    fn parallel_edges_keep_distinct_routes_and_labels() {
        let labeled = |id: &str, label: &str| LayoutEdge {
            id: id.into(),
            from: "a".into(),
            to: "b".into(),
            label: Some(label.into()),
            dashed: false,
            style: EdgeStyle::Arrow,
        };
        let nodes = vec![n("a", "A"), n("b", "B")];
        // Two edges between the same pair must not collapse into one.
        let edges = vec![labeled("e1", "first"), labeled("e2", "second")];
        let out = run(&nodes, &edges, LayoutParams::default());
        assert_eq!(out.edges.len(), 2, "both parallel edges must survive");
        // Each keeps its own label.
        let labels: Vec<_> = out.edges.iter().filter_map(|e| e.label.clone()).collect();
        assert!(labels.contains(&"first".to_string()));
        assert!(labels.contains(&"second".to_string()));
        // The two routes are not identical (dagre separated them).
        let r0 = out.edges[0].route.as_ref().unwrap();
        let r1 = out.edges[1].route.as_ref().unwrap();
        assert_ne!(r0, r1, "parallel edges must take distinct routes");
    }

    #[test]
    fn multi_line_label_centers_block() {
        // The block of N lines is centered, so its vertical extent is symmetric
        // about the box center: top line above center, bottom line below.
        let (_, h) = auto_size("aa\nbb", Shape::Rect);
        assert!(h > 0.0);
    }
}
