//! SVG -> PNG rasterization via usvg/resvg/tiny-skia, fully in-process (no
//! browser, no native cairo). Fonts come from the system plus the Yomogi
//! handwriting font embedded into the binary, with an optional extra font
//! directory loaded on top.

use std::path::Path;

/// The Yomogi handwriting font, embedded into the binary so a standalone
/// executable keeps its hand-drawn look without shipping `assets/fonts`
/// alongside it. The font is loaded at runtime; nothing is fetched over the
/// network (the local-only constraint). License: SIL OFL 1.1, see
/// `assets/fonts/Yomogi-OFL.txt`.
const BUNDLED_YOMOGI: &[u8] = include_bytes!("../assets/fonts/Yomogi-Regular.ttf");

/// Error rasterizing an SVG document.
#[derive(Debug)]
pub enum RasterError {
    Parse(String),
    Alloc,
    Encode(String),
    Io(std::io::Error),
}

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RasterError::Parse(e) => write!(f, "SVG parse error: {e}"),
            RasterError::Alloc => write!(f, "pixmap allocation failed"),
            RasterError::Encode(e) => write!(f, "PNG encode error: {e}"),
            RasterError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for RasterError {}

impl From<std::io::Error> for RasterError {
    fn from(e: std::io::Error) -> Self {
        RasterError::Io(e)
    }
}

/// A reusable rasterization context: the font database is loaded once
/// (scanning system fonts is the most expensive step) and reused across
/// renders. Build this once for a long-lived server; the standalone
/// [`svg_to_png`] helpers build a throwaway one for a single CLI render.
pub struct RasterContext {
    opt: usvg::Options<'static>,
}

impl RasterContext {
    /// Build a context with system fonts plus the embedded Yomogi font, and
    /// `extra_fonts_dir` (if any) loaded on top, using `font_family` as the
    /// default family for elements without one.
    ///
    /// The embedded Yomogi font is always available, so the default
    /// handwriting look does not depend on any file beside the executable.
    /// `extra_fonts_dir` is an optional directory of *additional* fonts to make
    /// selectable via `font_family`. It is additive only: a font there that
    /// reuses the embedded family name does not override the embedded Yomogi
    /// (the embedded copy loads first and wins `fontdb`'s best-match tie-break).
    /// To get a different look, choose a different `font_family`.
    ///
    /// Emits a stderr warning only if `font_family` does not resolve, since
    /// rendering would otherwise fall back to a different font (or tofu)
    /// silently.
    pub fn new(extra_fonts_dir: Option<&Path>, font_family: &str) -> Self {
        let mut opt = usvg::Options::default();
        opt.fontdb_mut().load_system_fonts();
        // Always load the embedded Yomogi font so the hand-drawn look works in a
        // standalone binary without `assets/fonts` present at runtime.
        opt.fontdb_mut().load_font_data(BUNDLED_YOMOGI.to_vec());
        // Optionally load extra fonts so other families can be selected via
        // `--font-family`.
        if let Some(dir) = extra_fonts_dir {
            opt.fontdb_mut().load_fonts_dir(dir);
        }
        // Default family so unspecified `font-family` still resolves to our font.
        opt.font_family = font_family.to_string();

        // Warn if the requested family is not present in the database.
        let has_family = opt
            .fontdb
            .faces()
            .any(|f| f.families.iter().any(|(name, _)| name == font_family));
        if !has_family {
            eprintln!(
                "warning: font family {font_family:?} not found in font database; \
                 text will fall back to another font"
            );
        }

        Self { opt }
    }

    /// Render an SVG document string to PNG bytes using the prepared fonts.
    pub fn render_png(&self, svg: &str) -> Result<Vec<u8>, RasterError> {
        let tree =
            usvg::Tree::from_str(svg, &self.opt).map_err(|e| RasterError::Parse(e.to_string()))?;
        let size = tree.size();
        let w = size.width().ceil() as u32;
        let h = size.height().ceil() as u32;
        let mut pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or(RasterError::Alloc)?;
        resvg::render(
            &tree,
            tiny_skia::Transform::identity(),
            &mut pixmap.as_mut(),
        );
        pixmap
            .encode_png()
            .map_err(|e| RasterError::Encode(e.to_string()))
    }
}

/// Render an SVG document string to PNG bytes, building a one-shot context.
///
/// The embedded Yomogi handwriting font is always available;
/// `extra_fonts_dir` is an optional directory of additional fonts to load on
/// top. `font_family` is the default family used when an element does not
/// specify one. For repeated renders (e.g. a server), build a
/// [`RasterContext`] once instead of calling this per request.
pub fn svg_to_png(
    svg: &str,
    extra_fonts_dir: Option<&Path>,
    font_family: &str,
) -> Result<Vec<u8>, RasterError> {
    RasterContext::new(extra_fonts_dir, font_family).render_png(svg)
}

/// Render an SVG document string and write the PNG to `path`.
pub fn svg_to_png_file(
    svg: &str,
    extra_fonts_dir: Option<&Path>,
    font_family: &str,
    path: &Path,
) -> Result<(), RasterError> {
    let png = svg_to_png(svg, extra_fonts_dir, font_family)?;
    std::fs::write(path, png)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_minimal_svg() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 20 20"><rect width="20" height="20" fill="#000000"/></svg>"##;
        let png = svg_to_png(svg, None, "Yomogi").unwrap();
        // PNG magic number.
        assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G']);
    }

    /// The embedded Yomogi font must resolve without any `extra_fonts_dir`, so
    /// a standalone binary keeps its hand-drawn look without `assets/fonts`
    /// present at runtime.
    #[test]
    fn embedded_yomogi_resolves_without_fonts_dir() {
        let ctx = RasterContext::new(None, "Yomogi");
        let has_yomogi = ctx
            .opt
            .fontdb
            .faces()
            .any(|f| f.families.iter().any(|(name, _)| name == "Yomogi"));
        assert!(
            has_yomogi,
            "embedded Yomogi family should be present in the font database"
        );
    }
}
