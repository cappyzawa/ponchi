//! `ponchi`: a local-only, hand-drawn-style diagram generator.
//!
//! An agent writes a declarative [`scene::Scene`] (JSON); the library renders
//! it to an SVG document ([`render`]) and rasterizes to PNG ([`raster`]),
//! entirely in-process with no browser and no runtime network access. A
//! loopback live viewer ([`server`]) lets a human watch updates while the
//! agent inspects the same server-rendered PNG.
//!
//! The crate is structured as a library (plus a thin CLI binary) so it can grow
//! into a CLI / skill / WASM target later.

pub mod backend;
pub mod input;
pub mod layout;
pub mod raster;
pub mod render;
pub mod scene;
pub mod server;
