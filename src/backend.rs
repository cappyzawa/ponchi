//! Hand-drawn shape backend: turns geometry into SVG `<path>` fragments with a
//! sketchy look. The [`HanddrawBackend`] trait keeps the rendering layer
//! independent of roughr so a fallback renderer could be swapped in.

use roughr::core::{Drawable, FillStyle, OpSet, OpSetType, OpType, OptionsBuilder};
use roughr::generator::Generator;
use roughr::{Point2D, Srgba};

/// Stroke/fill style for a hand-drawn shape, in `#hex` colors.
#[derive(Debug, Clone)]
pub struct Style {
    pub stroke: String,
    pub fill: Option<String>,
    pub stroke_width: f32,
    pub dashed: bool,
}

/// A backend that renders hand-drawn shapes into SVG fragment strings.
///
/// Each method takes a `seed` so the same scene element always produces the
/// same sketchy geometry (determinism).
pub trait HanddrawBackend {
    /// Axis-aligned rectangle at `(x, y)` with size `(w, h)`.
    fn rect(&self, x: f32, y: f32, w: f32, h: f32, style: &Style, seed: u64) -> String;
    /// Ellipse centered at `(cx, cy)` with full width/height `(w, h)`.
    fn ellipse(&self, cx: f32, cy: f32, w: f32, h: f32, style: &Style, seed: u64) -> String;
    /// Closed diamond inside the box `(x, y, w, h)`.
    fn diamond(&self, x: f32, y: f32, w: f32, h: f32, style: &Style, seed: u64) -> String;
    /// Open polyline through `points`. Stroke only (no fill).
    fn polyline(&self, points: &[(f32, f32)], style: &Style, seed: u64) -> String;
}

/// roughr-backed [`HanddrawBackend`].
#[derive(Default)]
pub struct RoughrBackend {
    generator: Generator,
}

impl RoughrBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Parse a `#rgb`/`#rrggbb`/`#rrggbbaa` hex color into an `Srgba`.
/// Falls back to opaque black if the string is malformed; callers are expected
/// to pass colors that already passed `scene::validate_color`.
fn parse_hex(c: &str) -> Srgba {
    let hex = c.strip_prefix('#').unwrap_or(c);
    let comp = |s: &str| u8::from_str_radix(s, 16).unwrap_or(0) as f32 / 255.0;
    match hex.len() {
        3 => {
            let r = comp(&hex[0..1].repeat(2));
            let g = comp(&hex[1..2].repeat(2));
            let b = comp(&hex[2..3].repeat(2));
            Srgba::new(r, g, b, 1.0)
        }
        6 => Srgba::new(comp(&hex[0..2]), comp(&hex[2..4]), comp(&hex[4..6]), 1.0),
        8 => Srgba::new(
            comp(&hex[0..2]),
            comp(&hex[2..4]),
            comp(&hex[4..6]),
            comp(&hex[6..8]),
        ),
        _ => Srgba::new(0.0, 0.0, 0.0, 1.0),
    }
}

/// `Srgba` (0..1 floats) -> `#rrggbb`.
fn hex(c: Srgba) -> String {
    let to = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", to(c.red), to(c.green), to(c.blue))
}

/// Convert one roughr [`OpSet`] into an SVG path `d` string.
///
/// roughr 0.12's own `ops_to_path` is buggy: it emits `L` for `OpType::Move`,
/// producing paths with no initial moveto that resvg refuses to render. We
/// build `d` ourselves: `Move -> M`, `LineTo -> L`, `BCurveTo -> C`. Ops whose
/// `data` is shorter than expected are skipped rather than panicking on a raw
/// index.
pub fn ops_to_d(set: &OpSet<f32>) -> String {
    let mut d = String::new();
    for op in &set.ops {
        match op.op {
            OpType::Move => {
                if op.data.len() >= 2 {
                    d.push_str(&format!("M{} {} ", op.data[0], op.data[1]));
                }
            }
            OpType::LineTo => {
                if op.data.len() >= 2 {
                    d.push_str(&format!("L{} {} ", op.data[0], op.data[1]));
                }
            }
            OpType::BCurveTo => {
                if op.data.len() >= 6 {
                    d.push_str(&format!(
                        "C{} {} {} {} {} {} ",
                        op.data[0], op.data[1], op.data[2], op.data[3], op.data[4], op.data[5]
                    ));
                }
            }
        }
    }
    d
}

/// Render a roughr [`Drawable`] into one or more `<path>` SVG fragments,
/// reproducing roughr's `to_paths` stroke/fill semantics per OpSet kind.
fn drawable_to_svg(drawable: &Drawable<f32>, dashed: bool) -> String {
    let o = &drawable.options;
    let dash_attr = if dashed {
        r#" stroke-dasharray="8 6""#
    } else {
        ""
    };
    let mut out = String::new();
    for set in &drawable.sets {
        let d = ops_to_d(set);
        if d.is_empty() {
            continue;
        }
        let (stroke, fill, sw) = match set.op_set_type {
            OpSetType::Path => (
                o.stroke.map(hex).unwrap_or_else(|| "none".into()),
                "none".to_string(),
                o.stroke_width.unwrap_or(1.0),
            ),
            OpSetType::FillPath => (
                "none".to_string(),
                o.fill.map(hex).unwrap_or_else(|| "none".into()),
                0.0,
            ),
            OpSetType::FillSketch => {
                let fw = o
                    .fill_weight
                    .filter(|w| *w > 0.0)
                    .unwrap_or(o.stroke_width.unwrap_or(1.0) / 2.0);
                (
                    o.fill.map(hex).unwrap_or_else(|| "none".into()),
                    "none".to_string(),
                    fw,
                )
            }
        };
        out.push_str(&format!(
            r#"<path d="{d}" stroke="{stroke}" stroke-width="{sw}" fill="{fill}" stroke-linecap="round" stroke-linejoin="round"{dash_attr}/>"#,
        ));
        out.push('\n');
    }
    out
}

impl RoughrBackend {
    /// Build roughr options for a filled shape with the given seed.
    fn fill_opts(&self, style: &Style, seed: u64) -> roughr::core::Options {
        let mut builder = OptionsBuilder::default();
        builder
            .seed(seed)
            .stroke(parse_hex(&style.stroke))
            .stroke_width(style.stroke_width);
        if let Some(fill) = &style.fill {
            builder
                .fill(parse_hex(fill))
                .fill_style(FillStyle::Hachure)
                .fill_weight(style.stroke_width / 2.0);
        }
        builder.build().expect("roughr options build")
    }

    /// Build roughr options for a stroke-only line with the given seed.
    fn line_opts(&self, style: &Style, seed: u64) -> roughr::core::Options {
        OptionsBuilder::default()
            .seed(seed)
            .stroke(parse_hex(&style.stroke))
            .stroke_width(style.stroke_width)
            .build()
            .expect("roughr options build")
    }
}

impl HanddrawBackend for RoughrBackend {
    fn rect(&self, x: f32, y: f32, w: f32, h: f32, style: &Style, seed: u64) -> String {
        let opts = self.fill_opts(style, seed);
        let d = self.generator.rectangle::<f32>(x, y, w, h, &Some(opts));
        drawable_to_svg(&d, style.dashed)
    }

    fn ellipse(&self, cx: f32, cy: f32, w: f32, h: f32, style: &Style, seed: u64) -> String {
        let opts = self.fill_opts(style, seed);
        let d = self.generator.ellipse::<f32>(cx, cy, w, h, &Some(opts));
        drawable_to_svg(&d, style.dashed)
    }

    fn diamond(&self, x: f32, y: f32, w: f32, h: f32, style: &Style, seed: u64) -> String {
        let opts = self.fill_opts(style, seed);
        let pts = [
            Point2D::new(x + w / 2.0, y),
            Point2D::new(x + w, y + h / 2.0),
            Point2D::new(x + w / 2.0, y + h),
            Point2D::new(x, y + h / 2.0),
        ];
        let d = self.generator.polygon::<f32>(&pts, &Some(opts));
        drawable_to_svg(&d, style.dashed)
    }

    fn polyline(&self, points: &[(f32, f32)], style: &Style, seed: u64) -> String {
        if points.len() < 2 {
            return String::new();
        }
        let opts = self.line_opts(style, seed);
        let pts: Vec<_> = points.iter().map(|&(x, y)| Point2D::new(x, y)).collect();
        let d = self.generator.linear_path::<f32>(&pts, false, &Some(opts));
        drawable_to_svg(&d, style.dashed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> Style {
        Style {
            stroke: "#222222".into(),
            fill: Some("#fff4d6".into()),
            stroke_width: 2.0,
            dashed: false,
        }
    }

    #[test]
    fn rect_path_starts_with_moveto() {
        // Regression guard for the roughr ops_to_path bug: stroke paths must
        // begin with an `M` (moveto), never `L`.
        let backend = RoughrBackend::new();
        let svg = backend.rect(10.0, 10.0, 100.0, 50.0, &style(), 42);
        assert!(svg.contains("<path"), "expected a path: {svg}");
        for d in extract_d_values(&svg) {
            assert!(
                d.trim_start().starts_with('M'),
                "path d must start with M, got: {d}"
            );
        }
    }

    #[test]
    fn deterministic_same_seed() {
        let backend = RoughrBackend::new();
        let a = backend.rect(0.0, 0.0, 80.0, 40.0, &style(), 7);
        let b = backend.rect(0.0, 0.0, 80.0, 40.0, &style(), 7);
        assert_eq!(a, b);
    }

    #[test]
    fn ops_to_d_emits_moveto() {
        // Build a small OpSet by hand and confirm Move -> M.
        use roughr::core::{Op, OpSetType, OpType};
        let set = OpSet::<f32> {
            op_set_type: OpSetType::Path,
            ops: vec![
                Op {
                    op: OpType::Move,
                    data: vec![1.0, 2.0],
                },
                Op {
                    op: OpType::LineTo,
                    data: vec![3.0, 4.0],
                },
            ],
            size: None,
            path: None,
        };
        let d = ops_to_d(&set);
        assert!(d.starts_with("M1 2"), "got {d}");
        assert!(d.contains("L3 4"));
    }

    fn extract_d_values(svg: &str) -> Vec<String> {
        svg.match_indices("d=\"")
            .filter_map(|(i, _)| {
                let rest = &svg[i + 3..];
                rest.find('"').map(|end| rest[..end].to_string())
            })
            .collect()
    }
}
