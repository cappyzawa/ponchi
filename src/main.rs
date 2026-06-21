//! `ponchi` CLI: `render` a scene to PNG/SVG, or `serve` a live viewer.
//!
//! Arguments are parsed by hand (no clap) to keep the dependency surface small.

use ponchi::input::parse_and_resolve;
use ponchi::raster::svg_to_png_file;
use ponchi::render::{DEFAULT_FONT_FAMILY, render_scene_with_font};
use ponchi::scene::Scene;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match args.first().map(String::as_str) {
        Some("render") => cmd_render(&args[1..]),
        Some("serve") => cmd_serve(&args[1..]),
        Some("--help") | Some("-h") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => {
            print_usage();
            Err(format!("unknown command: {other}").into())
        }
    }
}

fn print_usage() {
    eprintln!(
        "ponchi — local-only hand-drawn diagram generator

USAGE:
  ponchi render <scene.json> -o <out.png|out.svg> [--font-family F] [--font-path DIR]
  ponchi serve [--port N] [--font-family F] [--font-path DIR]

The Yomogi handwriting font is embedded in the binary and always available.
--font-path DIR loads extra fonts from DIR on top, selectable via --font-family.

Default font family: {DEFAULT_FONT_FAMILY}
Default serve port:  auto (OS-assigned free port; printed at startup)"
    );
}

/// Parsed common options shared by both subcommands.
struct CommonOpts {
    font_family: String,
    /// Optional directory of extra fonts to load on top of the always-embedded
    /// Yomogi font, so other families can be selected via `--font-family`.
    font_path: Option<PathBuf>,
}

fn cmd_render(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut common = CommonOpts {
        font_family: DEFAULT_FONT_FAMILY.to_string(),
        font_path: None,
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                output = Some(PathBuf::from(next_arg(args, &mut i, "-o")?));
            }
            "--font-family" => {
                common.font_family = next_arg(args, &mut i, "--font-family")?;
            }
            "--font-path" => {
                common.font_path = Some(PathBuf::from(next_arg(args, &mut i, "--font-path")?));
            }
            other if input.is_none() && !other.starts_with('-') => {
                input = Some(PathBuf::from(other));
            }
            other => return Err(format!("unexpected argument: {other}").into()),
        }
        i += 1;
    }

    let input = input.ok_or("missing <scene.json>")?;
    let output = output.ok_or("missing -o <out.png|out.svg>")?;

    let json = std::fs::read_to_string(&input)?;
    let scene = parse_and_resolve(&json)?;
    let svg = render_scene_with_font(&scene, &common.font_family);

    let extra_fonts_dir = common.font_path;
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("svg") => {
            std::fs::write(&output, svg)?;
        }
        Some("png") => {
            svg_to_png_file(
                &svg,
                extra_fonts_dir.as_deref(),
                &common.font_family,
                &output,
            )?;
        }
        _ => return Err("output must end in .png or .svg".into()),
    }
    println!("wrote {}", output.display());
    Ok(())
}

fn cmd_serve(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // None -> let the OS pick a free port (avoids collisions across sessions).
    let mut port: Option<u16> = None;
    let mut common = CommonOpts {
        font_family: DEFAULT_FONT_FAMILY.to_string(),
        font_path: None,
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                port = Some(next_arg(args, &mut i, "--port")?.parse()?);
            }
            "--font-family" => {
                common.font_family = next_arg(args, &mut i, "--font-family")?;
            }
            "--font-path" => {
                common.font_path = Some(PathBuf::from(next_arg(args, &mut i, "--font-path")?));
            }
            other => return Err(format!("unexpected argument: {other}").into()),
        }
        i += 1;
    }

    let extra_fonts_dir = common.font_path;
    let initial = empty_scene();
    ponchi::server::serve(
        port,
        initial,
        PathBuf::from("out"),
        extra_fonts_dir,
        common.font_family,
    )
}

/// A minimal valid starting scene shown before any POST arrives.
fn empty_scene() -> Scene {
    use ponchi::scene::{Canvas, Text};
    Scene {
        schema_version: ponchi::scene::SCHEMA_VERSION.to_string(),
        title: "ponchi".into(),
        seed: 1,
        canvas: Canvas {
            w: 800,
            h: 400,
            background: "#ffffff".into(),
        },
        nodes: vec![],
        edges: vec![],
        zones: vec![],
        texts: vec![Text {
            id: "hint".into(),
            text: "POST a scene to /api/scene".into(),
            x: 40.0,
            y: 60.0,
        }],
    }
}

/// Consume the value following a flag at `args[*i]`, advancing the index.
///
/// Errors with a clear "missing value" message if the flag is at the end of the
/// argument list or is immediately followed by another `--flag`, rather than
/// silently swallowing the next flag as a value.
fn next_arg(
    args: &[String],
    i: &mut usize,
    flag: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    *i += 1;
    match args.get(*i) {
        Some(v) if !v.starts_with("--") => Ok(v.clone()),
        _ => Err(format!("missing value for {flag}").into()),
    }
}
