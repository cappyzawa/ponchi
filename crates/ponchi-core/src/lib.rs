//! `ponchi-core`: the platform-independent core of ponchi.
//!
//! An agent writes a declarative [`scene::Scene`] (JSON); this crate parses and
//! resolves it ([`input`]), auto-lays it out ([`layout`]), and renders it to an
//! SVG document ([`render`]) using a hand-drawn backend ([`backend`]). It stops
//! at the SVG string and has no platform dependencies, so the same logic drives
//! both the native binary (`ponchi`, which adds PNG rasterization and a live
//! viewer) and the WASM build (`ponchi-wasm`, which renders SVG in the browser).
//!
//! There is no runtime network access anywhere in this crate (the local-only
//! invariant): every color is validated and every string XML-escaped, so no
//! external reference can leak into the generated SVG.

pub mod backend;
pub mod input;
pub mod layout;
pub mod render;
pub mod scene;
