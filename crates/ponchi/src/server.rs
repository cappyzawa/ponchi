//! Loopback live-viewer server.
//!
//! Binds 127.0.0.1 only. Renders happen at publish time; GET handlers serve
//! cached bytes from memory so polling cannot turn into a CPU loop. The HTTP
//! "source of truth" is the in-memory [`Published`] state; the `out/latest.*`
//! files are a best-effort convenience for the agent self-check loop (Read the
//! PNG directly) and are written via atomic rename after a successful render.

use crate::raster::RasterContext;
use ponchi_core::render::render_scene_with_font;
use ponchi_core::scene::Scene;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Request, Response, Server};

const MAX_BODY: usize = 1024 * 1024; // 1 MiB
const VIEWER_HTML: &str = include_str!("../viewer/index.html");

/// Monotonic counter making each atomic-write temp filename unique, so a stale
/// temp left by a crash or a concurrent write never causes `create_new` to fail
/// with `EEXIST`.
static WRITE_NONCE: AtomicU64 = AtomicU64::new(0);

/// A rendered, published scene plus its cached output bytes.
struct Published {
    version: u64,
    svg: Arc<str>,
    png: Arc<[u8]>,
}

/// Mutable state mutated under the lock during publish.
struct Inner {
    published: Published,
}

/// Shared server state. `inner` is the only thing mutated per publish; the
/// rest is set once at startup and read-only thereafter.
struct State {
    inner: Mutex<Inner>,
    out_dir: PathBuf,
    raster: RasterContext,
    font_family: String,
    token: String,
}

/// Run the live viewer until the process is killed. `initial` is the scene to
/// render first; it must already be validated. The chosen bearer token is
/// printed to stdout.
pub fn serve(
    port: Option<u16>,
    initial: Scene,
    out_dir: PathBuf,
    extra_fonts_dir: Option<PathBuf>,
    font_family: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let token = random_token();
    // Build the font database once; reused for every render. The embedded
    // Yomogi font is always loaded; `extra_fonts_dir` adds fonts on top.
    let raster = RasterContext::new(extra_fonts_dir.as_deref(), &font_family);

    let published = render_published(&raster, &initial, &font_family, 1)?;
    std::fs::create_dir_all(&out_dir)?;
    // Best-effort initial file dump; failure here is not fatal.
    if let Err(e) = write_outputs(&out_dir, &published) {
        eprintln!("warning: failed to write initial outputs: {e}");
    }

    let state = Arc::new(State {
        inner: Mutex::new(Inner { published }),
        out_dir,
        raster,
        font_family,
        token,
    });

    // `port` is None -> bind to :0 so the OS assigns a free port. This lets
    // multiple sessions each get their own viewer without colliding.
    let addr = format!("127.0.0.1:{}", port.unwrap_or(0));
    let server = Server::http(&addr).map_err(|e| format!("failed to bind {addr}: {e}"))?;
    let actual_port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or_else(|| port.unwrap_or(0));
    println!("ponchi serve listening on http://127.0.0.1:{actual_port}/");
    println!("POST token (Authorization: Bearer ...): {}", state.token);

    for request in server.incoming_requests() {
        handle(request, &state);
    }
    Ok(())
}

/// Shared SVG document text and its rendered PNG bytes, version-independent.
type RenderedBytes = (Arc<str>, Arc<[u8]>);

/// Render a scene to SVG + PNG bytes (version-independent). Either render step
/// failing returns an error and leaves shared state alone. The version is
/// assigned later, inside the publish lock.
fn render_bytes(
    raster: &RasterContext,
    scene: &Scene,
    font_family: &str,
) -> Result<RenderedBytes, Box<dyn std::error::Error>> {
    let svg = render_scene_with_font(scene, font_family);
    let png = raster.render_png(&svg)?;
    Ok((Arc::from(svg.as_str()), Arc::from(png.into_boxed_slice())))
}

/// Render and build the initial [`Published`] at version 1.
fn render_published(
    raster: &RasterContext,
    scene: &Scene,
    font_family: &str,
    version: u64,
) -> Result<Published, Box<dyn std::error::Error>> {
    let (svg, png) = render_bytes(raster, scene, font_family)?;
    Ok(Published { version, svg, png })
}

/// Write `latest.svg` and `latest.png` to `out_dir` via atomic rename. This is
/// best effort: two files cannot be swapped atomically together, but each file
/// is individually consistent (never half-written). The HTTP source of truth is
/// the in-memory cache, not these files.
fn write_outputs(out_dir: &Path, p: &Published) -> std::io::Result<()> {
    atomic_write(&out_dir.join("latest.svg"), p.svg.as_bytes())?;
    atomic_write(&out_dir.join("latest.png"), &p.png)?;
    Ok(())
}

/// Write `bytes` to `path` atomically: a uniquely named temp file in the same
/// directory is created, fully written, fsynced, then renamed over `path`. The
/// temp name carries a process-unique nonce so it never collides.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out".into());
    let nonce = WRITE_NONCE.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}.{nonce}.tmp", std::process::id()));

    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Generate a 32-hex-character random token from the OS entropy source. Uses
/// `/dev/urandom` directly to avoid adding a crate; falls back to a
/// time/pid-seeded mix only if that is unavailable (with a loud warning, since
/// the token then becomes predictable).
fn random_token() -> String {
    let mut buf = [0u8; 16];
    if read_urandom(&mut buf).is_err() {
        eprintln!(
            "warning: could not read /dev/urandom; falling back to a weak, \
             predictable token. Treat the POST endpoint as unauthenticated."
        );
        // Fallback: not cryptographically strong, but the server is loopback
        // only. Mix time and pid so it is at least unpredictable per run.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id() as u128;
        let mut x = t ^ (pid << 64) ^ 0x9e37_79b9_7f4a_7c15_9e37_79b9_7f4a_7c15;
        for b in buf.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = (x & 0xff) as u8;
        }
    }
    let token: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    debug_assert!(!token.is_empty());
    token
}

fn read_urandom(buf: &mut [u8]) -> std::io::Result<()> {
    let mut f = std::fs::File::open("/dev/urandom")?;
    f.read_exact(buf)
}

/// Constant-time string comparison to avoid leaking the token via timing.
fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Strip a query string (`?...`) and return just the path portion of a URL.
fn path_of(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

fn header_value<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    // HeaderField::equiv requires a &'static str; compare case-insensitively
    // against the field's string form instead so callers can pass any &str.
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn html_response(body: &'static str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_header(Header::from_bytes(b"Content-Type", b"text/html; charset=utf-8").unwrap())
}

fn json_error(code: u16, msg: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = format!("{{\"error\":{}}}", serde_json::to_string(msg).unwrap());
    Response::from_string(body)
        .with_status_code(code)
        .with_header(Header::from_bytes(b"Content-Type", b"application/json").unwrap())
}

fn handle(mut request: Request, state: &Arc<State>) {
    let method = request.method().clone();
    let path = path_of(request.url()).to_string();

    let result = match (&method, path.as_str()) {
        (Method::Get, "/") => request.respond(html_response(VIEWER_HTML)),
        (Method::Get, "/api/version") => {
            let v = state.inner.lock().unwrap().published.version;
            request.respond(Response::from_string(v.to_string()))
        }
        (Method::Get, "/api/scene.png") => {
            let png = Arc::clone(&state.inner.lock().unwrap().published.png);
            request.respond(
                Response::from_data(png.to_vec())
                    .with_header(Header::from_bytes(b"Content-Type", b"image/png").unwrap()),
            )
        }
        (Method::Get, "/api/scene.svg") => {
            let svg = Arc::clone(&state.inner.lock().unwrap().published.svg);
            request.respond(
                Response::from_string(svg.to_string())
                    .with_header(Header::from_bytes(b"Content-Type", b"image/svg+xml").unwrap()),
            )
        }
        (Method::Post, "/api/scene") => {
            let resp = handle_post_scene(&mut request, state);
            request.respond(resp)
        }
        _ => request.respond(json_error(404, "not found")),
    };
    if let Err(e) = result {
        eprintln!("response error: {e}");
    }
}

/// Validate auth + content type, read the body, render, then publish under a
/// single lock. On any pre-publish failure the current published state is left
/// intact and a 4xx/5xx error is returned. Disk writes after the in-memory swap
/// are best-effort and never fail the request.
fn handle_post_scene(
    request: &mut Request,
    state: &Arc<State>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    // Auth: a non-empty Bearer token, constant-time compared. An absent or
    // empty token is rejected outright.
    let auth = header_value(request, "Authorization").unwrap_or("");
    let presented = auth.strip_prefix("Bearer ").unwrap_or("").trim();
    if presented.is_empty() || !token_eq(presented, &state.token) {
        return json_error(401, "missing or invalid bearer token");
    }

    // Content-Type must be JSON.
    let ct = header_value(request, "Content-Type").unwrap_or("");
    if !ct
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("application/json")
    {
        return json_error(415, "Content-Type must be application/json");
    }

    // Body size cap (trust Content-Length only as a hint; cap the read too).
    if let Some(len) = request.body_length()
        && len > MAX_BODY
    {
        return json_error(413, "body too large");
    }
    let mut body = Vec::new();
    if request
        .as_reader()
        .take((MAX_BODY + 1) as u64)
        .read_to_end(&mut body)
        .is_err()
    {
        return json_error(400, "failed to read body");
    }
    if body.len() > MAX_BODY {
        return json_error(413, "body too large");
    }

    let text = match std::str::from_utf8(&body) {
        Ok(t) => t,
        Err(_) => return json_error(400, "body is not valid UTF-8"),
    };

    let scene = match ponchi_core::input::parse_and_resolve(text) {
        Ok(s) => s,
        Err(e) => return json_error(400, &format!("invalid scene: {e}")),
    };

    // Render + rasterize outside the lock (the heavy work). The version is
    // independent of the bytes, so it is assigned later inside the lock.
    let (svg, png) = match render_bytes(&state.raster, &scene, &state.font_family) {
        Ok(b) => b,
        Err(e) => return json_error(500, &format!("render failed: {e}")),
    };

    // Publish atomically: assign the next version, swap the in-memory cache,
    // and capture what we need to write to disk — all under one lock so version
    // is monotonic even under concurrent POSTs.
    let (version, to_write) = {
        let mut inner = state.inner.lock().unwrap();
        let version = inner.published.version + 1;
        inner.published = Published {
            version,
            svg: Arc::clone(&svg),
            png: Arc::clone(&png),
        };
        // Clone the Arcs to write outside the lock.
        (
            version,
            Published {
                version,
                svg: Arc::clone(&svg),
                png: Arc::clone(&png),
            },
        )
    };

    // Best-effort disk dump; the in-memory cache is already the source of
    // truth, so a write failure logs but still returns success.
    if let Err(e) = write_outputs(&state.out_dir, &to_write) {
        eprintln!("warning: failed to write outputs for v{version}: {e}");
    }

    Response::from_string(format!("{{\"version\":{version}}}"))
        .with_header(Header::from_bytes(b"Content-Type", b"application/json").unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_query_string() {
        assert_eq!(path_of("/api/scene.png?v=42"), "/api/scene.png");
        assert_eq!(path_of("/"), "/");
        assert_eq!(path_of("/api/version"), "/api/version");
    }

    #[test]
    fn token_eq_matches_only_identical() {
        assert!(token_eq("abc123", "abc123"));
        assert!(!token_eq("abc123", "abc124"));
        assert!(!token_eq("abc", "abc123"));
    }

    #[test]
    fn random_token_is_hex_and_sized() {
        let t = random_token();
        assert_eq!(t.len(), 32);
        assert!(t.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
