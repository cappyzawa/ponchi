//! SVG -> PNG rasterization via usvg/resvg/tiny-skia, fully in-process (no
//! browser, no native cairo). Fonts come from the system plus the bundled
//! handwriting font directory.

use std::path::Path;

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

/// Build usvg options with system fonts plus the bundled font directory loaded,
/// A reusable rasterization context: the font database is loaded once
/// (scanning system fonts is the most expensive step) and reused across
/// renders. Build this once for a long-lived server; the standalone
/// [`svg_to_png`] helpers build a throwaway one for a single CLI render.
pub struct RasterContext {
    opt: usvg::Options<'static>,
}

impl RasterContext {
    /// Build a context with system fonts plus the bundled `fonts_dir` loaded,
    /// and `font_family` as the default family for elements without one.
    ///
    /// Emits a stderr warning if the bundled font directory is missing/empty or
    /// if `font_family` does not resolve, since rendering would otherwise fall
    /// back to a different font (or tofu) silently.
    pub fn new(fonts_dir: Option<&Path>, font_family: &str) -> Self {
        let mut opt = usvg::Options::default();
        opt.fontdb_mut().load_system_fonts();
        match fonts_dir {
            Some(dir) => {
                let before = opt.fontdb.len();
                opt.fontdb_mut().load_fonts_dir(dir);
                if opt.fontdb.len() == before {
                    eprintln!(
                        "warning: no fonts loaded from {} (handwriting look may be lost)",
                        dir.display()
                    );
                }
            }
            None => {
                eprintln!(
                    "warning: no bundled font directory found; using system fonts only \
                     (pass --font-path to locate {font_family})"
                );
            }
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
/// `fonts_dir` is an optional directory of bundled fonts (e.g. the Yomogi
/// handwriting font); `font_family` is the default family used when an element
/// does not specify one. For repeated renders (e.g. a server), build a
/// [`RasterContext`] once instead of calling this per request.
pub fn svg_to_png(
    svg: &str,
    fonts_dir: Option<&Path>,
    font_family: &str,
) -> Result<Vec<u8>, RasterError> {
    RasterContext::new(fonts_dir, font_family).render_png(svg)
}

/// Render an SVG document string and write the PNG to `path`.
pub fn svg_to_png_file(
    svg: &str,
    fonts_dir: Option<&Path>,
    font_family: &str,
    path: &Path,
) -> Result<(), RasterError> {
    let png = svg_to_png(svg, fonts_dir, font_family)?;
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
}
