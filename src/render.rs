//! Scene -> SVG document rendering.
//!
//! Draw order is background -> zones -> edges -> nodes -> edge labels -> texts
//! so that nodes sit on top of edges (sketchy jitter on lines would otherwise
//! poke over the boxes) and all text sits on top of everything.
//!
//! All text is emitted as `<text>` (never hand-drawn). Every user-supplied
//! string is XML-escaped and every color is validated to `#hex`, which keeps
//! the local-only invariant: no `url(...)`, `<image>`, or external reference
//! can leak into the output.

use crate::backend::{HanddrawBackend, RoughrBackend, Style};
use crate::scene::{Edge, Node, Point, Port, Scene, Shape, Text, role_palette, validate_color};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use unicode_width::UnicodeWidthStr;

/// Default label font family. Yomogi is a casual handwriting font embedded in
/// the binary (source at `assets/fonts`), so it is always available at runtime.
pub const DEFAULT_FONT_FAMILY: &str = "Yomogi";

const STROKE_WIDTH: f32 = 2.0;
/// Node label font size. Public so the auto-layout sizer ([`crate::layout`])
/// derives node boxes from the same value the renderer draws text with.
pub const LABEL_FONT_SIZE: f32 = 18.0;
const ZONE_LABEL_FONT_SIZE: f32 = 16.0;
const TEXT_FONT_SIZE: f32 = 18.0;
/// Edge label font size. Public for the same single-source-of-truth reason as
/// [`LABEL_FONT_SIZE`].
pub const EDGE_LABEL_FONT_SIZE: f32 = 15.0;
const ARROW_HEAD_LEN: f32 = 12.0;
const TEXT_COLOR: &str = "#222222";
const ZONE_STROKE: &str = "#9aa0a6";
const ZONE_LABEL_COLOR: &str = "#5f6368";

/// Escape the five XML special characters so user text cannot break out of an
/// element or attribute.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Stable per-element seed: hash of `scene.seed`, the element id, and a purpose
/// tag. Using the id (not a positional index) keeps a shape's sketch stable
/// when other elements are inserted before it, which makes diffs reviewable.
fn element_seed(scene_seed: u64, id: &str, purpose: &str) -> u64 {
    let mut h = DefaultHasher::new();
    scene_seed.hash(&mut h);
    id.hash(&mut h);
    purpose.hash(&mut h);
    h.finish()
}

/// Render a validated scene into a complete SVG document with the default font.
pub fn render_scene(scene: &Scene) -> String {
    render_scene_with_font(scene, DEFAULT_FONT_FAMILY)
}

/// Render a validated scene into a complete SVG document, using `font_family`
/// for all text. The font family is XML-escaped before being embedded.
///
/// Colors that fail validation fall back to neutral defaults rather than
/// emitting an attacker-controlled string, preserving the local-only invariant
/// even if a caller skipped [`Scene::validate`].
pub fn render_scene_with_font(scene: &Scene, font_family: &str) -> String {
    let backend = RoughrBackend::new();
    let font = xml_escape(font_family);
    let w = scene.canvas.w;
    let h = scene.canvas.h;

    let mut body = String::new();

    // Accessible document title (also surfaces in some SVG viewers as a tooltip).
    if !scene.title.is_empty() {
        body.push_str(&format!("<title>{}</title>\n", xml_escape(&scene.title)));
    }

    // Background.
    let bg = safe_color(&scene.canvas.background, "#ffffff");
    body.push_str(&format!(
        r#"<rect width="{w}" height="{h}" fill="{bg}"/>
"#
    ));

    // Zones (behind everything, light dashed boxes with a top-left label).
    for zone in &scene.zones {
        let seed = element_seed(scene.seed, &zone.id, "zone");
        let style = Style {
            stroke: ZONE_STROKE.to_string(),
            fill: None,
            stroke_width: 1.5,
            dashed: true,
        };
        body.push_str(&backend.rect(zone.x, zone.y, zone.w, zone.h, &style, seed));
        if !zone.label.is_empty() {
            // `text_element` centers on y; convert the intended baseline
            // (one line below the top-left corner) to that center.
            let baseline = zone.y + ZONE_LABEL_FONT_SIZE + 4.0;
            body.push_str(&text_element(
                zone.x + 8.0,
                baseline - ZONE_LABEL_FONT_SIZE * BASELINE_SHIFT,
                &zone.label,
                &font,
                ZONE_LABEL_FONT_SIZE,
                ZONE_LABEL_COLOR,
                "start",
            ));
        }
    }

    // Index nodes by id for edge endpoint resolution.
    let node_map: HashMap<&str, &Node> = scene.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    // Edges (under nodes).
    let mut edge_labels = String::new();
    for edge in &scene.edges {
        let (Some(from), Some(to)) = (
            node_map.get(edge.from.as_str()),
            node_map.get(edge.to.as_str()),
        ) else {
            continue; // validation guarantees presence; be defensive anyway.
        };
        render_edge(
            &backend,
            scene.seed,
            edge,
            from,
            to,
            &font,
            &mut body,
            &mut edge_labels,
        );
    }

    // Nodes (on top of edges).
    for node in &scene.nodes {
        let seed = element_seed(scene.seed, &node.id, "node");
        let (stroke, fill) = role_palette(node.role);
        let style = Style {
            stroke: stroke.to_string(),
            fill: Some(fill.to_string()),
            stroke_width: STROKE_WIDTH,
            dashed: false,
        };
        match node.shape {
            Shape::Rect => {
                body.push_str(&backend.rect(node.x, node.y, node.w, node.h, &style, seed))
            }
            Shape::Ellipse => {
                let c = node.center();
                body.push_str(&backend.ellipse(c.x, c.y, node.w, node.h, &style, seed));
            }
            Shape::Diamond => {
                body.push_str(&backend.diamond(node.x, node.y, node.w, node.h, &style, seed))
            }
        }
        if !node.label.is_empty() {
            let c = node.center();
            // `text_element` centers on the given y, so pass the box center.
            body.push_str(&text_element(
                c.x,
                c.y,
                &node.label,
                &font,
                LABEL_FONT_SIZE,
                TEXT_COLOR,
                "middle",
            ));
        }
    }

    // Edge labels (above nodes so they stay readable).
    body.push_str(&edge_labels);

    // Free-standing texts (topmost). The user supplies `t.y` as the baseline of
    // the first line (legacy behavior), so convert it to the center y that
    // `text_element` now expects, keeping single-line placement unchanged.
    for t in &scene.texts {
        body.push_str(&text_element(
            t.x,
            t.y - TEXT_FONT_SIZE * BASELINE_SHIFT,
            &t.text,
            &font,
            TEXT_FONT_SIZE,
            TEXT_COLOR,
            "start",
        ));
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
{body}</svg>
"#
    )
}

/// Render one edge: resolve endpoints, draw the (possibly multi-segment)
/// hand-drawn polyline, optional arrow head, and queue its label.
#[allow(clippy::too_many_arguments)]
fn render_edge(
    backend: &RoughrBackend,
    scene_seed: u64,
    edge: &Edge,
    from: &Node,
    to: &Node,
    font: &str,
    body: &mut String,
    labels: &mut String,
) {
    // Auto-layout path: when the resolver supplied a full boundary-to-boundary
    // route, draw it verbatim. Recomputing endpoints from node boundaries would
    // double-correct the layout engine's geometry. Otherwise fall back to the
    // manual path that derives endpoints from node boundaries + waypoints.
    let pts: Vec<(f32, f32)> = if let Some(route) = &edge.route {
        route.iter().map(|p| (p.x, p.y)).collect()
    } else {
        // The first point the edge heads toward when leaving `from` is the
        // first waypoint if present, otherwise the target center. Likewise for
        // the end.
        let first_target = edge.waypoints.first().copied().unwrap_or(to.center());
        let last_target = edge.waypoints.last().copied().unwrap_or(from.center());

        let start = match edge.from_port {
            Some(p) => port_point(from, p),
            None => boundary_point(from, first_target),
        };
        let end = match edge.to_port {
            Some(p) => port_point(to, p),
            None => boundary_point(to, last_target),
        };

        let mut pts: Vec<(f32, f32)> = Vec::with_capacity(edge.waypoints.len() + 2);
        pts.push((start.x, start.y));
        for wp in &edge.waypoints {
            pts.push((wp.x, wp.y));
        }
        pts.push((end.x, end.y));
        pts
    };
    if pts.len() < 2 {
        return; // Nothing drawable.
    }

    let style = Style {
        stroke: "#3a3a3a".to_string(),
        fill: None,
        stroke_width: STROKE_WIDTH,
        dashed: edge.dashed,
    };
    let seed = element_seed(scene_seed, &edge.id, "edge");
    body.push_str(&backend.polyline(&pts, &style, seed));

    // Arrow head at the end. Walk back to the last non-degenerate segment so a
    // zero-length final segment (e.g. a duplicated endpoint in a route) does
    // not erase an arrowhead whose direction is well defined earlier.
    if matches!(edge.style, crate::scene::EdgeStyle::Arrow) {
        let tip = pts[pts.len() - 1];
        let tail = pts[..pts.len() - 1]
            .iter()
            .rev()
            .copied()
            .find(|&p| (tip.0 - p.0).hypot(tip.1 - p.1) >= 1e-4);
        if let Some(tail) = tail
            && let Some(head) = arrow_head(tail, tip)
        {
            let head_seed = element_seed(scene_seed, &edge.id, "arrowhead");
            body.push_str(&backend.polyline(&head, &style, head_seed));
        }
    }

    // Label: prefer the auto-layout-computed anchor when present (the engine
    // already reserved space for it); otherwise place it along the path at
    // label_pos, offset by the manual (dx, dy).
    if let Some(label) = &edge.label
        && !label.is_empty()
    {
        let (lx, ly) = match edge.label_anchor {
            Some(a) => (a.x, a.y),
            None => {
                let pos = edge.label_pos.clamp(0.0, 1.0);
                let p = point_along(&pts, pos);
                (p.0 + edge.label_dx, p.1 + edge.label_dy)
            }
        };
        labels.push_str(&text_element(
            lx,
            ly,
            label,
            font,
            EDGE_LABEL_FONT_SIZE,
            TEXT_COLOR,
            "middle",
        ));
    }
}

/// Compute the boundary point of a node's shape along the ray from its center
/// toward `target`. Avoids division by zero when the target coincides with the
/// center by emitting a point on the right edge.
fn boundary_point(node: &Node, target: Point) -> Point {
    const EPS: f32 = 1e-4;
    let c = node.center();
    let rx = node.w / 2.0;
    let ry = node.h / 2.0;
    let dx = target.x - c.x;
    let dy = target.y - c.y;

    if dx.hypot(dy) < EPS || rx < EPS || ry < EPS {
        return Point {
            x: c.x + rx,
            y: c.y,
        };
    }

    let t = match node.shape {
        Shape::Rect => {
            let tx = if dx.abs() < EPS {
                f32::INFINITY
            } else {
                rx / dx.abs()
            };
            let ty = if dy.abs() < EPS {
                f32::INFINITY
            } else {
                ry / dy.abs()
            };
            tx.min(ty)
        }
        Shape::Ellipse => 1.0 / ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt(),
        Shape::Diamond => 1.0 / (dx.abs() / rx + dy.abs() / ry),
    };

    Point {
        x: c.x + dx * t,
        y: c.y + dy * t,
    }
}

/// Point at the midpoint of a node edge for an explicit port.
fn port_point(node: &Node, port: Port) -> Point {
    let c = node.center();
    match port {
        Port::N => Point { x: c.x, y: node.y },
        Port::S => Point {
            x: c.x,
            y: node.y + node.h,
        },
        Port::W => Point { x: node.x, y: c.y },
        Port::E => Point {
            x: node.x + node.w,
            y: c.y,
        },
    }
}

/// Build a small two-segment arrow head at `tip`, pointing away from `tail`.
/// Returns `None` if the segment has zero length.
fn arrow_head(tail: (f32, f32), tip: (f32, f32)) -> Option<[(f32, f32); 3]> {
    let dx = tip.0 - tail.0;
    let dy = tip.1 - tail.1;
    let len = dx.hypot(dy);
    if len < 1e-4 {
        return None;
    }
    let (ux, uy) = (dx / len, dy / len);
    // Two barbs rotated +/- ~25 degrees from the reverse direction.
    let angle = 0.45_f32;
    let (ca, sa) = (angle.cos(), angle.sin());
    let rev = (-ux, -uy);
    let left = (
        tip.0 + ARROW_HEAD_LEN * (rev.0 * ca - rev.1 * sa),
        tip.1 + ARROW_HEAD_LEN * (rev.0 * sa + rev.1 * ca),
    );
    let right = (
        tip.0 + ARROW_HEAD_LEN * (rev.0 * ca + rev.1 * sa),
        tip.1 + ARROW_HEAD_LEN * (-rev.0 * sa + rev.1 * ca),
    );
    Some([left, tip, right])
}

/// Interpolate a point at fraction `t` along a polyline by arc length.
fn point_along(pts: &[(f32, f32)], t: f32) -> (f32, f32) {
    if pts.is_empty() {
        return (0.0, 0.0);
    }
    if pts.len() == 1 {
        return pts[0];
    }
    let total: f32 = pts
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum();
    if total < 1e-4 {
        return pts[0];
    }
    let target = total * t;
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let seg = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        if acc + seg >= target {
            let local = if seg < 1e-4 {
                0.0
            } else {
                (target - acc) / seg
            };
            return (
                w[0].0 + (w[1].0 - w[0].0) * local,
                w[0].1 + (w[1].1 - w[0].1) * local,
            );
        }
        acc += seg;
    }
    *pts.last().unwrap()
}

/// Line height as a multiple of the font size, shared by sizing and rendering
/// of multi-line labels.
pub const LINE_HEIGHT_FACTOR: f32 = 1.33;

/// Baseline shift from a line's visual vertical center, as a fraction of the
/// font size. SVG `<text>` positions by baseline, so to center a line on a
/// given `y` the baseline must sit this much below `y` (approximates the
/// cap-height/descent split without precise font metrics).
const BASELINE_SHIFT: f32 = 0.36;

/// Approximate maximum glyph advance as a fraction of the font size, used only
/// to *over*-estimate a free-standing text's drawn width for canvas fitting
/// (the renderer leaves actual advances to the SVG font). Intentionally
/// generous: over-estimation just pads the canvas, under-estimation would clip.
const TEXT_ADVANCE: f32 = 0.62;

/// Estimate the bottom-right extent (max x, max y in canvas coordinates) a
/// free-standing [`Text`] occupies when drawn by [`render_scene`]. Used by the
/// auto-layout `fit: "grow"` path to grow the canvas around manual annotations
/// so they are not clipped. Mirrors the free-standing-text drawing math
/// (anchor `start`, `t.y` as the first baseline, `\n` multi-line) and rounds
/// up, so the result never under-covers the real glyphs.
///
/// Width is approximated from `unicode-width` columns times [`TEXT_ADVANCE`];
/// this can over-estimate (safe: extra padding) but not clip.
#[must_use]
pub fn free_text_extent(text: &Text) -> (f32, f32) {
    let lines: Vec<&str> = text.text.split('\n').collect();
    let n = lines.len().max(1) as f32;
    let cols = lines.iter().map(|l| l.width()).max().unwrap_or(0) as f32;

    // Right edge: anchor is "start", so glyphs extend right from `t.x`.
    let right = text.x + cols * TEXT_FONT_SIZE * TEXT_ADVANCE;

    // Vertical: render uses center = t.y - size*BASELINE_SHIFT, then stacks
    // lines by line_h around that center. The last line's baseline plus a
    // descender allowance is the lowest drawn pixel.
    let line_h = TEXT_FONT_SIZE * LINE_HEIGHT_FACTOR;
    let center = text.y - TEXT_FONT_SIZE * BASELINE_SHIFT;
    let last_baseline = center + (n - 1.0) / 2.0 * line_h + TEXT_FONT_SIZE * BASELINE_SHIFT;
    // Allow a descender below the baseline (fraction of font size).
    let bottom = last_baseline + TEXT_FONT_SIZE * 0.3;

    (right.ceil(), bottom.ceil())
}

/// Emit a `<text>` element with escaped content and a validated fill color.
///
/// `y` is treated as the *visual vertical center* of the whole label,
/// regardless of line count, so single- and multi-line labels share one
/// reference point. A `\n` in `content` produces multiple lines stacked with
/// [`LINE_HEIGHT_FACTOR`] spacing and centered on `y`. Each line is XML-escaped.
#[allow(clippy::too_many_arguments)]
fn text_element(
    x: f32,
    y: f32,
    content: &str,
    font: &str,
    size: f32,
    fill: &str,
    anchor: &str,
) -> String {
    let fill = safe_color(fill, TEXT_COLOR);
    let lines: Vec<&str> = content.split('\n').collect();
    let line_h = size * LINE_HEIGHT_FACTOR;
    let n = lines.len() as f32;
    // Center the block on `y`: the vertical center of line `i` is offset from
    // `y` by `(i - (n-1)/2) * line_h`; the baseline sits BASELINE_SHIFT*size
    // below that center.
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let center = y + (i as f32 - (n - 1.0) / 2.0) * line_h;
        let ly = center + size * BASELINE_SHIFT;
        out.push_str(&format!(
            r#"<text x="{x}" y="{ly}" font-family="{font}" font-size="{size}" fill="{fill}" text-anchor="{anchor}">{}</text>
"#,
            xml_escape(line)
        ));
    }
    out
}

/// Return `color` if it is a valid `#hex`, otherwise the fallback. Both inputs
/// are trusted hex literals, so no escaping is needed; this only guards against
/// an unvalidated user color reaching an SVG attribute.
fn safe_color(color: &str, fallback: &str) -> String {
    if validate_color(color, "color").is_ok() {
        color.to_string()
    } else {
        fallback.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Canvas, EdgeStyle, Node, Scene, Shape};

    fn node(id: &str, x: f32, y: f32) -> Node {
        Node {
            id: id.into(),
            label: "L".into(),
            shape: Shape::Rect,
            x,
            y,
            w: 100.0,
            h: 60.0,
            role: None,
        }
    }

    fn scene_with(nodes: Vec<Node>, edges: Vec<Edge>) -> Scene {
        Scene {
            schema_version: crate::scene::SCHEMA_VERSION.into(),
            title: String::new(),
            seed: 1,
            canvas: Canvas {
                w: 400,
                h: 300,
                background: "#ffffff".into(),
            },
            nodes,
            edges,
            zones: vec![],
            texts: vec![],
        }
    }

    #[test]
    fn escapes_xml_special_chars() {
        assert_eq!(
            xml_escape(r#"<a> & "b" 'c'"#),
            "&lt;a&gt; &amp; &quot;b&quot; &apos;c&apos;"
        );
    }

    #[test]
    fn render_is_deterministic() {
        let s = scene_with(vec![node("a", 10.0, 10.0), node("b", 200.0, 200.0)], vec![]);
        assert_eq!(render_scene(&s), render_scene(&s));
    }

    #[test]
    fn coincident_centers_do_not_panic() {
        // Two nodes sharing the exact same box -> centers coincide.
        let edge = Edge {
            id: "e".into(),
            from: "a".into(),
            to: "b".into(),
            from_port: None,
            to_port: None,
            style: EdgeStyle::Arrow,
            dashed: false,
            waypoints: vec![],
            label: Some("x".into()),
            label_pos: 0.5,
            label_dx: 0.0,
            label_dy: 0.0,
            route: None,
            label_anchor: None,
        };
        let s = scene_with(
            vec![node("a", 50.0, 50.0), node("b", 50.0, 50.0)],
            vec![edge],
        );
        let svg = render_scene(&s);
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn user_label_is_escaped_in_output() {
        let mut n = node("a", 10.0, 10.0);
        n.label = "<script>".into();
        let s = scene_with(vec![n], vec![]);
        let svg = render_scene(&s);
        assert!(svg.contains("&lt;script&gt;"));
        assert!(!svg.contains("<script>"));
    }

    #[test]
    fn boundary_point_on_rect_edge() {
        let n = node("a", 0.0, 0.0); // box 0,0..100,60 center (50,30)
        // Target far to the right -> boundary on right edge x=100.
        let p = boundary_point(&n, Point { x: 1000.0, y: 30.0 });
        assert!((p.x - 100.0).abs() < 0.5, "x={}", p.x);
        assert!((p.y - 30.0).abs() < 0.5, "y={}", p.y);
    }

    fn routed_edge(route: Vec<Point>) -> Edge {
        Edge {
            id: "e".into(),
            from: "a".into(),
            to: "b".into(),
            from_port: None,
            to_port: None,
            style: EdgeStyle::Arrow,
            dashed: false,
            waypoints: vec![],
            label: None,
            label_pos: 0.5,
            label_dx: 0.0,
            label_dy: 0.0,
            route: Some(route),
            label_anchor: None,
        }
    }

    fn path_count(svg: &str) -> usize {
        svg.matches("<path").count()
    }

    #[test]
    fn degenerate_final_segment_keeps_arrowhead() {
        // A route whose last two points coincide still has a well-defined
        // direction earlier; the arrowhead must not vanish. Compare the path
        // count against a clean route with the same direction.
        let clean = routed_edge(vec![Point { x: 0.0, y: 0.0 }, Point { x: 100.0, y: 0.0 }]);
        let degenerate = routed_edge(vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 100.0, y: 0.0 },
            Point { x: 100.0, y: 0.0 }, // duplicated endpoint -> zero-length seg
        ]);
        let s_clean = scene_with(
            vec![node("a", 0.0, 0.0), node("b", 100.0, 0.0)],
            vec![clean],
        );
        let s_deg = scene_with(
            vec![node("a", 0.0, 0.0), node("b", 100.0, 0.0)],
            vec![degenerate],
        );
        assert_eq!(
            path_count(&render_scene(&s_clean)),
            path_count(&render_scene(&s_deg)),
            "arrowhead must survive a zero-length final segment"
        );
    }

    #[test]
    fn arrow_head_direction_is_well_defined() {
        // Sanity: a horizontal segment yields a 3-point head.
        let head = arrow_head((0.0, 0.0), (10.0, 0.0)).expect("head");
        assert_eq!(head.len(), 3);
        assert_eq!(head[1], (10.0, 0.0)); // tip is the segment end
    }

    #[test]
    fn multiline_node_label_is_vertically_centered() {
        // A two-line label centered on the box center: one baseline above the
        // center, one below, symmetric about it.
        let mut n = node("a", 0.0, 0.0); // center (50, 30)
        n.label = "top\nbottom".into();
        let s = scene_with(vec![n], vec![]);
        let svg = render_scene(&s);
        // Extract the y of each label <text>.
        let ys: Vec<f32> = svg
            .match_indices("<text")
            .filter_map(|(i, _)| {
                let seg = &svg[i..];
                let yi = seg.find("y=\"")? + 3;
                let end = seg[yi..].find('"')?;
                seg[yi..yi + end].parse::<f32>().ok()
            })
            .collect();
        assert_eq!(ys.len(), 2, "two lines -> two <text> elements");
        let center = 30.0; // box center y
        let mid = (ys[0] + ys[1]) / 2.0;
        // Midpoint of the two baselines should be near the center plus the
        // single-line baseline shift (block is centered on the box center).
        assert!(
            (mid - (center + LABEL_FONT_SIZE * BASELINE_SHIFT)).abs() < 1.0,
            "lines not centered: ys={ys:?}, mid={mid}"
        );
    }
}
