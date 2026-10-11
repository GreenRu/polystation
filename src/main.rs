mod atlas;
mod blend;
mod build;
mod gl;
mod graph;
mod gui;
mod hier;
mod import;
mod json;
mod mcp;
mod guide;
mod light;
mod math;
mod obj;
mod pak;
mod particles;
mod play;
mod player;
mod prefabs;
mod prims;
mod project;
mod scatter;
mod script;
mod scene;
mod settings;
mod templates;
mod terrain;
mod trees;
mod ui;
mod utex;
mod win32;

use project::{CameraState, ProjectState, ViewState};
use scene::{Edits, Extra, SkyKind, Xform};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime};

// ------------------------------------------------------------------------------------------
// Shaders
//
// Vertex stage: the "math" is a model matrix (read per object from a data texture) and one matrix
// multiply. UVs are passed twice: `noperspective` makes the rasterizer interpolate them linearly in
// SCREEN space (no division by w per pixel) = affine texture mapping; `smooth` is the usual
// perspective-correct version. A uniform picks one, so the effect can be toggled live.
//
// Every vertex carries the id of the scene node (object) it belongs to. The `objs` texture holds,
// per object: 4 texels model matrix, 1 texel colour override (a > 0.5 = use it), 1 texel state
// (x: 0 normal, 1 hidden/deleted, 2 selected).
// ------------------------------------------------------------------------------------------
const VS: &str = "#version 330 core
layout(location = 0) in vec3 pos;
layout(location = 1) in vec2 uv;
layout(location = 2) in vec3 nrm;
layout(location = 3) in vec4 rect;
layout(location = 4) in float oid;
uniform mat4 mvp;
uniform vec3 cam;
uniform float time;
uniform sampler2D objs;
uniform vec3 light;          // the single sun used while the scene has no lights
uniform int nLights;
uniform vec4 lPos[16];       // xyz position, w: 0 point, 1 spot, 2 sun
uniform vec4 lCol[16];       // rgb color * intensity, w: range
uniform vec4 lDir[16];       // xyz direction the light shines in, w: cos of the spot half angle
uniform vec3 ambient;
smooth out vec3 v_lit;
smooth out vec2 v_uv_p;
noperspective out vec2 v_uv_a;
flat out vec3 v_n;
flat out vec4 v_rect;
flat out vec4 v_col;
flat out vec4 v_utex;
flat out float v_state;
flat out float v_oid;
flat out float v_alpha;
smooth out float v_dist;
void main() {
    int id = int(oid + 0.5);
    ivec2 b = ivec2(7 * (id % 256), id / 256);
    mat4 m = mat4(texelFetch(objs, b, 0), texelFetch(objs, b + ivec2(1, 0), 0),
                  texelFetch(objs, b + ivec2(2, 0), 0), texelFetch(objs, b + ivec2(3, 0), 0));
    v_col = texelFetch(objs, b + ivec2(4, 0), 0);
    v_utex = texelFetch(objs, b + ivec2(6, 0), 0);
    vec4 flags = texelFetch(objs, b + ivec2(5, 0), 0);
    float st = flags.x;
    v_alpha = flags.y;
    vec4 wp = m * vec4(pos, 1.0);
    if (flags.z > 0.5 && pos.y > -0.001) {
        // water: the top of the volume ripples
        wp.y += sin(time * 1.6 + wp.x * 0.9 + wp.z * 0.7) * 0.05 + sin(time * 2.3 - wp.x * 0.5 + wp.z * 1.1) * 0.03;
    }
    gl_Position = (st == 1.0) ? vec4(2.0, 2.0, 2.0, 1.0) : mvp * wp; // hidden: outside the view
    v_uv_p = uv;
    v_uv_a = uv;
    v_n = mat3(m) * nrm;
    v_rect = rect;
    v_state = st;
    v_oid = oid;
    v_dist = length(wp.xyz - cam);
    // lighting per vertex (the result is blended across the triangle)
    vec3 n = normalize(mat3(m) * nrm);
    vec3 lit;
    if (nLights == 0) {
        lit = ambient + vec3(0.65 * max(dot(n, light), 0.0));
    } else {
        lit = ambient;
        for (int i = 0; i < nLights; i++) {
            vec3 L;
            float att = 1.0;
            if (lPos[i].w > 1.5) {
                L = -lDir[i].xyz;
            } else {
                vec3 d = lPos[i].xyz - wp.xyz;
                float dist = length(d);
                L = d / max(dist, 1e-4);
                float x = clamp(1.0 - dist / max(lCol[i].w, 1e-3), 0.0, 1.0);
                att = x * x;
                if (lPos[i].w > 0.5) att *= smoothstep(lDir[i].w, mix(lDir[i].w, 1.0, 0.35), dot(-L, lDir[i].xyz));
            }
            lit += lCol[i].rgb * max(dot(n, L), 0.0) * att;
        }
    }
    v_lit = (flags.w > 0.5) ? vec3(1.0) : lit; // light markers glow by themselves
}
";

// The atlas is read with exact texel fetches (nearest, no filtering). A material's rect wraps the
// UVs (fract) so tiling textures work; with no rect the UVs are clamped over the whole atlas.
// mode: 0 textured+lit, 1 solid, 2 unlit, 3 object colours, 4 wire lines.
const FS: &str = "uniform sampler2D atlas;
uniform sampler2D userAtlas;
uniform float cutoff;
uniform int drawPass;
uniform int mode;
uniform int affine;
smooth in vec3 v_lit;
smooth in vec2 v_uv_p;
noperspective in vec2 v_uv_a;
flat in vec3 v_n;
flat in vec4 v_rect;
flat in vec4 v_col;
flat in vec4 v_utex;
flat in float v_state;
flat in float v_oid;
flat in float v_alpha;
smooth in float v_dist;
out vec4 color;
void main() {
    vec3 orange = vec3(1.0, 0.6, 0.1);
    if (mode == 4) {
        color = vec4(apply_fog(v_state > 1.5 ? orange : vec3(0.82), v_dist), 1.0);
        return;
    }
    // pass 0: opaque objects; 1: translucent materials; 2: opaque materials of translucent objects
    if (drawPass == 0 && v_alpha < 0.999) discard;
    if (drawPass == 2 && v_alpha >= 0.999) discard;
    vec2 uv = (affine != 0) ? v_uv_a : v_uv_p;
    ivec2 size = textureSize(atlas, 0);
    ivec2 p;
    if (v_rect.z <= 0.0) {
        p = clamp(ivec2(clamp(uv, 0.0, 1.0) * vec2(size)), ivec2(0), size - 1);
    } else {
        ivec2 wh = ivec2(v_rect.zw);
        p = ivec2(v_rect.xy) + min(ivec2(fract(uv) * v_rect.zw), wh - 1);
    }
    vec4 t = texelFetch(atlas, p, 0);
    vec3 base = t.rgb;
    float a = t.a;
    if (v_utex.z > 0.0) {
        // a texture the user assigned to the object (in the user atlas); the colour, if set, tints it
        ivec2 uw = ivec2(v_utex.zw);
        ivec2 up = ivec2(v_utex.xy) + min(ivec2(fract(uv) * v_utex.zw), uw - 1);
        vec4 ut = texelFetch(userAtlas, up, 0);
        base = ut.rgb * (v_col.a > 0.5 ? v_col.rgb : vec3(1.0));
        a = ut.a;
    } else if (v_col.a > 0.5) {
        base = v_col.rgb;
        a = 1.0;
    }
    a *= v_alpha;
    if (a < cutoff) discard;
    vec3 n = normalize(v_n);
    if (!gl_FrontFacing) n = -n;
    vec3 k = v_lit;
    vec3 rgb;
    if (mode == 1) {
        rgb = vec3(0.78) * k;
    } else if (mode == 2) {
        rgb = base;
    } else if (mode == 3) {
        vec3 h = vec3(fract(sin(v_oid * 12.9898) * 43758.5453), fract(sin(v_oid * 78.233) * 12345.678), fract(sin(v_oid * 37.719) * 94512.12));
        rgb = (h * 0.6 + 0.35) * k;
    } else {
        rgb = base * k;
    }
    if (v_state > 1.5) rgb = mix(rgb, orange, 0.45); // selection highlight
    color = vec4(apply_fog(rgb, v_dist), a);
}
";

// Sky: one full-screen triangle; each pixel's view direction picks a colour (solid, vertical
// gradient, or a lookup into an equirectangular panorama).
const SKY_VS: &str = "#version 330 core
out vec2 ndc;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    ndc = p * 2.0 - 1.0;
    gl_Position = vec4(ndc, 1.0, 1.0);
}
";

const SKY_FS: &str = "#version 330 core
in vec2 ndc;
out vec4 color;
uniform vec3 camF;
uniform vec3 camR;
uniform vec3 camU;
uniform float tanHalf;
uniform float aspect;
uniform int skyMode;
uniform vec3 skySolid;
uniform vec3 skyTop;
uniform vec3 skyBottom;
uniform sampler2D skyTex;
void main() {
    vec3 dir = normalize(camF + camR * (ndc.x * tanHalf * aspect) + camU * (ndc.y * tanHalf));
    vec3 c;
    if (skyMode == 0) {
        c = skySolid;
    } else if (skyMode == 1) {
        c = mix(skyBottom, skyTop, smoothstep(-0.15, 0.65, dir.y));
    } else {
        float u = atan(dir.x, -dir.z) / 6.2831853 + 0.5;
        float v = asin(clamp(dir.y, -1.0, 1.0)) / 3.1415927 + 0.5;
        c = texture(skyTex, vec2(u, v)).rgb;
    }
    color = vec4(c, 1.0);
}
";

// Infinite ground grid on y = 0: each pixel's view ray is intersected with the plane, so the grid
// has no edge. Cell size follows the camera height (10x steps fade in and out), lines are
// anti-aliased with derivatives, and the plane fades away towards the horizon.
const GRID_INF_FS: &str = "in vec2 ndc;
out vec4 color;
uniform mat4 mvp;
uniform vec3 camPos;
uniform vec3 camF;
uniform vec3 camR;
uniform vec3 camU;
uniform float tanHalf;
uniform float aspect;
uniform float gridStep;
uniform float fadeDist;
float lineAA(vec2 p, float step) {
    vec2 q = p / step;
    vec2 d = abs(fract(q - 0.5) - 0.5) / max(fwidth(q), vec2(1e-6));
    return 1.0 - clamp(min(d.x, d.y), 0.0, 1.0);
}
void main() {
    vec3 dir = normalize(camF + camR * (ndc.x * tanHalf * aspect) + camU * (ndc.y * tanHalf));
    if (abs(dir.y) < 1e-5) discard;
    float t = -camPos.y / dir.y;
    if (t <= 0.0) discard;
    vec3 hit = camPos + dir * t;
    vec2 p = hit.xz;
    float h = max(abs(camPos.y), gridStep);
    float lod = log(h / gridStep) / log(10.0);
    float fl = floor(lod);
    float s1 = gridStep * pow(10.0, fl);
    float minor = lineAA(p, s1) * (1.0 - (lod - fl));
    float major = lineAA(p, s1 * 10.0);
    float a = max(minor * 0.55, major * 0.8);
    vec2 fw = max(fwidth(p), vec2(1e-6));
    float xAxis = 1.0 - clamp(abs(p.y) / (fw.y * 1.5), 0.0, 1.0);
    float zAxis = 1.0 - clamp(abs(p.x) / (fw.x * 1.5), 0.0, 1.0);
    vec3 col = vec3(0.34, 0.35, 0.42);
    float alpha = a;
    if (xAxis > 0.0) { col = mix(col, vec3(0.85, 0.22, 0.22), xAxis); alpha = max(alpha, xAxis); }
    if (zAxis > 0.0) { col = mix(col, vec3(0.22, 0.38, 0.92), zAxis); alpha = max(alpha, zAxis); }
    alpha *= 1.0 - smoothstep(fadeDist * 0.4, fadeDist, t);
    if (alpha < 0.01) discard;
    vec4 clip = mvp * vec4(hit, 1.0);
    gl_FragDepth = clip.z / clip.w * 0.5 + 0.5;
    color = vec4(apply_fog(col, t), alpha);
}
";

// Billboard particles. Everything is computed from the vertex id: a particle's fixed random place,
// where it has drifted to (velocity + gravity, restarting every `life` seconds, hidden by a fade),
// and its wrap-around into the cube centred on the camera. Six vertices per particle, no vertex buffer.
const PARTICLE_VS: &str = "#version 330 core
uniform mat4 mvp;
uniform float time;
uniform vec3 camPos;
uniform vec3 camR;
uniform vec3 camU;
uniform float boxSize;
uniform vec3 vel;
uniform float grav;
uniform float psize;
uniform float life;
out vec2 v_uv;
out float v_fade;
out float v_dist;
uint hash(uint x) {
    x ^= x >> 16; x *= 0x7feb352dU; x ^= x >> 15; x *= 0x846ca68bU; x ^= x >> 16;
    return x;
}
float rnd(uint id, uint k) {
    return float(hash(id * 4u + k + 0x9E3779B9U) & 0xFFFFFFu) / 16777216.0;
}
void main() {
    int pid = gl_VertexID / 6;
    int corner = gl_VertexID % 6;
    vec2 q;
    if (corner == 0) q = vec2(-1.0, -1.0);
    else if (corner == 1) q = vec2(1.0, -1.0);
    else if (corner == 2) q = vec2(1.0, 1.0);
    else if (corner == 3) q = vec2(-1.0, -1.0);
    else if (corner == 4) q = vec2(1.0, 1.0);
    else q = vec2(-1.0, 1.0);
    uint id = uint(pid);
    vec3 base = (vec3(rnd(id, 0u), rnd(id, 1u), rnd(id, 2u)) - 0.5) * boxSize;
    float u = fract(time / life + rnd(id, 3u));
    float tl = u * life;
    vec3 disp = vel * tl + vec3(0.0, -0.5 * grav * tl * tl, 0.0);
    vec3 rel = base + disp - camPos;
    rel = mod(rel + 0.5 * boxSize, boxSize) - 0.5 * boxSize;
    float edge = max(max(abs(rel.x), abs(rel.y)), abs(rel.z)) / (0.5 * boxSize);
    v_fade = smoothstep(0.0, 0.08, u) * (1.0 - smoothstep(0.92, 1.0, u)) * (1.0 - smoothstep(0.8, 1.0, edge));
    vec3 pos = camPos + rel + (camR * q.x + camU * q.y) * (0.5 * psize);
    v_uv = q * 0.5 + 0.5;
    v_dist = length(pos - camPos);
    gl_Position = mvp * vec4(pos, 1.0);
}
";

const PARTICLE_FS: &str = "in vec2 v_uv;
in float v_fade;
in float v_dist;
uniform vec4 tint;
uniform vec4 texRect;
uniform sampler2D userAtlas;
out vec4 color;
void main() {
    vec4 c = vec4(tint.rgb, 1.0);
    if (texRect.z > 0.0) {
        ivec2 p = ivec2(texRect.xy) + min(ivec2(clamp(v_uv, 0.0, 0.9999) * texRect.zw), ivec2(texRect.zw) - 1);
        c *= texelFetch(userAtlas, p, 0);
    }
    c.a *= tint.a * v_fade;
    if (c.a < 0.01) discard;
    color = vec4(apply_fog(c.rgb, v_dist), c.a);
}
";

const GRID_VS: &str = "#version 330 core
layout(location = 0) in vec3 p;
layout(location = 1) in vec3 c;
uniform mat4 mvp;
uniform vec3 cam;
out vec3 col;
smooth out float v_dist;
void main() {
    gl_Position = mvp * vec4(p, 1.0);
    col = c;
    v_dist = length(p - cam);
}
";

const GRID_FS: &str = "in vec3 col;
smooth in float v_dist;
out vec4 color;
void main() {
    color = vec4(apply_fog(col, v_dist), 1.0);
}
";

// Fog, shared by the model and grid fragment shaders. Mode 2 quantises the fog amount to `fogSteps` steps and
// picks the step with an 8x8 ordered (Bayer) dither in screen space. The pattern is evaluated on blocks
// of `ditherCell` pixels, so the dither is chunky and obvious like PlayStation-era fog.
const FOG_GLSL: &str = "uniform int fog;
uniform vec2 fogRange;
uniform vec3 fogColor;
uniform int ditherCell;
uniform float fogSteps;
float bayer8(ivec2 p) {
    int x = p.x & 7;
    int y = p.y & 7;
    int xy = x ^ y;
    int v = ((y & 1) << 5) | ((xy & 1) << 4) | (((y >> 1) & 1) << 3) | (((xy >> 1) & 1) << 2) | (((y >> 2) & 1) << 1) | ((xy >> 2) & 1);
    return (float(v) + 0.5) / 64.0;
}
vec3 apply_fog(vec3 rgb, float dist) {
    if (fog == 0) return rgb;
    float f = clamp((dist - fogRange.x) / max(fogRange.y - fogRange.x, 1e-4), 0.0, 1.0);
    if (fog == 2) f = floor(f * fogSteps + bayer8(ivec2(gl_FragCoord.xy) / max(ditherCell, 1))) / fogSteps;
    return mix(rgb, fogColor, f);
}
";

use scene::BACKGROUND;

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

/// Everything needed to display a model. `atlas` is None when the source supplied no texture.
struct Loaded {
    mesh: obj::Mesh,
    atlas: Option<atlas::Atlas>,
}

/// The model plus every model imported into it (imports that fail to load are skipped).
fn load_with_imports(path: &str, atlas_arg: Option<&str>, imports: &[String]) -> Result<Loaded, String> {
    let mut loaded = load_scene(path, atlas_arg)?;
    for imp in imports {
        match load_scene(imp, None) {
            Ok(b) => {
                let label = Path::new(imp).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Import".into());
                let (mesh, atlas) = import::merge(&loaded.mesh, loaded.atlas.as_ref(), &b.mesh, b.atlas.as_ref(), &label);
                loaded = Loaded { mesh, atlas };
            }
            Err(e) => eprintln!("import {imp} skipped: {e}"),
        }
    }
    Ok(loaded)
}

/// Loads an .obj, or a .blend (exported through Blender). `atlas_arg` overrides the texture.
fn load_scene(path: &str, atlas_arg: Option<&str>) -> Result<Loaded, String> {
    let (mut obj_path, mut atlas_path) = (path.to_string(), atlas_arg.map(String::from));
    if !path.to_lowercase().ends_with(".obj") && blend::is_model(path) {
        let exported = blend::export(path)?;
        obj_path = exported.obj;
        if atlas_path.is_none() {
            atlas_path = exported.bmp;
        }
    }
    let mesh = obj::load(&obj_path)?;
    let atlas = atlas_path.map(|p| atlas::Atlas::load_bmp(&p)).transpose()?;
    println!("{path}: {} triangles, {} quads, {} nodes", mesh.nodes[0].tris, mesh.nodes[0].quads, mesh.nodes.len());
    Ok(Loaded { mesh, atlas })
}

/// What "Add Script" starts a new script with.
const SCRIPT_TEMPLATE: &str = "// Runs once when you press Play.\non start {\n    print(\"hello from \" + self.name)\n}\n\n// Runs every frame while playing. dt = seconds since the last frame.\non update {\n    // rotation.y += 90 * dt\n}\n\n// Runs when a key goes down:\n// on key(\"e\") { visible = !visible }\n\n// Properties: position.x/y/z  rotation.x/y/z  scale.x/y/z  color.r/g/b  alpha  visible\n// Objects:    self  player  object(\"Name\")\n// Functions:  move(x,y,z) spin(x,y,z)  key(\"w\")  pressed(\"e\")  distance(a, b)  random(a, b)  set/get/add  hud  destroy  launch  sound\n// More: Help > Scripting Guide (GUIDE tab) and Help > Example Scripts.\n";

fn stamp(path: &str) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Why a model was loaded in the background.
enum Intent {
    /// The source file changed on disk: keep camera, visibility, selection and edits.
    Reload,
    /// A different model/project was opened. `state` restores a saved project's view and edits.
    Replace { state: Option<ProjectState>, project: Option<PathBuf> },
    /// A model to add to the open scene (`path` of the message).
    Import,
}

struct Msg {
    path: String,
    atlas_arg: Option<String>,
    imports: Vec<String>,
    intent: Intent,
    result: Result<Loaded, String>,
}

/// Result of converting an image to BMP in the background; the texture goes to the named objects.
struct TexMsg {
    names: Vec<String>,
    result: Result<String, String>,
}

/// Loads a model off the UI thread (a .blend export takes seconds) and reports back through `tx`.
fn spawn_load(tx: mpsc::Sender<Msg>, path: String, atlas_arg: Option<String>, imports: Vec<String>, intent: Intent) {
    std::thread::spawn(move || {
        let result = load_with_imports(&path, atlas_arg.as_deref(), &imports);
        let _ = tx.send(Msg { path, atlas_arg, imports, intent, result });
    });
}

/// Polls the source file; when it changes (and has stopped changing), reloads it in the background.
/// Blender saves via temp-file + rename, hence the settle wait. Set the returned flag to stop it.
fn spawn_watch(tx: mpsc::Sender<Msg>, path: String, atlas_arg: Option<String>, imports: Vec<String>) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        let mut seen = stamp(&path);
        loop {
            std::thread::sleep(Duration::from_millis(300));
            if flag.load(Ordering::Relaxed) {
                return;
            }
            let now = stamp(&path);
            if now.is_none() || now == seen {
                continue;
            }
            std::thread::sleep(Duration::from_millis(400));
            if flag.load(Ordering::Relaxed) {
                return;
            }
            if stamp(&path) != now {
                continue; // still being written
            }
            seen = now;
            println!("{path} changed, reloading ...");
            let result = load_with_imports(&path, atlas_arg.as_deref(), &imports);
            let msg = Msg { path: path.clone(), atlas_arg: atlas_arg.clone(), imports: imports.clone(), intent: Intent::Reload, result };
            if tx.send(msg).is_err() {
                return;
            }
        }
    });
    stop
}

// ------------------------------------------------------------------------------------------
// GPU uploads
// ------------------------------------------------------------------------------------------

unsafe fn upload_mesh(g: &gl::Gl, vbo: u32, ebo: u32, wire_ebo: u32, mesh: &obj::Mesh) {
    unsafe {
        (g.glBindBuffer)(gl::ARRAY_BUFFER, vbo);
        (g.glBufferData)(gl::ARRAY_BUFFER, (mesh.data.len() * 4) as isize, mesh.data.as_ptr() as *const c_void, gl::STATIC_DRAW);
        (g.glBindBuffer)(gl::ELEMENT_ARRAY_BUFFER, wire_ebo);
        (g.glBufferData)(gl::ELEMENT_ARRAY_BUFFER, (mesh.wire.len() * 4) as isize, mesh.wire.as_ptr() as *const c_void, gl::STATIC_DRAW);
        (g.glBindBuffer)(gl::ELEMENT_ARRAY_BUFFER, ebo);
        (g.glBufferData)(gl::ELEMENT_ARRAY_BUFFER, (mesh.indices.len() * 4) as isize, mesh.indices.as_ptr() as *const c_void, gl::STATIC_DRAW);
    }
}

/// Sky panorama: smooth filtering, wraps around horizontally, clamps at the poles. Texture unit 2.
unsafe fn upload_sky(g: &gl::Gl, tex: u32, atlas: &atlas::Atlas) {
    unsafe {
        (g.glActiveTexture)(gl::TEXTURE2);
        (g.glBindTexture)(gl::TEXTURE_2D, tex);
        (g.glTexImage2D)(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, atlas.w as i32, atlas.h as i32, 0, gl::RGBA, gl::UNSIGNED_BYTE, atlas.rgba.as_ptr() as *const c_void);
        for (p, v) in [
            (gl::TEXTURE_MIN_FILTER, gl::LINEAR), (gl::TEXTURE_MAG_FILTER, gl::LINEAR),
            (gl::TEXTURE_WRAP_S, gl::REPEAT), (gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE),
        ] {
            (g.glTexParameteri)(gl::TEXTURE_2D, p, v);
        }
        (g.glActiveTexture)(gl::TEXTURE0);
    }
}

/// The user texture atlas (everything assigned to objects and particles), nearest filtered. Unit 3.
unsafe fn upload_user_atlas(g: &gl::Gl, tex: u32, atlas: &atlas::Atlas) {
    unsafe {
        (g.glActiveTexture)(gl::TEXTURE3);
        (g.glBindTexture)(gl::TEXTURE_2D, tex);
        (g.glTexImage2D)(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, atlas.w as i32, atlas.h as i32, 0, gl::RGBA, gl::UNSIGNED_BYTE, atlas.rgba.as_ptr() as *const c_void);
        for (p, v) in [
            (gl::TEXTURE_MIN_FILTER, gl::NEAREST), (gl::TEXTURE_MAG_FILTER, gl::NEAREST),
            (gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE), (gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE),
        ] {
            (g.glTexParameteri)(gl::TEXTURE_2D, p, v);
        }
        (g.glActiveTexture)(gl::TEXTURE0);
    }
}

unsafe fn upload_atlas(g: &gl::Gl, tex: u32, atlas: &atlas::Atlas) {
    unsafe {
        (g.glActiveTexture)(gl::TEXTURE0);
        (g.glBindTexture)(gl::TEXTURE_2D, tex);
        (g.glTexImage2D)(gl::TEXTURE_2D, 0, gl::RGBA8 as i32, atlas.w as i32, atlas.h as i32, 0, gl::RGBA, gl::UNSIGNED_BYTE, atlas.rgba.as_ptr() as *const c_void);
    }
}

// ------------------------------------------------------------------------------------------
// Scene explorer: a native tree view mirrors `mesh.nodes`; checkboxes hide, selection highlights.
// ------------------------------------------------------------------------------------------

/// Objects per row of the per-object data texture (6 texels each).
const OBJ_PER_ROW: usize = 256;

struct Explorer {
    /// Tree item per node; 0 for deleted nodes (they are not shown).
    handles: Vec<isize>,
    checked: Vec<bool>, // visible?
    /// The active selected node (what the details panel shows).
    selected: Option<usize>,
    /// Every selected node; the active one is among them.
    multi: Vec<usize>,
}

fn is_ancestor_or_self(mesh: &obj::Mesh, anc: usize, mut i: usize) -> bool {
    loop {
        if i == anc {
            return true;
        }
        match mesh.nodes[i].parent {
            Some(p) => i = p,
            None => return false,
        }
    }
}

/// Rebuilds the tree view. `prev_hidden` carries visibility over from a previous state by node name.
fn build_explorer(window: &win32::Window, mesh: &obj::Mesh, hier: &[(String, String)], prev_hidden: &HashMap<String, bool>, prev_sel: Option<&str>, deleted: &[bool]) -> Explorer {
    window.tree_clear();
    let n = mesh.nodes.len();
    let (mut handles, mut checked) = (vec![0isize; n], vec![true; n]);
    let order = hier::order(&mesh.nodes, hier); // parents before their children, siblings as arranged
    for &i in &order {
        let node = &mesh.nodes[i];
        if deleted[i] {
            continue;
        }
        let hidden = prev_hidden.get(&node.name).copied().unwrap_or(node.hidden);
        let label = match node.kind {
            obj::Kind::Object => node.name.clone(),
            obj::Kind::Collection => format!("[{}]", node.name),
            obj::Kind::Environment => format!("<{}>", node.name),
        };
        let parent = node.parent.map(|p| handles[p]);
        handles[i] = window.tree_add(parent, &label, !hidden);
        checked[i] = !hidden;
    }
    for &i in order.iter().rev() {
        let has_kids = mesh.nodes.iter().enumerate().any(|(j, m)| m.parent == Some(i) && handles[j] != 0);
        if has_kids && handles[i] != 0 {
            window.tree_expand(handles[i]);
        }
    }
    let selected = prev_sel.and_then(|name| mesh.nodes.iter().enumerate().position(|(i, nd)| nd.name == name && handles[i] != 0));
    window.tree_select(selected.map_or(0, |s| handles[s]));
    Explorer { handles, checked, selected, multi: selected.into_iter().collect() }
}

/// Reads checkbox/selection changes from the tree. Returns true if anything changed.
fn poll_explorer(window: &win32::Window, mesh: &obj::Mesh, ex: &mut Explorer) -> bool {
    let mut changed = false;
    let n = mesh.nodes.len();
    for i in 0..n {
        if ex.handles[i] == 0 {
            continue;
        }
        let cur = window.tree_checked(ex.handles[i]);
        if cur != ex.checked[i] {
            changed = true;
            ex.checked[i] = cur;
            {
                // a checkbox applies to everything inside it (children of an object too)
                for j in 0..n {
                    if ex.handles[j] != 0 && is_ancestor_or_self(mesh, i, j) {
                        window.tree_set_checked(ex.handles[j], cur);
                        ex.checked[j] = cur;
                    }
                }
            }
        }
    }
    let h = window.tree_selected();
    let sel = if h == 0 { None } else { ex.handles.iter().position(|&x| x == h) };
    if sel != ex.selected {
        ex.selected = sel;
        changed = true;
    }
    changed
}

// ------------------------------------------------------------------------------------------
// Per-node runtime state derived from the edits
// ------------------------------------------------------------------------------------------

struct Runtime {
    xf: Vec<Xform>,
    color: Vec<Option<[f32; 3]>>,
    alpha: Vec<f32>,
    texture: Vec<Option<String>>,
    /// Deleted nodes, including everything inside a deleted collection.
    deleted: Vec<bool>,
}

/// Node `i` of the composed mesh: from the model, the "Primitives" collection, or an added primitive.
enum NodeRef {
    Base,
    /// The environment settings node.
    Env,
    PrimRoot,
    Extra(usize),
}

fn node_ref(n_base: usize, i: usize) -> NodeRef {
    if i < n_base {
        NodeRef::Base
    } else if i == n_base {
        NodeRef::Env
    } else if i == n_base + 1 {
        NodeRef::PrimRoot
    } else {
        NodeRef::Extra(i - n_base - 2)
    }
}

fn compute_runtime(n_base: usize, mesh: &obj::Mesh, edits: &Edits) -> Runtime {
    let n = mesh.nodes.len();
    let (mut xf, mut color, mut deleted): (Vec<Xform>, Vec<Option<[f32; 3]>>, Vec<bool>) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    let mut alpha: Vec<f32> = Vec::with_capacity(n);
    let mut texture: Vec<Option<String>> = Vec::with_capacity(n);
    for (i, node) in mesh.nodes.iter().enumerate() {
        match node_ref(n_base, i) {
            NodeRef::Base => {
                let e = edits.base.get(&node.name);
                xf.push(e.and_then(|e| e.xform).unwrap_or(Xform::at(node.center)));
                color.push(e.and_then(|e| e.color));
                alpha.push(e.and_then(|e| e.alpha).unwrap_or(1.0));
                texture.push(e.and_then(|e| e.texture.clone()));
                deleted.push(e.is_some_and(|e| e.deleted));
            }
            NodeRef::Env | NodeRef::PrimRoot => {
                xf.push(Xform::at([0.0; 3]));
                color.push(None);
                alpha.push(1.0);
                texture.push(None);
                deleted.push(false);
            }
            NodeRef::Extra(j) => {
                // a scatter layer lives in its terrain's space, so it takes that terrain's transform
                let ex = &edits.extras[j];
                let layer_xf = (ex.prim == prims::Prim::Scatter)
                    .then(|| edits.extras.iter().find(|t| t.prim == prims::Prim::Terrain && t.name == ex.scatter.target).map(|t| t.xform))
                    .flatten();
                xf.push(layer_xf.unwrap_or(ex.xform));
                color.push(edits.extras[j].color);
                alpha.push(edits.extras[j].alpha);
                texture.push(edits.extras[j].texture.clone());
                deleted.push(false);
            }
        }
    }
    // everything inside a deleted node is gone with it
    for i in 0..n {
        let (mut a, mut guard) = (mesh.nodes[i].parent, 0);
        while let Some(p) = a {
            if deleted[p] {
                deleted[i] = true;
                break;
            }
            a = mesh.nodes[p].parent;
            guard += 1;
            if guard > n {
                break;
            }
        }
    }
    Runtime { xf, color, alpha, texture, deleted }
}

/// Per-object extras of the object data texture: hidden by script / play mode, water, glowing.
struct ObjFx {
    hidden: HashSet<usize>,
    water: HashSet<usize>,
    emissive: HashSet<usize>,
}

unsafe fn upload_objects(g: &gl::Gl, tex: u32, mesh: &obj::Mesh, ex: &Explorer, rt: &Runtime, rects: &HashMap<String, [f32; 4]>, fx: &ObjFx) {
    let n = mesh.nodes.len();
    let rows = n.div_ceil(OBJ_PER_ROW).max(1);
    let width = 7 * OBJ_PER_ROW;
    let mut buf = vec![0f32; width * rows * 4];
    for (i, node) in mesh.nodes.iter().enumerate() {
        let x = &rt.xf[i];
        let m = math::model_matrix(x.pos, x.rot, x.scale, node.center);
        let selected = ex.multi.iter().any(|&s| is_ancestor_or_self(mesh, s, i));
        let hidden = !ex.checked[i] || rt.deleted[i] || fx.hidden.contains(&i);
        let state = if hidden { 1.0 } else if selected { 2.0 } else { 0.0 };
        let c = rt.color[i];
        let texel = ((i / OBJ_PER_ROW) * width + 7 * (i % OBJ_PER_ROW)) * 4;
        buf[texel..texel + 16].copy_from_slice(&m);
        buf[texel + 16..texel + 20].copy_from_slice(&c.map_or([0.0; 4], |c| [c[0], c[1], c[2], 1.0]));
        buf[texel + 20] = state;
        buf[texel + 21] = rt.alpha[i];
        buf[texel + 22] = if fx.water.contains(&i) { 1.0 } else { 0.0 };
        buf[texel + 23] = if fx.emissive.contains(&i) { 1.0 } else { 0.0 };
        if let Some(r) = rt.texture[i].as_ref().and_then(|p| rects.get(p)) {
            buf[texel + 24..texel + 28].copy_from_slice(r);
        }
    }
    unsafe {
        (g.glActiveTexture)(gl::TEXTURE1);
        (g.glBindTexture)(gl::TEXTURE_2D, tex);
        (g.glTexImage2D)(gl::TEXTURE_2D, 0, gl::RGBA32F, width as i32, rows as i32, 0, gl::RGBA, gl::FLOAT, buf.as_ptr() as *const c_void);
        (g.glActiveTexture)(gl::TEXTURE0);
    }
}

// ------------------------------------------------------------------------------------------
// Camera, display settings
// ------------------------------------------------------------------------------------------

/// Free-fly camera: `yaw` 0 looks down -Z, positive yaw turns right; positive pitch looks up.
#[derive(Clone, Copy)]
struct Cam {
    yaw: f32,
    pitch: f32,
    pos: [f32; 3],
}

impl Cam {
    fn forward(&self) -> [f32; 3] {
        [self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos()]
    }

    fn right(&self) -> [f32; 3] {
        [self.yaw.cos(), 0.0, self.yaw.sin()]
    }

    /// Looking at `center` from `dist` away along the default viewing direction.
    fn looking_at(center: [f32; 3], dist: f32, yaw: f32, pitch: f32) -> Cam {
        let mut c = Cam { yaw, pitch, pos: [0.0; 3] };
        let f = c.forward();
        c.pos = [center[0] - f[0] * dist, center[1] - f[1] * dist, center[2] - f[2] * dist];
        c
    }

    fn framed(mesh: &obj::Mesh) -> Cam {
        Cam::looking_at(mesh.center, mesh.radius * 3.0, -0.6, -0.4)
    }
}

#[derive(Clone, Copy)]
struct View {
    /// 0 textured, 1 solid, 2 wireframe, 3 textured + wireframe, 4 unlit, 5 object colours.
    mode: u16,
    affine: bool,
    grid: bool,
    cull: bool,
}

const FOG_NAMES: [&str; 3] = ["no fog", "smooth fog", "dithered fog"];
const MODE_NAMES: [&str; 6] = ["Textured", "Solid", "Wireframe", "Textured + Wire", "Unlit", "Object Colors"];


// ------------------------------------------------------------------------------------------
// Orientation widget (top right): shows the world axes as the camera sees them
// ------------------------------------------------------------------------------------------

/// Widget size and margin in pixels at 96 dpi.
const COMPASS_SIZE: f32 = 150.0;
const COMPASS_MARGIN: f32 = 10.0;

/// One end of a world axis as drawn in the widget (coordinates in the widget's -1..1 square).
struct Ball {
    x: f32,
    y: f32,
    depth: f32,
    r: f32,
    dir: [f32; 3],
    axis: usize,
    positive: bool,
}

/// A 2D line segment as a quad (screen-space, so no camera-facing needed).
fn push_line2(v: &mut Vec<f32>, a: (f32, f32), b: (f32, f32), hw: f32, col: [f32; 3]) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = dx.hypot(dy);
    if l < 1e-6 {
        return;
    }
    let (nx, ny) = (-dy / l * hw, dx / l * hw);
    let (a0, a1, b0, b1) = ([a.0 - nx, a.1 - ny, 0.0], [a.0 + nx, a.1 + ny, 0.0], [b.0 - nx, b.1 - ny, 0.0], [b.0 + nx, b.1 + ny, 0.0]);
    push_tri(v, a0, a1, b1, col);
    push_tri(v, a0, b1, b0, col);
}

// ------------------------------------------------------------------------------------------
// Gizmo: move / rotate / scale handles drawn over the selected object
// ------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gizmo {
    Off,
    Move,
    Rotate,
    Scale,
}

const GIZMOS: [Gizmo; 4] = [Gizmo::Off, Gizmo::Move, Gizmo::Rotate, Gizmo::Scale];
const GIZMO_NAMES: [&str; 4] = ["off", "move", "rotate", "scale"];

impl Gizmo {
    fn index(self) -> u16 {
        GIZMOS.iter().position(|&g| g == self).unwrap_or(0) as u16
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Handle {
    Axis(usize),
    /// The centre cube of the scale gizmo: scales all axes together.
    Uniform,
}

/// A handle being dragged. `start` is the transform when the drag began; `t0` the handle's initial
/// parameter (distance along the axis, angle around it, or on-screen distance for uniform scale).
#[derive(Clone)]
struct Drag {
    /// Every dragged object with its transform when the drag began.
    nodes: Vec<(usize, Xform)>,
    /// Gizmo pivot (centroid of the selection) when the drag began.
    center: [f32; 3],
    mode: Gizmo,
    handle: Handle,
    t0: f32,
}

const AXES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const AXIS_COLORS: [[f32; 3]; 3] = [[0.92, 0.22, 0.22], [0.3, 0.85, 0.3], [0.3, 0.45, 0.98]];
const HIGHLIGHT: [f32; 3] = [1.0, 0.9, 0.2];

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// Two axes spanning the plane perpendicular to axis `k` (u x v = axis, so angles are right-handed).
fn plane_basis(k: usize) -> ([f32; 3], [f32; 3]) {
    (AXES[(k + 1) % 3], AXES[(k + 2) % 3])
}

/// Parameter t of the point on the line `p0 + t * axis` that is closest to the ray (o, d).
fn axis_param(p0: [f32; 3], axis: [f32; 3], (o, d): ([f32; 3], [f32; 3])) -> f32 {
    let w0 = math::sub(p0, o);
    let (b, dd, e) = (math::dot(axis, d), math::dot(axis, w0), math::dot(d, w0));
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 { 0.0 } else { (b * e - dd) / denom }
}

/// Angle of the point where the ray hits the plane through `pivot` perpendicular to axis `k`.
fn plane_angle(pivot: [f32; 3], k: usize, (o, d): ([f32; 3], [f32; 3])) -> Option<f32> {
    let n = AXES[k];
    let denom = math::dot(d, n);
    if denom.abs() < 1e-4 {
        return None;
    }
    let s = math::dot(math::sub(pivot, o), n) / denom;
    let r = math::sub(add3(o, mul3(d, s)), pivot);
    let (u, v) = plane_basis(k);
    Some(math::dot(r, v).atan2(math::dot(r, u)))
}

fn dist_point_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    let t = if len2 < 1e-6 { 0.0 } else { (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0) };
    (p.0 - (a.0 + abx * t)).hypot(p.1 - (a.1 + aby * t))
}

fn push_tri(v: &mut Vec<f32>, a: [f32; 3], b: [f32; 3], c: [f32; 3], col: [f32; 3]) {
    for p in [a, b, c] {
        v.extend_from_slice(&p);
        v.extend_from_slice(&col);
    }
}

/// A flat ribbon from a to b that always faces the camera (so it has the same thickness from any angle).
fn push_ribbon(v: &mut Vec<f32>, a: [f32; 3], b: [f32; 3], view: [f32; 3], half_width: f32, col: [f32; 3]) {
    let side = math::cross(math::sub(b, a), view);
    let l = math::dot(side, side).sqrt();
    if l < 1e-9 {
        return;
    }
    let s = mul3(side, half_width / l);
    let (a0, a1, b0, b1) = (math::sub(a, s), add3(a, s), math::sub(b, s), add3(b, s));
    push_tri(v, a0, a1, b1, col);
    push_tri(v, a0, b1, b0, col);
}

fn push_pyramid(v: &mut Vec<f32>, base: [f32; 3], apex: [f32; 3], k: usize, r: f32, col: [f32; 3]) {
    let (u, w) = plane_basis(k);
    let c = |su: f32, sw: f32| add3(base, add3(mul3(u, su * r), mul3(w, sw * r)));
    let q = [c(-1.0, -1.0), c(1.0, -1.0), c(1.0, 1.0), c(-1.0, 1.0)];
    for i in 0..4 {
        push_tri(v, apex, q[i], q[(i + 1) % 4], col);
    }
    push_tri(v, q[0], q[2], q[1], col);
    push_tri(v, q[0], q[3], q[2], col);
}

fn push_cube(v: &mut Vec<f32>, center: [f32; 3], h: f32, col: [f32; 3]) {
    let c = |x: f32, y: f32, z: f32| add3(center, [x * h, y * h, z * h]);
    let p = [c(-1.0, -1.0, -1.0), c(1.0, -1.0, -1.0), c(1.0, 1.0, -1.0), c(-1.0, 1.0, -1.0), c(-1.0, -1.0, 1.0), c(1.0, -1.0, 1.0), c(1.0, 1.0, 1.0), c(-1.0, 1.0, 1.0)];
    for f in [[0, 1, 2, 3], [4, 5, 6, 7], [0, 1, 5, 4], [2, 3, 7, 6], [1, 2, 6, 5], [0, 3, 7, 4]] {
        push_tri(v, p[f[0]], p[f[1]], p[f[2]], col);
        push_tri(v, p[f[0]], p[f[2]], p[f[3]], col);
    }
}

const RING_SEGMENTS: usize = 64;

fn ring_point(pivot: [f32; 3], k: usize, size: f32, i: usize) -> [f32; 3] {
    let (u, w) = plane_basis(k);
    let th = std::f32::consts::TAU * i as f32 / RING_SEGMENTS as f32;
    add3(pivot, add3(mul3(u, th.cos() * size), mul3(w, th.sin() * size)))
}

impl Default for View {
    fn default() -> Self {
        View { mode: 0, affine: true, grid: true, cull: false }
    }
}

fn nice_step(x: f32) -> f32 {
    let p = 10f32.powf(x.max(1e-6).log10().floor());
    let m = x / p;
    p * if m < 2.0 { 1.0 } else if m < 5.0 { 2.0 } else { 5.0 }
}

/// The vertical (Y) axis line; the ground grid itself is drawn by the infinite-grid shader.
/// Interleaved pos(3) colour(3).
fn grid_vertices(scene_radius: f32) -> Vec<f32> {
    let e = scene_radius.max(1.0);
    vec![0.0, 0.0, 0.0, 0.25, 0.7, 0.25, 0.0, e, 0.0, 0.25, 0.7, 0.25]
}

// ------------------------------------------------------------------------------------------
// App
// ------------------------------------------------------------------------------------------

const PROJECT_FILTER: [(&str, &str); 1] = [("Polystation project (*.polystation, *.gpr)", "*.polystation;*.gpr")];
const MODEL_FILTER: [(&str, &str); 2] = [
    ("3D models (blend, obj, fbx, gltf, glb, stl, ply, dae, abc, usd, x3d, 3ds ...)", "*.blend;*.obj;*.fbx;*.gltf;*.glb;*.stl;*.ply;*.dae;*.abc;*.usd;*.usda;*.usdc;*.usdz;*.x3d;*.wrl;*.3ds"),
    ("All files", "*.*"),
];
const BMP_FILTER: [(&str, &str); 1] = [("Bitmap (*.bmp)", "*.bmp")];
const IMAGE_FILTER: [(&str, &str); 2] = [("Images (*.bmp, *.png, *.jpg, *.tga, *.tif)", "*.bmp;*.png;*.jpg;*.jpeg;*.tga;*.tif;*.tiff"), ("All files", "*.*")];

/// Everything play mode needs while it runs.
struct PlayState {
    walker: play::Walker,
    yaw: f32,
    pitch: f32,
    spawn: [f32; 3],
    world: play::World,
    world_key: (usize, usize, usize),
    params: player::PlayerParams,
    /// The step sound, a complete .wav already scaled to the step volume.
    sound: Option<Vec<u8>>,
    /// Node index of the Player object (hidden while playing).
    node: usize,
    saved_cam: Cam,
    saved_gizmo: Gizmo,
    scripts: Vec<(String, script::Script)>,
    keys_prev: Vec<bool>,
    t0: Instant,
    /// The edits as they were when Play started: scripts change objects, Stop puts everything back.
    backup: Edits,
    wrote: bool,
    world_dirty: bool,
    world_t: Instant,
    /// Trigger objects (node, world box) and the ones the player is inside right now.
    triggers: Vec<(usize, [f32; 6])>,
    inside: HashSet<usize>,
    water: Vec<(usize, [f32; 6])>,
}

/// Which blocks of the scripts to run.
enum ScriptRun {
    Start,
    Update(f32),
    /// (object name, true = the player left it / false = touched it)
    Events(Vec<(String, bool)>),
}

/// What scripts see and change while playing.
struct PlayHost<'a> {
    app: &'a mut App,
    walker: &'a mut play::Walker,
    yaw: &'a mut f32,
    held: &'a [bool],
    pressed: &'a [bool],
    gravity: f32,
    time: f64,
    changed: bool,
    world_dirty: bool,
}

/// Windows virtual-key code for a key name used in scripts ("w", "space", "up" ...).
fn script_vk(name: &str) -> Option<usize> {
    let b = name.as_bytes();
    if b.len() == 1 && b[0].is_ascii_alphanumeric() {
        return Some(b[0].to_ascii_uppercase() as usize);
    }
    Some(match name {
        "space" => 0x20,
        "shift" => 0x10,
        "ctrl" | "control" => 0x11,
        "alt" => 0x12,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "escape" | "esc" => 0x1B,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "mouse" | "click" => 0x01,
        "rightmouse" | "rightclick" => 0x02,
        _ => return None,
    })
}

impl PlayHost<'_> {
    fn node(&self, name: &str) -> Result<usize, String> {
        self.app
            .mesh
            .nodes
            .iter()
            .enumerate()
            .position(|(i, n)| n.name == name && n.kind == obj::Kind::Object && !self.app.rt.deleted[i])
            .ok_or_else(|| format!("there is no object called \"{name}\""))
    }
}

impl script::Host for PlayHost<'_> {
    fn get(&mut self, obj: &str, p: script::Prop) -> Result<f64, String> {
        use script::Prop::*;
        if obj == "player" {
            return match p {
                PosX | PosY | PosZ => Ok(self.walker.pos[p as usize - PosX as usize] as f64),
                RotY => Ok((-*self.yaw).to_degrees() as f64),
                Visible | Solid => Ok(1.0),
                _ => Err("the player only has position and rotation.y".into()),
            };
        }
        let i = self.node(obj)?;
        Ok(match p {
            PosX | PosY | PosZ => self.app.script_xform(i).pos[p as usize - PosX as usize] as f64,
            RotX | RotY | RotZ => self.app.script_xform(i).rot[p as usize - RotX as usize] as f64,
            ScaleX | ScaleY | ScaleZ => self.app.script_xform(i).scale[p as usize - ScaleX as usize] as f64,
            ColR | ColG | ColB => self.app.script_color(i)[p as usize - ColR as usize] as f64,
            Alpha => self.app.script_alpha(i) as f64,
            Visible => (self.app.ex.checked[i] && !self.app.script_hidden.contains(&i)) as u8 as f64,
            Solid => self.app.edits.collision_of(&self.app.mesh.nodes[i].name).blocks() as u8 as f64,
            Intensity => match node_ref(self.app.base.nodes.len(), i) {
                NodeRef::Extra(j) if self.app.edits.extras[j].prim == prims::Prim::Light => self.app.edits.extras[j].light.intensity as f64,
                _ => return Err("only a light has an intensity".into()),
            },
        })
    }

    fn set(&mut self, obj: &str, p: script::Prop, v: f64) -> Result<(), String> {
        use script::Prop::*;
        if !v.is_finite() {
            return Err("that value is not a number".into());
        }
        let v = v as f32;
        if obj == "player" {
            match p {
                PosX | PosY | PosZ => {
                    self.walker.pos[p as usize - PosX as usize] = v; // teleport
                    self.walker.vel[1] = 0.0;
                }
                RotY => *self.yaw = -v.to_radians(),
                _ => return Err("the player only has position and rotation.y".into()),
            }
            return Ok(());
        }
        let i = self.node(obj)?;
        match p {
            PosX | PosY | PosZ | RotX | RotY | RotZ | ScaleX | ScaleY | ScaleZ => {
                let mut x = self.app.script_xform(i);
                match p {
                    PosX | PosY | PosZ => x.pos[p as usize - PosX as usize] = v,
                    RotX | RotY | RotZ => x.rot[p as usize - RotX as usize] = v,
                    _ => x.scale[p as usize - ScaleX as usize] = v,
                }
                self.app.script_set_xform(i, x);
                self.world_dirty = true;
            }
            ColR | ColG | ColB => {
                let mut c = self.app.script_color(i);
                c[p as usize - ColR as usize] = v.clamp(0.0, 1.0);
                self.app.script_set_color(i, c);
            }
            Alpha => self.app.script_set_alpha(i, v.clamp(0.0, 1.0)),
            Intensity => match node_ref(self.app.base.nodes.len(), i) {
                NodeRef::Extra(j) if self.app.edits.extras[j].prim == prims::Prim::Light => self.app.edits.extras[j].light.intensity = v.clamp(0.0, 100.0),
                _ => return Err("only a light has an intensity".into()),
            },
            Solid => {
                let name = self.app.mesh.nodes[i].name.clone();
                let was = self.app.edits.collision_of(&name);
                // switching off makes it passable (a trigger stays a trigger); switching on makes it solid
                let new = if v != 0.0 { scene::Collision::Solid } else if was == scene::Collision::Solid { scene::Collision::Passable } else { was };
                self.app.edits.set_collision(&name, new);
                self.world_dirty = true;
            }
            Visible => {
                if v != 0.0 {
                    self.app.script_hidden.remove(&i);
                } else {
                    self.app.script_hidden.insert(i);
                }
                self.world_dirty = true;
            }
        }
        self.changed = true;
        Ok(())
    }

    fn exists(&self, obj: &str) -> bool {
        self.node(obj).is_ok()
    }

    fn key_held(&self, name: &str) -> bool {
        script_vk(name).is_some_and(|k| self.held[k])
    }

    fn key_pressed(&self, name: &str) -> bool {
        script_vk(name).is_some_and(|k| self.pressed[k])
    }

    fn time(&self) -> f64 {
        self.time
    }

    fn print(&mut self, text: &str) {
        println!("[script] {text}");
        self.app.status_note(text);
    }

    fn var_get(&self, name: &str) -> Option<script::Value> {
        self.app.script_vars.get(name).cloned()
    }

    fn var_set(&mut self, name: &str, v: script::Value) {
        self.app.script_vars.insert(name.to_string(), v);
    }

    fn launch(&mut self, height: f64) {
        let v = (2.0 * self.gravity.max(0.0) as f64 * height.max(0.0)).sqrt() as f32;
        self.walker.vel[1] = v;
        self.walker.on_ground = false;
    }

    fn gui_text(&self, name: &str) -> Option<String> {
        self.app.edits.gui.iter().find(|e| e.name == name).map(|e| e.text.clone())
    }

    fn gui_set_text(&mut self, name: &str, text: &str) -> Result<(), String> {
        let e = self.app.edits.gui.iter_mut().find(|e| e.name == name).ok_or_else(|| format!("there is no GUI element called \"{name}\""))?;
        e.text = text.to_string();
        self.changed = true;
        Ok(())
    }

    fn gui_get(&self, name: &str, field: &str) -> Result<f64, String> {
        let e = self.app.edits.gui.iter().find(|e| e.name == name).ok_or_else(|| format!("there is no GUI element called \"{name}\""))?;
        Ok(match field {
            "value" => e.value as f64,
            "visible" => e.visible as u8 as f64,
            "x" => e.x as f64,
            "y" => e.y as f64,
            "w" => e.w as f64,
            "h" => e.h as f64,
            "alpha" => e.alpha as f64,
            "r" => e.color[0] as f64,
            "g" => e.color[1] as f64,
            _ => e.color[2] as f64,
        })
    }

    fn gui_set(&mut self, name: &str, field: &str, v: f64) -> Result<(), String> {
        if !v.is_finite() {
            return Err("that value is not a number".into());
        }
        let e = self.app.edits.gui.iter_mut().find(|e| e.name == name).ok_or_else(|| format!("there is no GUI element called \"{name}\""))?;
        let v = v as f32;
        match field {
            "value" => e.value = v.clamp(0.0, 1.0),
            "visible" => e.visible = v != 0.0,
            "x" => e.x = v,
            "y" => e.y = v,
            "w" => e.w = v.max(1.0),
            "h" => e.h = v.max(1.0),
            "alpha" => e.alpha = v.clamp(0.0, 1.0),
            "r" => e.color[0] = v.clamp(0.0, 1.0),
            "g" => e.color[1] = v.clamp(0.0, 1.0),
            _ => e.color[2] = v.clamp(0.0, 1.0),
        }
        self.changed = true;
        Ok(())
    }

    fn gui_clicked(&self, name: &str) -> bool {
        self.app.gui_clicks.iter().any(|n| n == name)
    }

    fn set_cursor(&mut self, free: Option<bool>) {
        self.app.cursor_override = free; // the frame loop applies it
    }

    fn set_gui_mode(&mut self, mode: &str) -> Result<(), String> {
        let m = gui::Mode::from_name(mode).ok_or("the modes are \"look\", \"cursor\", \"toggle\" and \"hold\"")?;
        self.app.edits.gui_mode = m;
        self.changed = true;
        Ok(())
    }

    fn parent_of(&self, obj: &str) -> Option<String> {
        let i = self.node(obj).ok()?;
        let p = self.app.mesh.nodes[i].parent?;
        (self.app.mesh.nodes[p].kind == obj::Kind::Object).then(|| self.app.mesh.nodes[p].name.clone())
    }

    fn children_of(&self, obj: &str) -> Vec<String> {
        let Ok(i) = self.node(obj) else { return Vec::new() };
        let kids = hier::children(&self.app.mesh.nodes, &self.app.edits.hier);
        kids[i].iter().filter(|&&k| self.app.mesh.nodes[k].kind == obj::Kind::Object && !self.app.rt.deleted[k]).map(|&k| self.app.mesh.nodes[k].name.clone()).collect()
    }

    fn set_parent(&mut self, child: &str, parent: Option<&str>) -> Result<(), String> {
        let c = self.node(child)?;
        let target = match parent {
            Some(p) => self.node(p)?,
            None => 0,
        };
        let h = hier::moved(&self.app.mesh.nodes, &self.app.edits.hier, c, hier::Place::Inside(target)).ok_or("an object cannot become its own parent or the parent of its parent")?;
        self.app.edits.hier = h;
        hier::apply(&mut self.app.mesh.nodes, &self.app.base.nodes, &self.app.edits.hier);
        self.changed = true;
        Ok(())
    }

    fn hud(&mut self, text: &str) {
        self.app.hud = text.to_string();
    }

    fn sound(&mut self, path: &str) {
        let p = Path::new(path);
        let full = match (p.is_relative(), self.app.project_path.as_ref().and_then(|pp| pp.parent())) {
            (true, Some(dir)) => dir.join(p),
            _ => p.to_path_buf(),
        };
        self.app.window.play_sound_file(&full.to_string_lossy());
    }
}

struct App {
    g: gl::Gl,
    window: win32::Window,
    prog: u32,
    locs: Locs,
    grid_prog: u32,
    grid_locs: Locs,
    grid_vao: u32,
    grid_vbo: u32,
    grid_count: i32,
    vao: u32,
    vbo: u32,
    ebo: u32,
    wire_ebo: u32,
    tex: u32,
    obj_tex: u32,
    /// The model as loaded (never modified) and the displayed mesh = model + added primitives.
    base: obj::Mesh,
    mesh: obj::Mesh,
    edits: Edits,
    undo: Vec<Edits>,
    redo: Vec<Edits>,
    /// (node, control) of the details-panel field being typed in, so one typing session = one undo step.
    edit_key: Option<(usize, u16)>,
    rt: Runtime,
    ex: Explorer,
    cam: Cam,
    speed_mul: f32,
    view: View,
    model_path: String,
    atlas_arg: Option<String>,
    project_path: Option<PathBuf>,
    watcher: Arc<AtomicBool>,
    tx: mpsc::Sender<Msg>,
    objects_dirty: bool,
    details_dirty: bool,
    look_active: bool,
    /// Right button held on the 3D view: (press position, when, mouse travel) - a short click opens a menu.
    rmb: Option<((i32, i32), Instant, i32)>,
    /// Play mode (first-person walk through the scene as the Player object), if running.
    play: Option<PlayState>,
    /// Objects a script has hidden (`visible = false`) while playing.
    script_hidden: HashSet<usize>,
    /// Nodes hidden only while playing (light markers).
    play_hidden: HashSet<usize>,
    /// The camera is below the surface of a water volume (play mode).
    underwater: bool,
    /// Values the scripts share (set / get / add) and the text they show in the status bar.
    script_vars: HashMap<String, script::Value>,
    hud: String,
    press: Option<((i32, i32), bool)>,
    orbit: Option<([f32; 3], f32)>,
    last_mouse: Option<(i32, i32)>,
    poll_t: Instant,
    status_t: Instant,
    fps: f32,
    shot: Option<String>,
    /// MCP screenshot requests, answered after the next frame is drawn.
    shot_waiters: Vec<mpsc::Sender<json::J>>,
    /// Running as a compiled game (no editor): Esc quits.
    game_mode: bool,
    /// The object whose script was typed in last, and when (typing is grouped into one undo step).
    script_undo: Option<(String, Instant)>,
    /// The mouse is a pointer right now (GUI buttons can be clicked) instead of turning the camera.
    cursor_free: bool,
    /// A script forced the pointer on / off (None: the GUI mode decides).
    cursor_override: Option<bool>,
    /// Tab toggles the pointer in the "Tab toggle" mode.
    tab_free: bool,
    tab_prev: bool,
    /// GUI buttons clicked this frame, and where the mouse is (in the 3D view).
    gui_clicks: Vec<String>,
    gui_mouse: (f32, f32),
    gui_down: bool,
    /// How the game is compiled (asked for in a dialog; saved with the project).
    build: build::BuildSettings,
    /// Models imported into the scene (on top of `model_path`), and the scene's current texture atlas.
    imports: Vec<String>,
    base_atlas: Option<atlas::Atlas>,
    /// Engine settings (this computer, not the project).
    settings: settings::Settings,
    gizmo: Gizmo,
    drag: Option<Drag>,
    hover: Option<Handle>,
    /// Folder shown in the file explorer and its rows (name, is a folder); row 0 may be "..".
    explorer_dir: PathBuf,
    explorer_items: Vec<(String, bool)>,
    /// Chunk the camera is in, per added object (only chunked terrains care).
    terrain_views: Vec<(i32, i32)>,
    /// Rubber-band selection rectangle (start, current) in canvas pixels.
    box_sel: Option<((i32, i32), (i32, i32))>,
    last_mvp: math::Mat4,
    gizmo_vao: u32,
    gizmo_vbo: u32,
    grid_inf_prog: u32,
    grid_inf: GridInfLocs,
    sky_prog: u32,
    sky_vao: u32,
    sky_locs: SkyLocs,
    sky_tex: u32,
    /// Path of the sky panorama currently uploaded to the GPU.
    sky_loaded: Option<String>,
    /// Textures assigned to objects, packed into one atlas on unit 3: where each path lives, which paths
    /// are packed, and the decoded images (so changing one texture does not re-read the others).
    particle_prog: u32,
    particle_locs: ParticleLocs,
    /// When the program started; the particles animate with it.
    t0: Instant,
    user_tex: u32,
    utex_rects: HashMap<String, [f32; 4]>,
    utex_key: Vec<String>,
    utex_cache: HashMap<String, atlas::Atlas>,
    tex_tx: mpsc::Sender<TexMsg>,
}

struct Locs {
    n_lights: i32,
    l_pos: i32,
    l_col: i32,
    l_dir: i32,
    ambient: i32,
    time: i32,
    mvp: i32,
    cutoff: i32,
    mode: i32,
    affine: i32,
    cam: i32,
    fog: i32,
    fog_range: i32,
    draw_pass: i32,
    dither: i32,
    fog_color: i32,
    fog_steps: i32,
}

struct GridInfLocs {
    mvp: i32,
    cam_pos: i32,
    cam_f: i32,
    cam_r: i32,
    cam_u: i32,
    tan_half: i32,
    aspect: i32,
    step: i32,
    fade: i32,
    fog: i32,
    fog_range: i32,
    fog_color: i32,
    fog_steps: i32,
    dither: i32,
}

struct ParticleLocs {
    mvp: i32,
    time: i32,
    cam_pos: i32,
    cam_r: i32,
    cam_u: i32,
    box_size: i32,
    vel: i32,
    grav: i32,
    size: i32,
    life: i32,
    tint: i32,
    tex_rect: i32,
    fog: i32,
    fog_range: i32,
    fog_color: i32,
    fog_steps: i32,
    dither: i32,
}

struct SkyLocs {
    cam_f: i32,
    cam_r: i32,
    cam_u: i32,
    tan_half: i32,
    aspect: i32,
    mode: i32,
    solid: i32,
    top: i32,
    bottom: i32,
}

/// Where an object that is glued to a parent ends up when the parent goes from pose `old` to pose `new`.
fn follow(old: &Xform, new: &Xform, c: &Xform) -> Xform {
    if old.rot == new.rot && old.scale == new.scale {
        // only moved: the exact, drift-free case
        let mut x = *c;
        for k in 0..3 {
            x.pos[k] += new.pos[k] - old.pos[k];
        }
        return x;
    }
    let transpose = |m: &math::Mat4| {
        let mut t = *m;
        t.swap(1, 4);
        t.swap(2, 8);
        t.swap(6, 9);
        t
    };
    let (r0, r1) = (math::rot_matrix(old.rot), math::rot_matrix(new.rot));
    let r0t = transpose(&r0);
    let ratio = |k: usize| if old.scale[k].abs() > 1e-6 { new.scale[k] / old.scale[k] } else { 1.0 };
    let sd = [ratio(0), ratio(1), ratio(2)];
    // into the parent's old frame, scale, back out through its new frame
    let local = math::transform_dir(&r0t, math::sub(c.pos, old.pos));
    let local = [local[0] * sd[0], local[1] * sd[1], local[2] * sd[2]];
    let pos = add3(new.pos, math::transform_dir(&r1, local));
    let turn = math::mul(&r1, &r0t);
    Xform { pos, rot: math::euler_from_matrix(&math::mul(&turn, &math::rot_matrix(c.rot))), scale: [c.scale[0] * sd[0], c.scale[1] * sd[1], c.scale[2] * sd[2]] }
}

/// Files being gathered into a game: every one is stored once and gets a virtual path inside `vdir`
/// (the project text is written relative to that directory).
struct Packer {
    pak: pak::Pak,
    vdir: PathBuf,
    done: HashMap<String, String>,
    /// Where relative paths are looked up (the project's folder).
    base: PathBuf,
}

impl Packer {
    /// Packs the file at `path` and returns its virtual absolute path (None: not a file we can read).
    fn add(&mut self, path: &str) -> Option<String> {
        if path.starts_with("builtin:") {
            return None;
        }
        if let Some(v) = self.done.get(path) {
            return Some(v.clone());
        }
        let p = Path::new(path);
        let full = if p.is_absolute() { p.to_path_buf() } else { self.base.join(p) };
        let bytes = std::fs::read(&full).ok()?;
        let file: String = full.file_name()?.to_string_lossy().chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
        let name = format!("pak/f{}_{}", self.done.len(), file);
        self.pak.add(&name, bytes);
        let v = self.vdir.join(&name).to_string_lossy().into_owned();
        self.done.insert(path.to_string(), v.clone());
        Some(v)
    }

    /// A virtual absolute path as a path relative to the game's folder.
    fn relative(&self, v: &str) -> String {
        Path::new(v).strip_prefix(&self.vdir).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| v.to_string())
    }
}

/// Replaces the file names in `sound("...")` calls of a script with whatever `f` returns for them.
fn rewrite_sounds(src: &str, f: &mut dyn FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(i) = rest.find("sound(") {
        let (head, tail) = rest.split_at(i + 6);
        out.push_str(head);
        let trimmed = tail.trim_start();
        let skipped = tail.len() - trimmed.len();
        match trimmed.strip_prefix('"').and_then(|t| t.find('"').map(|e| (&t[..e], &t[e..]))) {
            Some((name, after)) => {
                out.push_str(&tail[..skipped]);
                out.push('"');
                out.push_str(&f(name).unwrap_or_else(|| name.to_string()));
                rest = after;
            }
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

fn fmt_f(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" { "0".into() } else { s.to_string() }
}

impl App {
    #[allow(clippy::too_many_arguments)]
    fn new(window: win32::Window, loaded: Loaded, model_path: String, atlas_arg: Option<String>, project_path: Option<PathBuf>, startup: Option<ProjectState>, tx: mpsc::Sender<Msg>, watcher: Arc<AtomicBool>, tex_tx: mpsc::Sender<TexMsg>) -> App {
        let base_atlas = loaded.atlas.clone();
        let imports = startup.as_ref().map(|s| s.imports.clone()).unwrap_or_default();
        let atlas = loaded.atlas.unwrap_or_else(atlas::Atlas::procedural);
        let base = loaded.mesh;
        let g = unsafe { gl::Gl::load() }.unwrap_or_else(|e| die(&e));
        window.init_ui(g);
        unsafe {
            let fs = format!("#version 330 core\n{FOG_GLSL}{FS}");
            let grid_fs = format!("#version 330 core\n{FOG_GLSL}{GRID_FS}");
            let prog = g.compile_program(VS, &fs).unwrap_or_else(|e| die(&e));
            let grid_prog = g.compile_program(GRID_VS, &grid_fs).unwrap_or_else(|e| die(&e));
            (g.glPixelStorei)(gl::UNPACK_ALIGNMENT, 1);

            let (mut vao, mut vbo, mut ebo, mut wire_ebo) = (0, 0, 0, 0);
            (g.glGenVertexArrays)(1, &mut vao);
            (g.glBindVertexArray)(vao);
            (g.glGenBuffers)(1, &mut vbo);
            (g.glGenBuffers)(1, &mut ebo);
            (g.glGenBuffers)(1, &mut wire_ebo);
            (g.glBindBuffer)(gl::ARRAY_BUFFER, vbo);
            let stride = (obj::FLOATS * 4) as i32;
            for (loc, n, off) in [(0u32, 3, 0usize), (1, 2, 12), (2, 3, 20), (3, 4, 32), (4, 1, 48)] {
                (g.glVertexAttribPointer)(loc, n, gl::FLOAT, 0, stride, off as *const c_void);
                (g.glEnableVertexAttribArray)(loc);
            }

            let (mut grid_vao, mut grid_vbo) = (0, 0);
            (g.glGenVertexArrays)(1, &mut grid_vao);
            (g.glBindVertexArray)(grid_vao);
            (g.glGenBuffers)(1, &mut grid_vbo);
            (g.glBindBuffer)(gl::ARRAY_BUFFER, grid_vbo);
            for (loc, off) in [(0u32, 0usize), (1, 12)] {
                (g.glVertexAttribPointer)(loc, 3, gl::FLOAT, 0, 24, off as *const c_void);
                (g.glEnableVertexAttribArray)(loc);
            }
            let (mut gizmo_vao, mut gizmo_vbo) = (0, 0);
            (g.glGenVertexArrays)(1, &mut gizmo_vao);
            (g.glBindVertexArray)(gizmo_vao);
            (g.glGenBuffers)(1, &mut gizmo_vbo);
            (g.glBindBuffer)(gl::ARRAY_BUFFER, gizmo_vbo);
            for (loc, off) in [(0u32, 0usize), (1, 12)] {
                (g.glVertexAttribPointer)(loc, 3, gl::FLOAT, 0, 24, off as *const c_void);
                (g.glEnableVertexAttribArray)(loc);
            }
            (g.glBindVertexArray)(vao);

            // Unit 0: atlas (nearest filtering, no mipmaps: crisp PS1 look). Unit 1: per-object data.
            let (mut tex, mut obj_tex) = (0, 0);
            (g.glGenTextures)(1, &mut tex);
            (g.glGenTextures)(1, &mut obj_tex);
            upload_atlas(&g, tex, &atlas);
            for t in [tex, obj_tex] {
                (g.glActiveTexture)(if t == tex { gl::TEXTURE0 } else { gl::TEXTURE1 });
                (g.glBindTexture)(gl::TEXTURE_2D, t);
                for (p, v) in [
                    (gl::TEXTURE_MIN_FILTER, gl::NEAREST), (gl::TEXTURE_MAG_FILTER, gl::NEAREST),
                    (gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE), (gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE),
                ] {
                    (g.glTexParameteri)(gl::TEXTURE_2D, p, v);
                }
            }
            (g.glActiveTexture)(gl::TEXTURE0);

            (g.glUseProgram)(prog);
            let loc = |name: &[u8]| (g.glGetUniformLocation)(prog, name.as_ptr());
            let locs = Locs {
                n_lights: loc(b"nLights\0"), l_pos: loc(b"lPos\0"), l_col: loc(b"lCol\0"), l_dir: loc(b"lDir\0"), ambient: loc(b"ambient\0"),
                time: loc(b"time\0"),
                mvp: loc(b"mvp\0"), cutoff: loc(b"cutoff\0"), mode: loc(b"mode\0"), affine: loc(b"affine\0"),
                cam: loc(b"cam\0"), fog: loc(b"fog\0"), fog_range: loc(b"fogRange\0"), draw_pass: loc(b"drawPass\0"), dither: loc(b"ditherCell\0"), fog_color: loc(b"fogColor\0"), fog_steps: loc(b"fogSteps\0"),
            };
            (g.glUniform1i)(loc(b"atlas\0"), 0);
            (g.glUniform1i)(loc(b"objs\0"), 1);
            let l = math::normalize([0.4, 1.0, 0.6]);
            (g.glUniform3f)(loc(b"light\0"), l[0], l[1], l[2]);
            let gloc = |name: &[u8]| (g.glGetUniformLocation)(grid_prog, name.as_ptr());
            let grid_locs = Locs {
                n_lights: 0, l_pos: 0, l_col: 0, l_dir: 0, ambient: 0,
                time: 0,
                mvp: gloc(b"mvp\0"), cutoff: 0, mode: 0, affine: 0,
                cam: gloc(b"cam\0"), fog: gloc(b"fog\0"), fog_range: gloc(b"fogRange\0"), draw_pass: 0, dither: gloc(b"ditherCell\0"), fog_color: gloc(b"fogColor\0"), fog_steps: gloc(b"fogSteps\0"),
            };


            // Infinite grid program (same full-screen triangle as the sky)
            let grid_inf_fs = format!("#version 330 core\n{FOG_GLSL}{GRID_INF_FS}");
            let grid_inf_prog = g.compile_program(SKY_VS, &grid_inf_fs).unwrap_or_else(|e| die(&e));
            let giloc = |name: &[u8]| (g.glGetUniformLocation)(grid_inf_prog, name.as_ptr());
            let grid_inf = GridInfLocs {
                mvp: giloc(b"mvp\0"), cam_pos: giloc(b"camPos\0"), cam_f: giloc(b"camF\0"), cam_r: giloc(b"camR\0"), cam_u: giloc(b"camU\0"),
                tan_half: giloc(b"tanHalf\0"), aspect: giloc(b"aspect\0"), step: giloc(b"gridStep\0"), fade: giloc(b"fadeDist\0"),
                fog: giloc(b"fog\0"), fog_range: giloc(b"fogRange\0"), fog_color: giloc(b"fogColor\0"), fog_steps: giloc(b"fogSteps\0"), dither: giloc(b"ditherCell\0"),
            };

            // Sky: a program for the full-screen sky triangle (needs an empty VAO) and the panorama texture on unit 2.
            let sky_prog = g.compile_program(SKY_VS, SKY_FS).unwrap_or_else(|e| die(&e));
            let mut sky_vao = 0;
            (g.glGenVertexArrays)(1, &mut sky_vao);
            let mut sky_tex = 0;
            (g.glGenTextures)(1, &mut sky_tex);
            upload_sky(&g, sky_tex, &atlas::Atlas { w: 1, h: 1, rgba: vec![255; 4] });
            (g.glUseProgram)(sky_prog);
            let sloc = |name: &[u8]| (g.glGetUniformLocation)(sky_prog, name.as_ptr());
            let sky_locs = SkyLocs {
                cam_f: sloc(b"camF\0"), cam_r: sloc(b"camR\0"), cam_u: sloc(b"camU\0"), tan_half: sloc(b"tanHalf\0"), aspect: sloc(b"aspect\0"),
                mode: sloc(b"skyMode\0"), solid: sloc(b"skySolid\0"), top: sloc(b"skyTop\0"), bottom: sloc(b"skyBottom\0"),
            };
            (g.glUniform1i)(sloc(b"skyTex\0"), 2);
            (g.glUseProgram)(prog);
            let mut user_tex = 0;
            (g.glGenTextures)(1, &mut user_tex);
            upload_user_atlas(&g, user_tex, &atlas::Atlas { w: 1, h: 1, rgba: vec![255; 4] });
            (g.glUniform1i)((g.glGetUniformLocation)(prog, b"userAtlas\0".as_ptr()), 3);

            // particle billboards
            let particle_fs = format!("#version 330 core\n{FOG_GLSL}{PARTICLE_FS}");
            let particle_prog = g.compile_program(PARTICLE_VS, &particle_fs).unwrap_or_else(|e| die(&e));
            let ploc = |name: &[u8]| (g.glGetUniformLocation)(particle_prog, name.as_ptr());
            let particle_locs = ParticleLocs {
                mvp: ploc(b"mvp\0"), time: ploc(b"time\0"), cam_pos: ploc(b"camPos\0"), cam_r: ploc(b"camR\0"), cam_u: ploc(b"camU\0"),
                box_size: ploc(b"boxSize\0"), vel: ploc(b"vel\0"), grav: ploc(b"grav\0"), size: ploc(b"psize\0"), life: ploc(b"life\0"),
                tint: ploc(b"tint\0"), tex_rect: ploc(b"texRect\0"), fog: ploc(b"fog\0"), fog_range: ploc(b"fogRange\0"),
                fog_color: ploc(b"fogColor\0"), fog_steps: ploc(b"fogSteps\0"), dither: ploc(b"ditherCell\0"),
            };
            (g.glUseProgram)(particle_prog);
            (g.glUniform1i)(ploc(b"userAtlas\0"), 3);
            (g.glUseProgram)(prog);

            (g.glEnable)(gl::DEPTH_TEST);
            (g.glBlendFunc)(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            (g.glClearColor)(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 1.0);

            let mesh = scene::compose(&base, &[], &[]);
            let rt = compute_runtime(base.nodes.len(), &mesh, &Edits::default());
            let ex = Explorer { handles: Vec::new(), checked: Vec::new(), selected: None, multi: Vec::new() };
            let mut app = App {
                g, window, prog, locs, grid_prog, grid_locs, grid_vao, grid_vbo, grid_count: 0, vao, vbo, ebo, wire_ebo, tex, obj_tex,
                base, mesh, edits: Edits::default(), undo: Vec::new(), redo: Vec::new(), edit_key: None, rt, ex,
                cam: Cam { yaw: 0.0, pitch: 0.0, pos: [0.0; 3] }, speed_mul: 1.0, view: View::default(),
                model_path, atlas_arg, project_path, watcher, tx, imports, base_atlas,
                objects_dirty: true, details_dirty: true, look_active: false, rmb: None, play: None, script_hidden: HashSet::new(), script_vars: HashMap::new(), hud: String::new(), underwater: false, play_hidden: HashSet::new(), press: None, orbit: None, last_mouse: None,
                poll_t: Instant::now(), status_t: Instant::now() - Duration::from_secs(1), fps: 60.0, shot: None, shot_waiters: Vec::new(), game_mode: false, script_undo: None, cursor_free: false, cursor_override: None, tab_free: false, tab_prev: false, gui_clicks: Vec::new(), gui_mouse: (0.0, 0.0), gui_down: false, build: startup.as_ref().map(|s| s.build.clone()).unwrap_or_default(), settings: settings::Settings::load(),
                gizmo: Gizmo::Move, drag: None, hover: None, box_sel: None, explorer_dir: PathBuf::new(), explorer_items: Vec::new(), terrain_views: Vec::new(), last_mvp: math::identity(), gizmo_vao, gizmo_vbo,
                grid_inf_prog, grid_inf, sky_prog, sky_vao, sky_locs, sky_tex, sky_loaded: None,
                particle_prog, particle_locs, t0: Instant::now(),
                user_tex, utex_rects: HashMap::new(), utex_key: Vec::new(), utex_cache: HashMap::new(), tex_tx,
            };
            app.install(startup.as_ref(), None);
            // the explorer starts in "My Projects" (if there is one) next to where the program was started
            let home = std::env::current_dir().unwrap_or_default();
            app.explorer_dir = if home.join("My Projects").is_dir() { home.join("My Projects") } else { home };
            app.explorer_refresh();
            app
        }
    }

    /// Applies a project's saved state (or defaults) to the freshly set `base`, then rebuilds everything.
    fn install(&mut self, st: Option<&ProjectState>, keep_hidden: Option<(HashMap<String, bool>, Option<String>)>) {
        let (mut hidden, mut selected) = keep_hidden.unwrap_or_default();
        if let Some(st) = st {
            self.edits = st.edits.clone();
            self.view = View { mode: st.view.mode as u16, affine: st.view.affine, grid: st.view.grid, cull: st.view.cull };
            // hidden state is stored as a list; every other node is visible
            let mut names: Vec<String> = self.base.nodes.iter().map(|n| n.name.clone()).collect();
            names.extend(self.edits.extras.iter().map(|e| e.name.clone()));
            names.push(scene::PRIMS_COLLECTION.to_string());
            hidden = names.into_iter().map(|n| { let h = st.hidden.contains(&n); (n, h) }).collect();
            selected = st.selected.clone();
            match &st.camera {
                Some(c) => {
                    self.cam = Cam { yaw: c.yaw, pitch: c.pitch, pos: c.pos };
                    self.speed_mul = c.speed;
                }
                None => self.cam = Cam::framed(&self.base),
            }
        }
        self.rebuild(&hidden, selected.as_deref());
        self.rebuild_grid();
        self.sync_menu();
    }

    fn rebuild_grid(&mut self) {
        let verts = grid_vertices(self.base.radius);
        self.grid_count = (verts.len() / 6) as i32;
        unsafe {
            (self.g.glBindBuffer)(gl::ARRAY_BUFFER, self.grid_vbo);
            (self.g.glBufferData)(gl::ARRAY_BUFFER, (verts.len() * 4) as isize, verts.as_ptr() as *const c_void, gl::STATIC_DRAW);
        }
    }

    fn sync_menu(&self) {
        self.window.set_radio(win32::CMD_MODE_BASE, win32::CMD_MODE_BASE + win32::MODE_COUNT - 1, win32::CMD_MODE_BASE + self.view.mode);
        self.window.set_check(win32::CMD_AFFINE, self.view.affine);
        self.window.set_check(win32::CMD_GRID, self.view.grid);
        self.window.set_check(win32::CMD_CULL, self.view.cull);
        self.window.set_radio(win32::CMD_GIZMO_BASE, win32::CMD_GIZMO_BASE + 3, win32::CMD_GIZMO_BASE + self.gizmo.index());
        self.window.set_radio(win32::CMD_FOG_BASE, win32::CMD_FOG_BASE + win32::FOG_MODES - 1, win32::CMD_FOG_BASE + self.edits.env.fog as u16);
    }

    fn hidden_by_name(&self) -> HashMap<String, bool> {
        self.mesh.nodes.iter().zip(&self.ex.checked).map(|(n, &c)| (n.name.clone(), !c)).collect()
    }

    fn selected_name(&self) -> Option<String> {
        self.ex.selected.map(|s| self.mesh.nodes[s].name.clone())
    }

    /// Recomposes the mesh from the model + edits and rebuilds the GPU data and the explorer tree.
    fn rebuild(&mut self, hidden: &HashMap<String, bool>, selected: Option<&str>) {
        self.terrain_views = self.compute_terrain_views();
        self.mesh = scene::compose(&self.base, &self.edits.extras, &self.terrain_views);
        hier::apply(&mut self.mesh.nodes, &self.base.nodes, &self.edits.hier);
        self.rt = compute_runtime(self.base.nodes.len(), &self.mesh, &self.edits);
        unsafe { upload_mesh(&self.g, self.vbo, self.ebo, self.wire_ebo, &self.mesh) };
        self.ex = build_explorer(&self.window, &self.mesh, &self.edits.hier, hidden, selected, &self.rt.deleted);
        self.sync_script_badges();
        self.objects_dirty = true;
        self.details_dirty = true;
        self.edit_key = None;
    }

    /// Rebuild keeping the current visibility and selection.
    fn refresh(&mut self) {
        let (hidden, sel) = (self.hidden_by_name(), self.selected_name());
        self.rebuild(&hidden, sel.as_deref());
    }

    fn title(&self) -> String {
        let source = self.project_path.as_deref().unwrap_or(Path::new(&self.model_path));
        let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        format!("{name} - Polystation")
    }

    fn snapshot(&self) -> ProjectState {
        ProjectState {
            model: self.model_path.clone(),
            atlas: self.atlas_arg.clone(),
            imports: self.imports.clone(),
            build: self.build.clone(),
            camera: Some(CameraState { yaw: self.cam.yaw, pitch: self.cam.pitch, pos: self.cam.pos, speed: self.speed_mul }),
            view: ViewState { mode: self.view.mode as u8, affine: self.view.affine, grid: self.view.grid, cull: self.view.cull },
            hidden: self.mesh.nodes.iter().zip(&self.ex.checked).filter(|(_, c)| !**c).map(|(n, _)| n.name.clone()).collect(),
            selected: self.selected_name(),
            edits: self.edits.clone(),
        }
    }

    /// Swaps a freshly loaded model in and updates the view state.
    fn apply(&mut self, msg: Msg) {
        if matches!(msg.intent, Intent::Reload) && (msg.path != self.model_path || msg.imports != self.imports) {
            return; // stale: that model is no longer open
        }
        let new = match msg.result {
            Ok(n) => n,
            Err(e) => {
                eprintln!("load failed (keeping previous model): {e}");
                self.window.message(&format!("Could not load the model:\n\n{e}"));
                self.window.set_title(&self.title());
                return;
            }
        };
        if matches!(msg.intent, Intent::Import) {
            self.import_loaded(msg.path, new);
            return;
        }
        let replace = matches!(msg.intent, Intent::Replace { .. });
        let keep = (self.hidden_by_name(), self.selected_name());
        self.base = new.mesh;
        unsafe {
            match (&new.atlas, replace) {
                (Some(a), _) => upload_atlas(&self.g, self.tex, a),
                (None, true) => upload_atlas(&self.g, self.tex, &atlas::Atlas::procedural()),
                (None, false) => {} // a reload without a texture keeps the current one
            }
        }
        match (&new.atlas, replace) {
            (Some(a), _) => self.base_atlas = Some(a.clone()),
            (None, true) => self.base_atlas = None,
            (None, false) => {}
        }
        match msg.intent {
            Intent::Import => {}
            Intent::Reload => self.install(None, Some(keep)),
            Intent::Replace { state, project } => {
                self.watcher.store(true, Ordering::Relaxed);
                self.watcher = spawn_watch(self.tx.clone(), msg.path.clone(), msg.atlas_arg.clone(), msg.imports.clone());
                self.imports = msg.imports;
                self.model_path = msg.path;
                self.atlas_arg = msg.atlas_arg;
                self.project_path = project;
                self.edits = Edits::default();
                self.undo.clear();
                self.redo.clear();
                self.speed_mul = 1.0;
                self.cam = Cam::framed(&self.base);
                self.install(state.as_ref(), None);
            }
        }
        self.window.set_title(&self.title());
        println!("loaded");
    }

    /// Adds a freshly loaded model to the open scene: its nodes land in a collection of their own.
    fn import_loaded(&mut self, path: String, new: Loaded) {
        let label = Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Import".into());
        let keep = self.hidden_by_name();
        let nb = self.base.nodes.len();
        let (mesh, atlas) = import::merge(&self.base, self.base_atlas.as_ref(), &new.mesh, new.atlas.as_ref(), &label);
        let name = mesh.nodes[nb].name.clone();
        self.base = mesh;
        if let Some(a) = &atlas {
            unsafe { upload_atlas(&self.g, self.tex, a) };
            self.base_atlas = atlas;
        }
        self.imports.push(path);
        self.watcher.store(true, Ordering::Relaxed);
        self.watcher = spawn_watch(self.tx.clone(), self.model_path.clone(), self.atlas_arg.clone(), self.imports.clone());
        self.install(None, Some((keep, Some(name))));
        self.frame_selected();
        self.window.set_title(&self.title());
        self.status_note(&format!("Imported {label}"));
    }

    // ---------------------------------------------------------------- file explorer
    fn explorer_refresh(&mut self) {
        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<(String, u64)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.explorer_dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                match e.metadata() {
                    Ok(m) if m.is_dir() => dirs.push(name),
                    Ok(m) => files.push((name, m.len())),
                    Err(_) => {}
                }
            }
        }
        dirs.sort_by_key(|n| n.to_lowercase());
        files.sort_by_key(|(n, _)| n.to_lowercase());
        self.window.explorer_clear();
        self.explorer_items.clear();
        if self.explorer_dir.parent().is_some() {
            self.window.explorer_add("..", "Parent folder", "");
            self.explorer_items.push(("..".into(), true));
        }
        for d in dirs {
            self.window.explorer_add(&format!("{d}\\"), "Folder", "");
            self.explorer_items.push((d, true));
        }
        for (f, size) in files {
            let kind = match Path::new(&f).extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
                Some("polystation" | "gpr") => "Project",
                _ if blend::is_model(&f) => "Model",
                Some("bmp" | "png" | "jpg" | "jpeg" | "tga" | "tif" | "tiff") => "Image",
                _ => "File",
            };
            let size = if size < 1024 { format!("{size} B") } else { format!("{} KB", size / 1024) };
            self.window.explorer_add(&f, kind, &size);
            self.explorer_items.push((f, false));
        }
        self.window.set_text(win32::ST_EXPLORER_PATH, &self.explorer_dir.to_string_lossy());
    }

    /// Double-click on a row: enter a folder, open a project, import a model, or use an image as a texture.
    fn explorer_activate(&mut self) {
        let Some(row) = self.window.explorer_selected() else { return };
        let Some((name, is_dir)) = self.explorer_items.get(row).cloned() else { return };
        if is_dir {
            if name == ".." {
                self.explorer_dir.pop();
            } else {
                self.explorer_dir.push(&name);
            }
            self.explorer_refresh();
            return;
        }
        let path = self.explorer_dir.join(&name).to_string_lossy().into_owned();
        match Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
            Some("polystation" | "gpr") => self.open_project_path(path),
            _ if blend::is_model(&path) => self.import_model_path(path),
            Some("bmp" | "png" | "jpg" | "jpeg" | "tga" | "tif" | "tiff") => self.apply_texture_file(&path),
            _ => self.status_note("Nothing to do with that file type"),
        }
    }

    /// Uses an image as the texture of the selected objects. BMP files are used directly; other formats
    /// are converted to a BMP next to the original by Blender (in the background).
    fn apply_texture_file(&mut self, path: &str) {
        let objs = self.selected_objects();
        if objs.is_empty() {
            return self.status_note("Select an object (or a particle system) first, then pick a texture");
        }
        if Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("bmp")) {
            self.set_texture(&objs, Some(path.to_string()));
            return;
        }
        self.status_note("Converting the image with Blender ...");
        let names: Vec<String> = objs.iter().map(|&i| self.mesh.nodes[i].name.clone()).collect();
        let (tx, src) = (self.tex_tx.clone(), path.to_string());
        std::thread::spawn(move || {
            let result = blend::convert_image(&src);
            let _ = tx.send(TexMsg { names, result });
        });
    }

    fn apply_tex_msg(&mut self, m: TexMsg) {
        match m.result {
            Ok(path) => {
                let objs: Vec<usize> = (0..self.mesh.nodes.len()).filter(|&i| self.mesh.nodes[i].kind == obj::Kind::Object && m.names.contains(&self.mesh.nodes[i].name)).collect();
                self.set_texture(&objs, Some(path));
            }
            Err(e) => self.window.message(&format!("Could not use the image:\n\n{e}")),
        }
    }

    /// Sets (or clears) the texture of the given objects; one undo step.
    fn set_texture(&mut self, objs: &[usize], path: Option<String>) {
        if objs.is_empty() {
            return;
        }
        self.push_undo();
        let nb = self.base.nodes.len();
        for &i in objs {
            match node_ref(nb, i) {
                NodeRef::Base => self.edits.base.entry(self.base.nodes[i].name.clone()).or_default().texture = path.clone(),
                NodeRef::Extra(j) => self.edits.extras[j].texture = path.clone(),
                NodeRef::PrimRoot | NodeRef::Env => {}
            }
        }
        self.rt = compute_runtime(nb, &self.mesh, &self.edits);
        self.objects_dirty = true;
        self.details_dirty = true;
    }

    /// Packs every texture assigned to an object into the user atlas when the set of textures changed.
    fn sync_user_textures(&mut self) {
        let mut key: Vec<String> = self.rt.texture.iter().flatten().cloned().collect();
        key.sort();
        key.dedup();
        if key == self.utex_key {
            return;
        }
        self.utex_key = key.clone();
        let mut images = Vec::new();
        for path in key {
            if path == trees::PALETTE {
                self.utex_cache.entry(path.clone()).or_insert_with(trees::palette); // not a file: made by the engine
            } else if !self.utex_cache.contains_key(&path) {
                match atlas::Atlas::load_bmp(&path) {
                    Ok(a) => {
                        self.utex_cache.insert(path.clone(), a);
                    }
                    Err(e) => {
                        self.window.message(&format!("Could not load the texture:\n\n{e}"));
                        continue;
                    }
                }
            }
            images.push((path.clone(), self.utex_cache[&path].clone()));
        }
        let (packed, rects) = utex::pack(&images);
        unsafe { upload_user_atlas(&self.g, self.user_tex, &packed) };
        self.utex_rects = rects;
        self.objects_dirty = true;
    }

    // ---------------------------------------------------------------- project / files
    fn open_project(&mut self) {
        let Some(file) = self.window.file_dialog(false, &PROJECT_FILTER, project::EXT, "") else { return };
        self.open_project_path(file);
    }

    fn open_project_path(&mut self, file: String) {
        match project::load(Path::new(&file)) {
            Ok(st) => {
                settings::add_recent(&file);
                self.window.set_title("Loading... - Polystation");
                let (path, atlas) = (st.model.clone(), st.atlas.clone());
                let imports = st.imports.clone();
                let intent = Intent::Replace { state: Some(st), project: Some(PathBuf::from(file)) };
                spawn_load(self.tx.clone(), path, atlas, imports, intent);
            }
            Err(e) => self.window.message(&format!("Could not open the project:\n\n{e}")),
        }
    }

    fn save_project(&mut self, save_as: bool) {
        let path = match (&self.project_path, save_as) {
            (Some(p), false) => p.clone(),
            _ => {
                let stem = Path::new(&self.model_path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                match self.window.file_dialog(true, &PROJECT_FILTER, project::EXT, &format!("{stem}.{}", project::EXT)) {
                    Some(p) => PathBuf::from(p),
                    None => return,
                }
            }
        };
        match project::save(&path, &self.snapshot()) {
            Ok(()) => {
                settings::add_recent(&path.to_string_lossy());
                println!("saved {}", path.display());
                if let Some(dir) = std::path::absolute(&path).ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
                    self.explorer_dir = dir;
                    self.explorer_refresh();
                }
                self.project_path = Some(path);
                self.window.set_title(&self.title());
            }
            Err(e) => self.window.message(&format!("Could not save the project:\n\n{e}")),
        }
    }

    fn import_model(&mut self) {
        let Some(file) = self.window.file_dialog(false, &MODEL_FILTER, "", "") else { return };
        self.import_model_path(file);
    }

    fn import_model_path(&mut self, file: String) {
        self.window.set_title("Loading... - Polystation");
        // the model is merged into the open scene (see `apply`), not opened as a new project
        spawn_load(self.tx.clone(), file, None, Vec::new(), Intent::Import);
    }

    // ---------------------------------------------------------------- selection / camera helpers
    fn set_selected(&mut self, sel: Option<usize>) {
        self.set_multi(sel.into_iter().collect());
    }

    /// Selects exactly these nodes; the last one becomes the active one.
    fn set_multi(&mut self, list: Vec<usize>) {
        let list: Vec<usize> = list.into_iter().filter(|&i| self.ex.handles.get(i).is_some_and(|&h| h != 0)).collect();
        self.ex.selected = list.last().copied();
        self.ex.multi = list;
        self.window.tree_select(self.ex.selected.map_or(0, |s| self.ex.handles[s]));
        self.sync_selection_marks();
        self.objects_dirty = true;
        self.details_dirty = true;
        self.edit_key = None;
    }

    fn toggle_selected(&mut self, i: usize) {
        let mut list = self.ex.multi.clone();
        match list.iter().position(|&x| x == i) {
            Some(p) => {
                list.remove(p);
            }
            None => list.push(i),
        }
        self.set_multi(list);
    }

    fn select_names(&mut self, names: &[String]) {
        let list: Vec<usize> = names.iter().filter_map(|n| self.mesh.nodes.iter().position(|x| &x.name == n)).collect();
        self.set_multi(list);
    }

    /// A visible, undeleted object (something that can be picked, moved, deleted ...).
    fn is_selectable_object(&self, i: usize) -> bool {
        self.mesh.nodes[i].kind == obj::Kind::Object && !self.rt.deleted[i] && self.ex.handles[i] != 0 && self.ex.checked[i] && !self.is_particles(i)
    }

    fn is_particles(&self, i: usize) -> bool {
        matches!(node_ref(self.base.nodes.len(), i), NodeRef::Extra(j) if matches!(self.edits.extras[j].prim, prims::Prim::Particles | prims::Prim::Scatter))
    }

    /// Selected objects (collections and the environment are not included).
    fn selected_objects(&self) -> Vec<usize> {
        self.ex.multi.iter().copied().filter(|&i| self.mesh.nodes[i].kind == obj::Kind::Object && !self.rt.deleted[i]).collect()
    }

    /// Items of a multi-selection are drawn bold in the tree (the native control only highlights one).
    fn sync_selection_marks(&self) {
        let multi = self.ex.multi.len() > 1;
        for (i, &h) in self.ex.handles.iter().enumerate() {
            if h != 0 {
                self.window.tree_set_bold(h, multi && self.ex.multi.contains(&i));
            }
        }
    }

    /// Finishes a rubber-band selection: every visible object whose pivot lies inside the rectangle.
    fn finish_box(&mut self, a: (i32, i32), b: (i32, i32), additive: bool) {
        let (x0, x1) = (a.0.min(b.0) as f32, a.0.max(b.0) as f32);
        let (y0, y1) = (a.1.min(b.1) as f32, a.1.max(b.1) as f32);
        let mut list = if additive { self.ex.multi.clone() } else { Vec::new() };
        for i in 0..self.mesh.nodes.len() {
            if !self.is_selectable_object(i) || list.contains(&i) {
                continue;
            }
            if let Some((px, py)) = self.project(self.rt.xf[i].pos) {
                if px >= x0 && px <= x1 && py >= y0 && py <= y1 {
                    list.push(i);
                }
            }
        }
        self.set_multi(list);
    }

    /// Radius of a sphere around the model's centre that contains the model and every object added
    /// or moved since; fog distances, clip planes and fly speed scale with it.
    fn scene_radius(&self) -> f32 {
        let c = self.base.center;
        let mut r = self.base.radius;
        for (i, n) in self.mesh.nodes.iter().enumerate() {
            if n.kind == obj::Kind::Object && !self.rt.deleted[i] {
                let (p, wr) = self.world_bounds(i);
                let d = math::sub(p, c);
                r = r.max(math::dot(d, d).sqrt() + wr);
            }
        }
        r
    }

    /// World-space centre and radius of a node, including its transform.
    fn world_bounds(&self, i: usize) -> ([f32; 3], f32) {
        let n = &self.mesh.nodes[i];
        if n.kind == obj::Kind::Object {
            let x = &self.rt.xf[i];
            (x.pos, n.radius * x.scale.iter().fold(0.0f32, |m, s| m.max(s.abs())))
        } else {
            (n.center, n.radius)
        }
    }

    fn frame_selected(&mut self) {
        let objs = self.selected_objects();
        if objs.len() > 1 {
            // frame the whole selection: a sphere around every selected object
            let bounds: Vec<([f32; 3], f32)> = objs.iter().map(|&i| self.world_bounds(i)).collect();
            let n = bounds.len() as f32;
            let c = bounds.iter().fold([0.0; 3], |a, (p, _)| add3(a, *p));
            let c = mul3(c, 1.0 / n);
            let r = bounds.iter().map(|(p, r)| { let d = math::sub(*p, c); math::dot(d, d).sqrt() + r }).fold(0.0, f32::max);
            self.cam = Cam::looking_at(c, (r * 2.5).max(self.base.radius * 0.02), self.cam.yaw, self.cam.pitch);
            return;
        }
        let Some(s) = self.ex.selected else { return self.frame_all() };
        let (c, r) = self.world_bounds(s);
        if r <= 0.0 {
            return;
        }
        let d = (r * 2.5).max(self.base.radius * 0.02);
        self.cam = Cam::looking_at(c, d, self.cam.yaw, self.cam.pitch);
    }

    fn frame_all(&mut self) {
        self.cam = Cam::looking_at(self.base.center, self.base.radius * 2.5, self.cam.yaw, self.cam.pitch);
    }

    /// Ray through window pixel (mx, my) of the canvas: (origin, direction).
    fn ray(&self, mx: i32, my: i32) -> ([f32; 3], [f32; 3]) {
        let (w, h) = self.window.size();
        let (w, h) = (w.max(1) as f32, h.max(1) as f32);
        let t = (60f32.to_radians() * 0.5).tan();
        let (nx, ny) = (2.0 * mx as f32 / w - 1.0, 1.0 - 2.0 * my as f32 / h);
        let (f, r) = (self.cam.forward(), self.cam.right());
        let u = math::cross(r, f);
        let d = [
            f[0] + r[0] * nx * t * w / h + u[0] * ny * t,
            f[1] + r[1] * nx * t * w / h + u[1] * ny * t,
            f[2] + r[2] * nx * t * w / h + u[2] * ny * t,
        ];
        (self.cam.pos, math::normalize(d))
    }

    /// Nearest visible object under a canvas pixel (CPU ray cast against every triangle).
    fn pick(&self, mx: i32, my: i32) -> Option<usize> {
        let (o, d) = self.ray(mx, my);
        let inv: Vec<Option<math::Mat4>> = self
            .mesh
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let visible = n.kind == obj::Kind::Object && self.ex.checked[i] && !self.rt.deleted[i];
                visible.then(|| {
                    let x = &self.rt.xf[i];
                    math::inverse_model_matrix(x.pos, x.rot, x.scale, n.center)
                })
            })
            .collect();
        let (data, mut best, mut hit) = (&self.mesh.data, f32::MAX, None);
        for t in self.mesh.indices.chunks_exact(3) {
            let id = data[t[0] as usize * obj::FLOATS + obj::FLOATS - 1] as usize;
            let Some(m) = &inv[id] else { continue };
            let v = |k: usize| {
                let b = t[k] as usize * obj::FLOATS;
                [data[b], data[b + 1], data[b + 2]]
            };
            if let Some(tt) = math::ray_triangle(math::transform_point(m, o), math::transform_dir(m, d), v(0), v(1), v(2)) {
                if tt < best {
                    best = tt;
                    hit = Some(id);
                }
            }
        }
        hit
    }

    // ---------------------------------------------------------------- editing
    fn push_undo(&mut self) {
        self.undo.push(self.edits.clone());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn restore(&mut self, edits: Edits) {
        self.edits = edits;
        self.refresh();
        self.sync_script_tabs();
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.edits, Edits::default()));
            self.restore(prev);
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.edits, Edits::default()));
            self.restore(next);
        }
    }

    fn edit_xform(&mut self, i: usize, f: impl FnOnce(&mut Xform)) {
        let nb = self.base.nodes.len();
        let old = self.script_xform(i);
        match node_ref(nb, i) {
            NodeRef::Base => {
                let node = &self.base.nodes[i];
                let e = self.edits.base.entry(node.name.clone()).or_default();
                let mut x = e.xform.unwrap_or(Xform::at(node.center));
                f(&mut x);
                e.xform = Some(x);
            }
            NodeRef::Extra(j) => f(&mut self.edits.extras[j].xform),
            NodeRef::PrimRoot | NodeRef::Env => return,
        }
        let new = self.script_xform(i);
        self.push_followers(i, old, new, &[]);
        self.rt = compute_runtime(nb, &self.mesh, &self.edits);
        self.objects_dirty = true;
    }

    /// Stores new transforms for several nodes at once.
    fn set_xforms(&mut self, list: &[(usize, Xform)]) {
        let nb = self.base.nodes.len();
        let listed: Vec<usize> = list.iter().map(|l| l.0).collect();
        for &(i, x) in list {
            let old = self.script_xform(i);
            match node_ref(nb, i) {
                NodeRef::Base => self.edits.base.entry(self.base.nodes[i].name.clone()).or_default().xform = Some(x),
                NodeRef::Extra(j) => self.edits.extras[j].xform = x,
                NodeRef::PrimRoot | NodeRef::Env => {}
            }
            self.push_followers(i, old, x, &listed);
        }
        self.rt = compute_runtime(nb, &self.mesh, &self.edits);
        self.objects_dirty = true;
    }

    fn edit_color(&mut self, i: usize, c: Option<[f32; 3]>) {
        let nb = self.base.nodes.len();
        match node_ref(nb, i) {
            NodeRef::Base => self.edits.base.entry(self.base.nodes[i].name.clone()).or_default().color = c,
            NodeRef::Extra(j) => self.edits.extras[j].color = c,
            NodeRef::PrimRoot | NodeRef::Env => return,
        }
        self.rt = compute_runtime(nb, &self.mesh, &self.edits);
        self.objects_dirty = true;
        self.details_dirty = true;
    }

    // ---------------------------------------------------------------- orientation widget
    /// Widget square in canvas pixels: (left, top, size).
    fn compass_rect(&self) -> (f32, f32, f32) {
        let (w, _) = self.window.size();
        let (s, m) = (COMPASS_SIZE * win32::dpi_scale(), COMPASS_MARGIN * win32::dpi_scale());
        (w as f32 - s - m, m, s)
    }

    /// The six axis ends, farthest from the camera first (so nearer ones draw on top).
    fn compass_balls(&self) -> Vec<Ball> {
        let (f, r) = (self.cam.forward(), self.cam.right());
        let u = math::cross(r, f);
        let mut balls = Vec::new();
        for axis in 0..3 {
            for positive in [true, false] {
                let dir = mul3(AXES[axis], if positive { 1.0 } else { -1.0 });
                balls.push(Ball {
                    x: math::dot(dir, r) * 0.6,
                    y: math::dot(dir, u) * 0.6,
                    depth: math::dot(dir, f),
                    r: if positive { 0.17 } else { 0.11 },
                    dir,
                    axis,
                    positive,
                });
            }
        }
        balls.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        balls
    }

    /// Direction of the axis ball under canvas pixel (px, py), if any (the nearest one wins).
    fn compass_hit(&self, px: f32, py: f32) -> Option<[f32; 3]> {
        let (x0, y0, s) = self.compass_rect();
        let (nx, ny) = ((px - x0) / s * 2.0 - 1.0, 1.0 - (py - y0) / s * 2.0);
        self.compass_balls().iter().rev().find(|b| (nx - b.x).hypot(ny - b.y) < b.r * 1.25).map(|b| b.dir)
    }

    /// Snap the camera to look along an axis (from the +/- side the clicked ball is on), keeping the distance.
    fn view_along(&mut self, dir: [f32; 3]) {
        let f = mul3(dir, -1.0);
        let focus = self.ex.selected.map_or(self.base.center, |s| self.world_bounds(s).0);
        let to = math::sub(self.cam.pos, focus);
        let d = math::dot(to, to).sqrt().max(self.base.radius * 0.5);
        self.cam.pitch = f[1].asin().clamp(-1.55, 1.55);
        if f[0].abs() + f[2].abs() > 1e-4 {
            self.cam.yaw = f[0].atan2(-f[2]);
        }
        let nf = self.cam.forward();
        self.cam.pos = math::sub(focus, mul3(nf, d));
    }

    /// Draws the orientation widget with the UI renderer: smooth discs, a thin axis line to each
    /// positive end and the axis letter in the UI font. `vp` is the 3D view in window pixels.
    fn draw_compass(&self, c: &mut ui::style::Ctx, vp: [f32; 4]) {
        if self.play.is_some() {
            return;
        }
        // (the window is mutably borrowed while the UI draws: size comes from `vp`, not from `window.size()`)
        let size = COMPASS_SIZE * win32::dpi_scale();
        let (x0, y0) = (vp[2] - size - COMPASS_MARGIN * win32::dpi_scale(), COMPASS_MARGIN * win32::dpi_scale());
        let half = size * 0.5;
        let (cx, cy) = (vp[0] + x0 + half, vp[1] + y0 + half);
        let s = win32::dpi_scale();
        c.r.push_clip(vp[0], vp[1], vp[2], vp[3]);
        c.r.circle(cx, cy, half * 0.97, [0.07, 0.08, 0.10, 0.55], 1.0 * s, [1.0, 1.0, 1.0, 0.10]);
        let col = |k: usize| [AXIS_COLORS[k][0], AXIS_COLORS[k][1], AXIS_COLORS[k][2], 1.0];
        for b in self.compass_balls() {
            let (bx, by) = (cx + b.x * half, cy - b.y * half);
            // farther ends fade a little, so the order in depth reads at a glance
            let fade = 1.0 - 0.28 * (b.depth * 0.5 + 0.5).clamp(0.0, 1.0);
            let base = col(b.axis);
            if b.positive {
                let line = [base[0] * fade, base[1] * fade, base[2] * fade, 0.85];
                c.r.line(cx, cy, bx, by, 1.6 * s, line);
                let r = b.r * half * 1.28;
                let fill = [base[0] * fade, base[1] * fade, base[2] * fade, 1.0];
                c.r.circle(bx, by, r, fill, 1.2 * s, [1.0, 1.0, 1.0, 0.55 * fade]);
                let letter = ["X", "Y", "Z"][b.axis];
                let font = &c.f.bold;
                c.text_center(font, letter, bx - r, by - r, r * 2.0, r * 2.0, [0.05, 0.06, 0.09, 1.0]);
            } else {
                let r = b.r * half * 1.1;
                let fill = [base[0] * 0.35 * fade, base[1] * 0.35 * fade, base[2] * 0.35 * fade, 0.95];
                c.r.circle(bx, by, r, fill, 1.2 * s, [base[0], base[1], base[2], 0.75 * fade]);
            }
        }
        c.r.pop_clip();
    }

    // ---------------------------------------------------------------- gizmo
    /// The objects the gizmo manipulates: every selected, visible object.
    fn gizmo_targets(&self) -> Vec<usize> {
        if self.gizmo == Gizmo::Off {
            return Vec::new();
        }
        self.ex.multi.iter().copied().filter(|&i| self.is_selectable_object(i)).collect()
    }

    /// Where the gizmo sits: the centroid of the manipulated objects.
    fn gizmo_pivot(&self) -> Option<[f32; 3]> {
        let t = self.gizmo_targets();
        if t.is_empty() {
            return None;
        }
        let sum = t.iter().fold([0.0; 3], |a, &i| add3(a, self.rt.xf[i].pos));
        Some(mul3(sum, 1.0 / t.len() as f32))
    }

    /// Gizmo size in world units: a fixed fraction of the distance to the camera, so it keeps its size on screen.
    fn gizmo_size(&self, pivot: [f32; 3]) -> f32 {
        let d = math::sub(self.cam.pos, pivot);
        math::dot(d, d).sqrt().max(1e-3) * 0.2
    }

    /// World point to canvas pixel, using the previous frame's matrices.
    fn project(&self, p: [f32; 3]) -> Option<(f32, f32)> {
        let m = &self.last_mvp;
        let w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
        if w <= 1e-6 {
            return None;
        }
        let x = (m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12]) / w;
        let y = (m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13]) / w;
        let (cw, ch) = self.window.size();
        Some(((x * 0.5 + 0.5) * cw as f32, (0.5 - y * 0.5) * ch as f32))
    }

    fn gizmo_hit(&self, mx: i32, my: i32) -> Option<Handle> {
        let pivot = self.gizmo_pivot()?;
        let size = self.gizmo_size(pivot);
        let thr = 12.0 * win32::dpi_scale();
        let m = (mx as f32, my as f32);
        if self.gizmo == Gizmo::Scale {
            // the centre cube wins over the axes that start under it
            if let Some(c) = self.project(pivot) {
                if (m.0 - c.0).hypot(m.1 - c.1) < 14.0 * win32::dpi_scale() {
                    return Some(Handle::Uniform);
                }
            }
        }
        let mut best: Option<(f32, Handle)> = None;
        let mut consider = |d: f32, h: Handle| {
            if d < thr && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, h));
            }
        };
        match self.gizmo {
            Gizmo::Move | Gizmo::Scale => {
                if let Some(c) = self.project(pivot) {
                    for k in 0..3 {
                        if let Some(t) = self.project(add3(pivot, mul3(AXES[k], size))) {
                            consider(dist_point_segment(m, c, t), Handle::Axis(k));
                        }
                    }
                    if self.gizmo == Gizmo::Scale {
                        consider((m.0 - c.0).hypot(m.1 - c.1) * 0.6, Handle::Uniform);
                    }
                }
            }
            Gizmo::Rotate => {
                for k in 0..3 {
                    let pts: Vec<Option<(f32, f32)>> = (0..=RING_SEGMENTS).map(|i| self.project(ring_point(pivot, k, size, i))).collect();
                    for pair in pts.windows(2) {
                        if let (Some(a), Some(b)) = (pair[0], pair[1]) {
                            consider(dist_point_segment(m, a, b), Handle::Axis(k));
                        }
                    }
                }
            }
            Gizmo::Off => {}
        }
        best.map(|(_, h)| h)
    }

    fn begin_drag(&mut self, p: (i32, i32), handle: Handle) {
        let targets = self.gizmo_targets();
        let Some(center) = self.gizmo_pivot() else { return };
        let ray = self.ray(p.0, p.1);
        let t0 = match (self.gizmo, handle) {
            (Gizmo::Move | Gizmo::Scale, Handle::Axis(k)) => axis_param(center, AXES[k], ray),
            (Gizmo::Rotate, Handle::Axis(k)) => plane_angle(center, k, ray).unwrap_or(0.0),
            (Gizmo::Scale, Handle::Uniform) => {
                let c = self.project(center).unwrap_or((p.0 as f32, p.1 as f32));
                ((p.0 as f32 - c.0).hypot(p.1 as f32 - c.1)).max(8.0)
            }
            _ => 0.0,
        };
        self.push_undo(); // the whole drag is one undo step
        self.edit_key = None;
        let nodes = targets.iter().map(|&i| (i, self.rt.xf[i])).collect();
        self.drag = Some(Drag { nodes, center, mode: self.gizmo, handle, t0 });
    }

    /// Moves, rotates or scales every dragged object. Rotation and scale act around the selection's
    /// centroid (for a single object that is its own pivot).
    fn update_drag(&mut self, mx: i32, my: i32) {
        let Some(d) = self.drag.clone() else { return };
        let ray = self.ray(mx, my);
        let snap = win32::key_down(0x11); // Ctrl snaps
        let c = d.center;
        let (mut delta, mut factor, mut rot) = (0.0f32, 1.0f32, None);
        match (d.mode, d.handle) {
            (Gizmo::Move, Handle::Axis(k)) => {
                delta = axis_param(c, AXES[k], ray) - d.t0;
                if snap {
                    let step = nice_step(self.base.radius / 8.0) * 0.5;
                    let first = d.nodes[0].1.pos[k];
                    delta = ((first + delta) / step).round() * step - first;
                }
            }
            (Gizmo::Scale, Handle::Axis(k)) => {
                let t = axis_param(c, AXES[k], ray);
                factor = if d.t0.abs() > 1e-4 { (t / d.t0).max(0.01) } else { 1.0 };
            }
            (Gizmo::Scale, Handle::Uniform) => {
                let p = self.project(c).unwrap_or((mx as f32, my as f32));
                factor = ((mx as f32 - p.0).hypot(my as f32 - p.1) / d.t0).max(0.01);
            }
            (Gizmo::Rotate, Handle::Axis(k)) => {
                if let Some(a) = plane_angle(c, k, ray) {
                    let mut dl = a - d.t0;
                    if snap {
                        let step = 15f32.to_radians();
                        dl = (dl / step).round() * step;
                    }
                    rot = Some([math::rot_x(dl), math::rot_y(dl), math::rot_z(dl)][k]);
                }
            }
            _ => {}
        }
        if snap && d.mode == Gizmo::Scale {
            factor = ((factor * 10.0).round() / 10.0).max(0.1);
        }
        let mut out = Vec::with_capacity(d.nodes.len());
        for (node, start) in &d.nodes {
            let mut x = *start;
            match (d.mode, d.handle) {
                (Gizmo::Move, Handle::Axis(k)) => x.pos[k] += delta,
                (Gizmo::Scale, Handle::Axis(k)) => {
                    x.pos[k] = c[k] + (start.pos[k] - c[k]) * factor;
                    x.scale[k] = start.scale[k] * factor;
                }
                (Gizmo::Scale, Handle::Uniform) => {
                    for k in 0..3 {
                        x.pos[k] = c[k] + (start.pos[k] - c[k]) * factor;
                        x.scale[k] = start.scale[k] * factor;
                    }
                }
                (Gizmo::Rotate, Handle::Axis(_)) => {
                    if let Some(r) = &rot {
                        // rotate about the WORLD axis through the pivot: positions swing around it and the
                        // object's own rotation gets the same turn applied after its current one
                        x.pos = add3(c, math::transform_dir(r, math::sub(start.pos, c)));
                        x.rot = math::euler_from_matrix(&math::mul(r, &math::rot_matrix(start.rot)));
                    }
                }
                _ => {}
            }
            out.push((*node, x));
        }
        self.set_xforms(&out);
        self.details_dirty = true;
    }

    fn end_drag(&mut self) {
        self.drag = None;
        self.details_dirty = true;
    }

    /// Triangles (pos + colour per vertex) for the current gizmo.
    fn gizmo_geometry(&self) -> Vec<f32> {
        let mut v = Vec::new();
        let Some(pivot) = self.gizmo_pivot() else { return v };
        let size = self.gizmo_size(pivot);
        let view = self.cam.forward();
        let hw = size * 0.014;
        let active = self.drag.as_ref().map(|d| d.handle).or(self.hover);
        let col = |h: Handle, k: usize| if active == Some(h) { HIGHLIGHT } else { AXIS_COLORS[k] };
        match self.gizmo {
            Gizmo::Move => {
                for k in 0..3 {
                    let c = col(Handle::Axis(k), k);
                    let neck = add3(pivot, mul3(AXES[k], size * 0.78));
                    push_ribbon(&mut v, pivot, neck, view, hw, c);
                    push_pyramid(&mut v, neck, add3(pivot, mul3(AXES[k], size)), k, size * 0.07, c);
                }
            }
            Gizmo::Scale => {
                for k in 0..3 {
                    let c = col(Handle::Axis(k), k);
                    let tip = add3(pivot, mul3(AXES[k], size * 0.92));
                    push_ribbon(&mut v, pivot, tip, view, hw, c);
                    push_cube(&mut v, tip, size * 0.06, c);
                }
                let c = if active == Some(Handle::Uniform) { HIGHLIGHT } else { [0.85, 0.85, 0.85] };
                push_cube(&mut v, pivot, size * 0.075, c);
            }
            Gizmo::Rotate => {
                for k in 0..3 {
                    let c = col(Handle::Axis(k), k);
                    for i in 0..RING_SEGMENTS {
                        push_ribbon(&mut v, ring_point(pivot, k, size, i), ring_point(pivot, k, size, i + 1), view, hw, c);
                    }
                }
            }
            Gizmo::Off => {}
        }
        v
    }

    fn edit_alpha(&mut self, i: usize, a: f32) {
        let nb = self.base.nodes.len();
        match node_ref(nb, i) {
            NodeRef::Base => self.edits.base.entry(self.base.nodes[i].name.clone()).or_default().alpha = Some(a),
            NodeRef::Extra(j) => self.edits.extras[j].alpha = a,
            NodeRef::PrimRoot | NodeRef::Env => return,
        }
        self.rt = compute_runtime(nb, &self.mesh, &self.edits);
        self.objects_dirty = true;
    }

    fn delete_selected(&mut self) {
        let nb = self.base.nodes.len();
        let mut sel: Vec<usize> = self.ex.multi.iter().copied().filter(|&s| s != 0 && !matches!(node_ref(nb, s), NodeRef::Env)).collect();
        if sel.is_empty() {
            return; // the scene root and the environment cannot be deleted
        }
        for s in sel.clone() {
            for d in hier::descendants(&self.mesh.nodes, s) {
                if !sel.contains(&d) && !matches!(node_ref(nb, d), NodeRef::Env) {
                    sel.push(d); // whatever is inside goes along
                }
            }
        }
        self.push_undo();
        let (mut extra_idx, mut clear_all) = (Vec::new(), false);
        for &s in &sel {
            match node_ref(nb, s) {
                NodeRef::Base => self.edits.base.entry(self.base.nodes[s].name.clone()).or_default().deleted = true,
                NodeRef::Extra(j) => extra_idx.push(j),
                NodeRef::PrimRoot => clear_all = true,
                NodeRef::Env => {}
            }
        }
        if clear_all {
            self.edits.extras.clear();
        } else {
            extra_idx.sort_unstable();
            extra_idx.dedup();
            for j in extra_idx.into_iter().rev() {
                self.edits.extras.remove(j);
            }
        }
        let hidden = self.hidden_by_name();
        self.rebuild(&hidden, None);
    }

    fn add_primitive(&mut self, prim: prims::Prim) {
        self.push_undo();
        let name = scene::unique_name(prim.name(), |n| self.mesh.nodes.iter().any(|x| x.name == n) || self.edits.extras.iter().any(|e| e.name == n));
        let d = (self.base.radius * 0.3).clamp(2.0, 30.0);
        let f = self.cam.forward();
        let pos = [self.cam.pos[0] + f[0] * d, self.cam.pos[1] + f[1] * d, self.cam.pos[2] + f[2] * d];
        let (s, col) = if prim == prims::Prim::Terrain {
            ((self.base.radius * 0.8).max(8.0), [0.42, 0.55, 0.34])
        } else if prim == prims::Prim::Player {
            (1.0, [0.9, 0.55, 0.2])
        } else if prim == prims::Prim::Light {
            (1.0, [1.0, 0.95, 0.8])
        } else if prim == prims::Prim::Tree {
            ((self.base.radius * 0.4).max(2.0), [1.0, 1.0, 1.0])
        } else {
            ((self.base.radius * 0.1).max(0.5), scene::DEFAULT_PRIM_COLOR)
        };
        self.edits.extras.push(Extra { light: Default::default(), tree: Default::default(), player: Default::default(), scatter: Default::default(), texture: None, particles: Default::default(), terrain: Default::default(), prim, name: name.clone(), xform: Xform { pos, rot: [0.0; 3], scale: [s; 3] }, color: Some(col), alpha: 1.0 });
        if prim == prims::Prim::Tree {
            if let Some(e) = self.edits.extras.last_mut() {
                e.texture = Some(trees::PALETTE.to_string()); // colored by the palette, not a flat color
                e.color = None;
            }
        }
        if prim == prims::Prim::Particles {
            let r = self.scene_radius();
            if let Some(e) = self.edits.extras.last_mut() {
                e.particles = particles::ParticleParams::for_scene(r);
                e.color = Some([1.0, 1.0, 1.0]);
            }
        }
        let hidden = self.hidden_by_name();
        self.rebuild(&hidden, Some(&name));
    }

    fn duplicate_selected(&mut self) {
        let nb = self.base.nodes.len();
        let js: Vec<usize> = self.ex.multi.iter().filter_map(|&s| if let NodeRef::Extra(j) = node_ref(nb, s) { Some(j) } else { None }).collect();
        if js.is_empty() {
            return self.status_note("Duplicate works on added primitives and terrain");
        }
        self.push_undo();
        let mut taken: Vec<String> = self.mesh.nodes.iter().map(|n| n.name.clone()).collect();
        let mut names = Vec::new();
        for j in js {
            let mut e = self.edits.extras[j].clone();
            let original = e.name.clone();
            e.name = scene::unique_name(&e.name, |n| taken.iter().any(|t| t == n));
            if let Some(src) = self.edits.scripts.get(&original).cloned() {
                self.edits.scripts.insert(e.name.clone(), src); // a duplicate keeps its script
            }
            if let Some(g) = self.edits.graphs.get(&original).cloned() {
                self.edits.graphs.insert(e.name.clone(), g);
            }
            taken.push(e.name.clone());
            e.xform.pos[0] += e.xform.scale[0].abs().max(0.2);
            names.push(e.name.clone());
            self.edits.extras.push(e);
        }
        let hidden = self.hidden_by_name();
        self.rebuild(&hidden, names.last().map(|s| s.as_str()));
        self.select_names(&names);
    }

    // ---------------------------------------------------------------- context menus
    fn show_menu(&mut self, items: Vec<win32::MenuItem>, at: (i32, i32)) {
        self.window.open_popup(&items, at.0, at.1);
    }

    fn add_menu(&self) -> Vec<win32::MenuItem> {
        use win32::MenuItem as M;
        let mut v: Vec<M> = win32::add_menu_items();
        v.push(M::sep());
        v.push(M::new(win32::CMD_SCATTER_ADD, "Scatter on selected terrain").enabled(self.terrain_index().is_some()));
        v
    }

    /// Menu for the selected object(s).
    fn object_menu(&self) -> Vec<win32::MenuItem> {
        use win32::MenuItem as M;
        let nb = self.base.nodes.len();
        let is_extra = self.ex.selected.is_some_and(|s| matches!(node_ref(nb, s), NodeRef::Extra(_)));
        let name = match self.ex.multi.len() {
            0 | 1 => self.selected_name().unwrap_or_default(),
            n => format!("{n} objects"),
        };
        let has_script = self.selected_name().is_some_and(|n| self.edits.scripts.contains_key(&n));
        let tools: Vec<M> = ["None\tQ", "Move\tR", "Rotate\tT", "Scale\tY"]
            .iter()
            .enumerate()
            .map(|(i, t)| M::new(win32::CMD_GIZMO_BASE + i as u16, t).checked(self.gizmo.index() as usize == i))
            .collect();
        let mut v = vec![
            M::new(0, &name).enabled(false),
            M::sep(),
            M::new(win32::CMD_FRAME_SEL, "Frame\tF"),
            M::new(win32::CMD_DUPLICATE, "Duplicate\tCtrl+D").enabled(is_extra),
            M::new(win32::CMD_DELETE, "Delete\tDel"),
            M::new(win32::CMD_HIDE_SEL, "Hide\tH"),
            M::sep(),
            M::submenu("Gizmo", tools),
            M::new(win32::CMD_RESET_XFORM, "Reset Transform"),
            M::new(win32::CMD_CLEAR_COLOR, "Clear Color"),
            M::new(win32::CMD_TEXTURE_SET, "Set Texture..."),
            M::new(win32::CMD_TEXTURE_CLEAR, "Clear Texture"),
            M::sep(),
            M::submenu("Collision", scene::Collision::NAMES.iter().enumerate().map(|(i, n)| M::new(win32::CMD_COLL_BASE + i as u16, n).checked(self.selected_name().is_some_and(|s| self.edits.collision_of(&s).index() == i))).collect()),
            M::new(win32::CMD_SCRIPT_ADD, if has_script { "Edit Script" } else { "Add Script" }),
        ];
        if has_script {
            v.push(M::new(win32::CMD_SCRIPT_REMOVE, "Remove Script"));
        }
        v.push(M::sep());
        v.push(M::submenu("Add", self.add_menu()));
        v
    }

    /// Menu for empty space in the 3D view or the hierarchy.
    fn empty_menu(&self) -> Vec<win32::MenuItem> {
        use win32::MenuItem as M;
        let modes: Vec<M> = MODE_NAMES.iter().enumerate().map(|(i, n)| M::new(win32::CMD_MODE_BASE + i as u16, n).checked(self.view.mode as usize == i)).collect();
        vec![
            M::submenu("Add", self.add_menu()),
            M::sep(),
            M::new(win32::CMD_SELECT_ALL, "Select All\tCtrl+A"),
            M::new(win32::CMD_SHOW_ALL, "Show All\tAlt+H"),
            M::new(win32::CMD_FRAME_ALL, "Frame All\tShift+F"),
            M::new(win32::CMD_RESET_CAMERA, "Reset Camera"),
            M::sep(),
            M::submenu("View mode", modes),
            M::new(win32::CMD_GRID, "Grid").checked(self.view.grid),
            M::new(win32::CMD_AFFINE, "Affine Texture Mapping").checked(self.view.affine),
            M::new(win32::CMD_CULL, "Backface Culling").checked(self.view.cull),
            M::new(win32::CMD_SELECT_ENV, "Environment Settings"),
        ]
    }

    fn explorer_menu(&self) -> Vec<win32::MenuItem> {
        use win32::MenuItem as M;
        let sel = self.window.explorer_selected().and_then(|r| self.explorer_items.get(r)).cloned();
        let label = match &sel {
            Some((n, true)) if n == ".." => "Go Up",
            Some((_, true)) => "Open Folder",
            Some((n, false)) if blend::is_model(n) => "Import Model",
            Some((n, false)) if project::is_project_file(n) => "Open Project",
            Some(_) => "Use as Texture",
            None => "Open",
        };
        vec![M::new(win32::CMD_EXPLORER_OPEN, label).enabled(sel.is_some()), M::sep(), M::new(win32::CMD_EXPLORER_UP, "Up one Folder"), M::new(win32::CMD_EXPLORER_REFRESH, "Refresh")]
    }

    /// Right click (without dragging) on the 3D view: menu for the object under the cursor, or for empty space.
    fn canvas_context_menu(&mut self, pos: (i32, i32)) {
        let items = match self.pick(pos.0, pos.1) {
            Some(h) => {
                if !self.ex.multi.contains(&h) {
                    self.set_selected(Some(h));
                }
                self.object_menu()
            }
            None => self.empty_menu(),
        };
        let at = self.window.look_anchor();
        self.show_menu(items, at);
    }

    // ---------------------------------------------------------------- lights
    /// The lights that count right now, packed for the shader: (count, positions, colors, directions).
    fn gather_lights(&self) -> (i32, Vec<f32>, Vec<f32>, Vec<f32>) {
        let nb = self.base.nodes.len();
        let (mut pos, mut col, mut dir) = (Vec::new(), Vec::new(), Vec::new());
        let mut count = 0;
        for (j, e) in self.edits.extras.iter().enumerate() {
            let node = nb + 2 + j;
            if e.prim != prims::Prim::Light || count >= light::MAX_LIGHTS || self.rt.deleted[node] || !self.ex.checked[node] || self.script_hidden.contains(&node) {
                continue;
            }
            let x = &self.rt.xf[node];
            let aim = math::transform_dir(&math::rot_matrix(x.rot), [0.0, -1.0, 0.0]);
            let c = self.rt.color[node].unwrap_or([1.0; 3]);
            let l = &e.light;
            let kind = l.kind.index() as f32;
            pos.extend_from_slice(&[x.pos[0], x.pos[1], x.pos[2], kind]);
            col.extend_from_slice(&[c[0] * l.intensity, c[1] * l.intensity, c[2] * l.intensity, l.range]);
            dir.extend_from_slice(&[aim[0], aim[1], aim[2], (l.angle * 0.5).to_radians().cos()]);
            count += 1;
        }
        (count as i32, pos, col, dir)
    }

    fn light_index(&self) -> Option<usize> {
        let s = self.ex.selected?;
        match node_ref(self.base.nodes.len(), s) {
            NodeRef::Extra(j) if self.edits.extras[j].prim == prims::Prim::Light => Some(j),
            _ => None,
        }
    }

    fn refresh_light(&self, j: usize) {
        let (w, e) = (&self.window, &self.edits.extras[j]);
        let l = &e.light;
        w.combo_set(win32::CB_LKIND, &light::LightKind::NAMES, l.kind.index());
        w.set_text(win32::ED_LINT, &fmt_f(l.intensity));
        w.set_text(win32::ED_LRANGE, &fmt_f(l.range));
        w.set_text(win32::ED_LANGLE, &fmt_f(l.angle));
        w.set_enabled(win32::ED_LRANGE, l.kind != light::LightKind::Sun);
        w.set_enabled(win32::ED_LANGLE, l.kind == light::LightKind::Spot);
        let c = e.color.unwrap_or([1.0; 3]);
        w.set_text(win32::ST_LINFO, &format!("Color: #{:02X}{:02X}{:02X}", (c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8));
    }

    fn on_light_edit(&mut self, id: u16, text: &str) {
        let (Some(j), Some(sel)) = (self.light_index(), self.ex.selected) else { return };
        let Ok(v) = text.trim().replace(',', ".").parse::<f32>() else { return };
        if !v.is_finite() {
            return;
        }
        if self.edit_key != Some((sel, id)) {
            self.push_undo();
            self.edit_key = Some((sel, id));
        }
        let l = &mut self.edits.extras[j].light;
        match id {
            win32::ED_LINT => l.intensity = v,
            win32::ED_LRANGE => l.range = v,
            _ => l.angle = v,
        }
        l.sanitize();
    }

    fn on_light_combo(&mut self) {
        let Some(j) = self.light_index() else { return };
        let sel = self.window.combo_selected(win32::CB_LKIND);
        self.push_undo();
        self.edits.extras[j].light.kind = light::LightKind::from_index(sel);
        self.regenerate(); // the marker shows the kind
        self.edit_key = None;
    }

    // ---------------------------------------------------------------- tree objects
    fn tree_index(&self) -> Option<usize> {
        let s = self.ex.selected?;
        match node_ref(self.base.nodes.len(), s) {
            NodeRef::Extra(j) if self.edits.extras[j].prim == prims::Prim::Tree => Some(j),
            _ => None,
        }
    }

    fn refresh_tree(&self, j: usize) {
        let (w, e) = (&self.window, &self.edits.extras[j]);
        let t = &e.tree;
        w.combo_set(win32::CB_TKIND, &trees::TreeKind::NAMES, t.kind.index());
        w.set_text(win32::ED_TRUNK, &fmt_f(t.trunk));
        w.set_text(win32::ED_TRWIDTH, &fmt_f(t.trunk_width));
        w.set_text(win32::ED_CROWN, &fmt_f(t.crown));
        w.set_text(win32::ED_DETAIL, &t.detail.to_string());
        w.set_text(win32::ED_TRSEED, &t.seed.to_string());
        let tex = match e.texture.as_deref() {
            Some(trees::PALETTE) => "tree palette".to_string(),
            Some(p) => Path::new(p).file_name().map_or(p.to_string(), |f| f.to_string_lossy().into_owned()),
            None => "none".to_string(),
        };
        let c = e.color.unwrap_or([1.0; 3]);
        w.set_text(win32::ST_TRINFO, &format!("Texture: {tex}   Color: #{:02X}{:02X}{:02X}", (c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8));
    }

    fn on_tree_edit(&mut self, id: u16, text: &str) {
        let (Some(j), Some(sel)) = (self.tree_index(), self.ex.selected) else { return };
        let Ok(v) = text.trim().replace(',', ".").parse::<f32>() else { return };
        if !v.is_finite() {
            return;
        }
        if self.edit_key != Some((sel, id)) {
            self.push_undo();
            self.edit_key = Some((sel, id));
        }
        let t = &mut self.edits.extras[j].tree;
        match id {
            win32::ED_TRUNK => t.trunk = v,
            win32::ED_TRWIDTH => t.trunk_width = v,
            win32::ED_CROWN => t.crown = v,
            win32::ED_DETAIL => t.detail = v.max(0.0) as u32,
            _ => t.seed = v.max(0.0) as u32,
        }
        t.sanitize();
        self.regenerate();
    }

    fn on_tree_combo(&mut self) {
        let Some(j) = self.tree_index() else { return };
        let sel = self.window.combo_selected(win32::CB_TKIND);
        self.push_undo();
        self.edits.extras[j].tree.kind = trees::TreeKind::from_index(sel);
        self.regenerate();
        self.edit_key = None;
    }

    // ---------------------------------------------------------------- gameplay objects
    /// Adds a ready-made gameplay object: a shape with its size, color, collision mode and script.
    fn add_prefab(&mut self, i: usize) {
        let p = &prefabs::PREFABS[i];
        self.add_primitive(p.prim);
        let Some(last) = self.edits.extras.len().checked_sub(1) else { return };
        let taken: Vec<String> = self.mesh.nodes.iter().map(|n| n.name.clone()).chain(self.edits.extras[..last].iter().map(|e| e.name.clone())).collect();
        let name = scene::unique_name(p.name, |n| taken.iter().any(|t| t == n));
        let e = &mut self.edits.extras[last];
        e.name = name.clone();
        e.xform.scale = p.scale;
        e.color = Some(p.color);
        e.alpha = p.alpha;
        if let Some(l) = p.light {
            e.light = l;
        }
        self.edits.set_collision(&name, p.collision);
        if !p.script.is_empty() {
            self.edits.scripts.insert(name.clone(), p.script.to_string());
        }
        let hidden = self.hidden_by_name();
        self.rebuild(&hidden, Some(&name));
        self.status_note(&format!("Added {name}{}", if p.script.is_empty() { "" } else { " with a script (see the SCRIPT tab)" }));
    }

    // ---------------------------------------------------------------- collision
    /// Sets how the player collides with every selected object.
    fn set_collision(&mut self, c: scene::Collision) {
        let names: Vec<String> = self.selected_objects().into_iter().map(|i| self.mesh.nodes[i].name.clone()).collect();
        if names.is_empty() || names.iter().all(|n| self.edits.collision_of(n) == c) {
            return;
        }
        self.push_undo();
        for n in &names {
            self.edits.set_collision(n, c);
        }
        self.objects_dirty = true; // water ripples
        self.details_dirty = true;
        self.edit_key = None;
    }

    // ---------------------------------------------------------------- scripts
    /// Opens an object's script in its own tab above the scene.
    fn open_script_tab(&mut self, name: &str) {
        let code = self.edits.scripts.get(name).cloned().unwrap_or_default();
        self.window.open_script_tab(name, &code, self.edits.graphs.get(name).cloned());
        self.sync_script_badges();
    }

    /// Open script tabs show what the edits say (after undo / redo, loading ...).
    fn sync_script_tabs(&mut self) {
        for name in self.window.open_script_tabs() {
            if self.find_object(&name).is_none() {
                self.window.close_script_tab(&name);
                continue;
            }
            let code = self.edits.scripts.get(&name).cloned().unwrap_or_default();
            self.window.update_script_tab(&name, &code, self.edits.graphs.get(&name).cloned());
        }
        self.sync_script_badges();
    }

    /// Objects with a script get a small button in the hierarchy.
    fn sync_script_badges(&self) {
        for (i, n) in self.mesh.nodes.iter().enumerate() {
            if self.ex.handles.get(i).is_some_and(|&h| h != 0) {
                self.window.tree_set_badge(self.ex.handles[i], n.kind == obj::Kind::Object && self.edits.scripts.get(&n.name).is_some_and(|s| !s.trim().is_empty()));
            }
        }
    }

    /// What the script tabs did: code typed, blocks wired, a visual script turned into code.
    fn handle_script_events(&mut self) {
        if let Some(name) = self.window.take_script_open() {
            self.open_script_tab(&name);
        }
        for ev in self.window.take_script_events() {
            let object = match &ev {
                ui::ScriptEvent::Code { object, .. } | ui::ScriptEvent::Graph { object, .. } | ui::ScriptEvent::ToCode { object } => object.clone(),
            };
            if self.play.is_some() || self.find_object(&object).is_none() {
                continue;
            }
            // typing is one undo step per object and burst
            if self.script_undo.as_ref().is_none_or(|(o, t)| *o != object || t.elapsed() > Duration::from_millis(1500)) {
                self.push_undo();
            }
            self.script_undo = Some((object.clone(), Instant::now()));
            match ev {
                ui::ScriptEvent::Code { text, .. } => {
                    if text.trim().is_empty() {
                        self.edits.scripts.remove(&object);
                    } else {
                        self.edits.scripts.insert(object.clone(), text);
                    }
                }
                ui::ScriptEvent::Graph { graph, code, .. } => {
                    self.edits.graphs.insert(object.clone(), graph);
                    if let Ok(code) = code {
                        self.edits.scripts.insert(object.clone(), code);
                    }
                }
                ui::ScriptEvent::ToCode { .. } => {
                    self.edits.graphs.remove(&object);
                }
            }
        }
        self.sync_script_badges();
    }

    /// The object a script would belong to: the selected real object (not a collection / environment).
    fn script_target(&self) -> Option<String> {
        let s = self.ex.selected?;
        let n = &self.mesh.nodes[s];
        (n.kind == obj::Kind::Object && !self.rt.deleted[s]).then(|| n.name.clone())
    }

    /// Shows the selected object's script in the editor tab (only when that tab is showing).
    fn refresh_script_editor(&self) {
        let w = &self.window;
        if w.bottom_tab() != 1 {
            return;
        }
        let Some(name) = self.script_target() else {
            w.set_text(win32::ED_SCRIPT, "");
            w.set_script_status("Select an object in the hierarchy to give it a script", true);
            return;
        };
        let src = self.edits.scripts.get(&name).cloned().unwrap_or_default();
        if w.get_text_long(win32::ED_SCRIPT).replace("\r\n", "\n") != src {
            w.set_text(win32::ED_SCRIPT, &src.replace('\n', "\r\n"));
        }
        self.show_script_status(&name, &src);
    }

    fn show_script_status(&self, name: &str, src: &str) {
        if src.trim().is_empty() {
            self.window.set_script_status(&format!("{name}: no script yet. Write code below (or use the template: right-click the object > Add Script)."), true);
        } else {
            match script::check(src) {
                Ok(()) => self.window.set_script_status(&format!("{name}: script OK, it runs when you press Play"), true),
                Err(e) => self.window.set_script_status(&format!("{name}: {e}"), false),
            }
        }
    }

    /// The code box changed: store the text as the object's script.
    fn on_script_edit(&mut self) {
        let (Some(name), Some(sel)) = (self.script_target(), self.ex.selected) else { return };
        let text = self.window.get_text_long(win32::ED_SCRIPT).replace("\r\n", "\n");
        if text == self.edits.scripts.get(&name).cloned().unwrap_or_default() {
            return;
        }
        if self.edit_key != Some((sel, win32::ED_SCRIPT)) {
            self.push_undo();
            self.edit_key = Some((sel, win32::ED_SCRIPT));
        }
        if text.trim().is_empty() {
            self.edits.scripts.remove(&name);
        } else {
            self.edits.scripts.insert(name.clone(), text.clone());
        }
        self.show_script_status(&name, &text);
    }

    /// Runs `on start` (start = true) or the per-frame blocks of every script.
    fn run_scripts(&mut self, ps: &mut PlayState, mode: ScriptRun) {
        if ps.scripts.is_empty() {
            return;
        }
        let mut now = vec![false; 256];
        for (vk, v) in now.iter_mut().enumerate().skip(1) {
            *v = win32::key_down(vk as i32);
        }
        let pressed: Vec<bool> = (0..256).map(|v| now[v] && !ps.keys_prev[v]).collect();
        let mut scripts = std::mem::take(&mut ps.scripts);
        let mut host = PlayHost { app: self, walker: &mut ps.walker, yaw: &mut ps.yaw, held: &now, pressed: &pressed, gravity: ps.params.gravity, time: ps.t0.elapsed().as_secs_f64(), changed: false, world_dirty: false };
        let mut failed = Vec::new();
        for (i, (name, s)) in scripts.iter_mut().enumerate() {
            let r = match &mode {
                ScriptRun::Start => s.start(name, &mut host),
                ScriptRun::Update(dt) => s.update(name, *dt as f64, &mut host),
                ScriptRun::Events(list) => list.iter().filter(|(n, _)| n.as_str() == name.as_str()).try_for_each(|(_, leave)| s.touch(name, *leave, &mut host)),
            };
            if let Err(e) = r {
                failed.push((i, format!("Script error in {name}: {e}")));
            }
        }
        let (changed, world_dirty) = (host.changed, host.world_dirty);
        for (i, msg) in failed.into_iter().rev() {
            scripts.remove(i); // a script that failed stops running; the others go on
            self.status_note(&msg);
        }
        ps.scripts = scripts;
        ps.keys_prev = now;
        if changed {
            self.rt = compute_runtime(self.base.nodes.len(), &self.mesh, &self.edits);
            self.objects_dirty = true;
            ps.wrote = true;
            ps.world_dirty |= world_dirty;
        }
    }

    fn script_xform(&self, i: usize) -> Xform {
        let nb = self.base.nodes.len();
        match node_ref(nb, i) {
            NodeRef::Base => self.edits.base.get(&self.base.nodes[i].name).and_then(|e| e.xform).unwrap_or(Xform::at(self.base.nodes[i].center)),
            NodeRef::Extra(j) => self.edits.extras[j].xform,
            _ => Xform::at([0.0; 3]),
        }
    }

    /// Stores a node's transform without touching the nodes inside it.
    fn store_xform(&mut self, i: usize, x: Xform) {
        let nb = self.base.nodes.len();
        match node_ref(nb, i) {
            NodeRef::Base => self.edits.base.entry(self.base.nodes[i].name.clone()).or_default().xform = Some(x),
            NodeRef::Extra(j) => self.edits.extras[j].xform = x,
            _ => {}
        }
    }

    /// Sets a node's transform; the objects inside it move along.
    fn script_set_xform(&mut self, i: usize, x: Xform) {
        let old = self.script_xform(i);
        self.store_xform(i, x);
        self.push_followers(i, old, x, &[]);
    }

    /// The objects inside node `i` follow when it goes from `old` to `new` (as if they were glued to it).
    /// `listed` are nodes that are being moved on their own in the same step; they and their contents are skipped.
    fn push_followers(&mut self, i: usize, old: Xform, new: Xform, listed: &[usize]) {
        if old == new {
            return;
        }
        for d in hier::descendants(&self.mesh.nodes, i) {
            if self.mesh.nodes[d].kind != obj::Kind::Object {
                continue;
            }
            if listed.iter().any(|&m| m != i && hier::is_ancestor_or_self(&self.mesh.nodes, m, d) && hier::is_ancestor_or_self(&self.mesh.nodes, i, m)) {
                continue;
            }
            let c = self.script_xform(d);
            self.store_xform(d, follow(&old, &new, &c));
        }
    }

    /// Drag and drop in the explorer: puts `node` at `place`. Returns false when that is not allowed.
    fn reparent(&mut self, node: usize, place: hier::Place) -> bool {
        if self.play.is_some() || node == self.base.nodes.len() + 1 {
            return false;
        }
        match hier::moved(&self.mesh.nodes, &self.edits.hier, node, place) {
            Some(h) if h != self.edits.hier => {
                self.push_undo();
                self.edits.hier = h;
                self.refresh();
                true
            }
            _ => false,
        }
    }

    fn script_color(&self, i: usize) -> [f32; 3] {
        let nb = self.base.nodes.len();
        let own = match node_ref(nb, i) {
            NodeRef::Base => self.edits.base.get(&self.base.nodes[i].name).and_then(|e| e.color),
            NodeRef::Extra(j) => self.edits.extras[j].color,
            _ => None,
        };
        own.or(self.rt.color[i]).unwrap_or([0.8, 0.8, 0.8])
    }

    fn script_set_color(&mut self, i: usize, c: [f32; 3]) {
        let nb = self.base.nodes.len();
        match node_ref(nb, i) {
            NodeRef::Base => self.edits.base.entry(self.base.nodes[i].name.clone()).or_default().color = Some(c),
            NodeRef::Extra(j) => self.edits.extras[j].color = Some(c),
            _ => {}
        }
    }

    fn script_alpha(&self, i: usize) -> f32 {
        let nb = self.base.nodes.len();
        let own = match node_ref(nb, i) {
            NodeRef::Base => self.edits.base.get(&self.base.nodes[i].name).and_then(|e| e.alpha),
            NodeRef::Extra(j) => Some(self.edits.extras[j].alpha),
            _ => None,
        };
        own.unwrap_or(self.rt.alpha[i])
    }

    fn script_set_alpha(&mut self, i: usize, a: f32) {
        let nb = self.base.nodes.len();
        match node_ref(nb, i) {
            NodeRef::Base => self.edits.base.entry(self.base.nodes[i].name.clone()).or_default().alpha = Some(a),
            NodeRef::Extra(j) => self.edits.extras[j].alpha = a,
            _ => {}
        }
    }

    // ---------------------------------------------------------------- play mode
    /// World-space triangles everything solid in the scene is made of (the player's own body,
    /// particles and grass excluded).
    fn collision_world(&self, skip_node: usize) -> play::World {
        let nb = self.base.nodes.len();
        let solid: Vec<Option<math::Mat4>> = self
            .mesh
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let mut ok = n.kind == obj::Kind::Object && self.ex.checked[i] && !self.rt.deleted[i] && i != skip_node && !self.script_hidden.contains(&i) && self.edits.collision_of(&n.name).blocks();
                if let NodeRef::Extra(j) = node_ref(nb, i) {
                    let e = &self.edits.extras[j];
                    ok &= !matches!(e.prim, prims::Prim::Player | prims::Prim::Particles | prims::Prim::Light) && !(e.prim == prims::Prim::Scatter && e.scatter.shape == scatter::Shape::Grass);
                }
                ok.then(|| {
                    let x = &self.rt.xf[i];
                    math::model_matrix(x.pos, x.rot, x.scale, n.center)
                })
            })
            .collect();
        let (data, mut tris) = (&self.mesh.data, Vec::new());
        for t in self.mesh.indices.chunks_exact(3) {
            let id = data[t[0] as usize * obj::FLOATS + obj::FLOATS - 1] as usize;
            let Some(m) = &solid[id] else { continue };
            let v = |k: usize| {
                let b = t[k] as usize * obj::FLOATS;
                math::transform_point(m, [data[b], data[b + 1], data[b + 2]])
            };
            tris.push([v(0), v(1), v(2)]);
        }
        play::World::new(tris)
    }

    /// World-space boxes (min xyz, max xyz) of the objects with the given collision mode.
    fn collision_boxes(&self, mode: scene::Collision) -> Vec<(usize, [f32; 6])> {
        let mats: Vec<Option<math::Mat4>> = self
            .mesh
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let on = n.kind == obj::Kind::Object && self.ex.checked[i] && !self.rt.deleted[i] && !self.script_hidden.contains(&i) && self.edits.collision_of(&n.name) == mode;
                on.then(|| {
                    let x = &self.rt.xf[i];
                    math::model_matrix(x.pos, x.rot, x.scale, n.center)
                })
            })
            .collect();
        let mut boxes: HashMap<usize, [f32; 6]> = HashMap::new();
        let data = &self.mesh.data;
        for t in self.mesh.indices.chunks_exact(3) {
            let id = data[t[0] as usize * obj::FLOATS + obj::FLOATS - 1] as usize;
            let Some(m) = &mats[id] else { continue };
            let b = boxes.entry(id).or_insert([f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN]);
            for k in 0..3 {
                let o = t[k] as usize * obj::FLOATS;
                let p = math::transform_point(m, [data[o], data[o + 1], data[o + 2]]);
                for a in 0..3 {
                    b[a] = b[a].min(p[a]);
                    b[a + 3] = b[a + 3].max(p[a]);
                }
            }
        }
        let mut v: Vec<(usize, [f32; 6])> = boxes.into_iter().collect();
        v.sort_by_key(|(i, _)| *i);
        v
    }

    fn start_play(&mut self) {
        let nb = self.base.nodes.len();
        let found = self.edits.extras.iter().enumerate().find(|(j, e)| e.prim == prims::Prim::Player && self.ex.checked[nb + 2 + j]);
        let Some((j, e)) = found else {
            let any = self.edits.extras.iter().any(|e| e.prim == prims::Prim::Player);
            return self.status_note(if any { "The Player is hidden: tick it in the explorer to play" } else { "Add a Player object first (Add > Player), then press Play" });
        };
        let (node, mut params) = (nb + 2 + j, e.player.clone());
        params.sanitize();
        let params = params.scaled(); // play with every size-related setting multiplied by the player's Size
        let sound = if params.step_sound.is_empty() {
            None
        } else {
            match player::load_wav_scaled(&params.step_sound, params.step_volume) {
                Ok(s) => Some(s),
                Err(err) => {
                    self.status_note(&err);
                    None
                }
            }
        };
        // compile every script first: a script with a mistake stops Play and shows the error
        let mut scripts = Vec::new();
        let mut names: Vec<String> = self.edits.scripts.keys().cloned().collect();
        names.sort();
        for name in names {
            let Some(n) = self.mesh.nodes.iter().enumerate().position(|(i, n)| n.name == name && n.kind == obj::Kind::Object && !self.rt.deleted[i]) else { continue };
            match script::Script::compile(&self.edits.scripts[&name]) {
                Ok(s) => scripts.push((name, s)),
                Err(e) => {
                    self.window.show_script_tab(true);
                    self.set_selected(Some(n));
                    self.status_note(&format!("Script error in {name}: {e}"));
                    return;
                }
            }
        }
        let x = self.rt.xf[node];
        let ps = PlayState {
            walker: play::Walker::new(x.pos),
            yaw: -x.rot[1].to_radians(),
            pitch: 0.0,
            spawn: x.pos,
            world: self.collision_world(node),
            world_key: (self.mesh.data.len(), self.mesh.indices.len(), self.mesh.data.as_ptr() as usize),
            params,
            sound,
            node,
            saved_cam: self.cam,
            saved_gizmo: self.gizmo,
            scripts,
            keys_prev: vec![false; 256],
            t0: Instant::now(),
            backup: self.edits.clone(),
            wrote: false,
            world_dirty: false,
            world_t: Instant::now(),
            triggers: self.collision_boxes(scene::Collision::Trigger),
            inside: HashSet::new(),
            water: self.collision_boxes(scene::Collision::Water),
        };
        let mut ps = ps;
        self.gizmo = Gizmo::Off;
        if self.ex.handles[node] != 0 {
            self.window.tree_set_checked(self.ex.handles[node], false); // the player's stand-in is not drawn while playing
        }
        self.ex.checked[node] = false;
        self.objects_dirty = true;
        self.poll_t = Instant::now() - Duration::from_secs(1);
        if self.look_active {
            self.window.look_end();
        }
        self.window.set_playing(true);
        self.window.look_begin();
        self.look_active = true;
        self.cursor_free = false;
        self.cursor_override = None;
        self.tab_free = false;
        self.tab_prev = win32::key_down(0x09);
        self.window.set_text(win32::CMD_PLAY, "Stop");
        self.script_hidden.clear();
        self.script_vars.clear();
        self.hud.clear();
        let nb = self.base.nodes.len();
        self.play_hidden = self.edits.extras.iter().enumerate().filter(|(_, e)| e.prim == prims::Prim::Light).map(|(j, _)| nb + 2 + j).collect();
        self.objects_dirty = true;
        self.run_scripts(&mut ps, ScriptRun::Start);
        self.play = Some(ps);
    }

    fn stop_play(&mut self) {
        let Some(ps) = self.play.take() else { return };
        self.window.stop_sound();
        self.window.set_playing(false);
        self.cursor_free = false;
        self.cursor_override = None;
        self.tab_free = false;
        if self.look_active {
            self.window.look_end();
            self.look_active = false;
        }
        self.cam = ps.saved_cam;
        self.gizmo = ps.saved_gizmo;
        if self.ex.handles[ps.node] != 0 {
            self.window.tree_set_checked(self.ex.handles[ps.node], true);
        }
        self.ex.checked[ps.node] = true;
        self.script_hidden.clear();
        self.hud.clear();
        self.play_hidden.clear();
        self.underwater = false;
        if ps.wrote {
            // scripts moved / recoloured things: put the scene back the way it was
            self.edits = ps.backup;
            hier::apply(&mut self.mesh.nodes, &self.base.nodes, &self.edits.hier);
            self.rt = compute_runtime(self.base.nodes.len(), &self.mesh, &self.edits);
            self.details_dirty = true;
        }
        self.objects_dirty = true;
        self.poll_t = Instant::now() - Duration::from_secs(1);
        self.window.set_text(win32::CMD_PLAY, "Play");
    }

    /// One frame of play mode: mouse look, walking, footsteps, and the camera follows the player's eyes.
    fn play_frame(&mut self, dt: f32) {
        let Some(mut ps) = self.play.take() else { return };
        let (dx, dy) = if self.cursor_free { (0, 0) } else { self.window.look_delta() };
        let sens = ps.params.mouse_sens.to_radians();
        ps.yaw += dx as f32 * sens;
        ps.pitch = (ps.pitch - dy as f32 * sens).clamp(-1.5, 1.5);
        self.run_scripts(&mut ps, ScriptRun::Update(dt.min(0.05)));
        if ps.world_dirty && ps.world_t.elapsed() > Duration::from_millis(250) {
            ps.world = self.collision_world(ps.node); // something moved: refresh what the player collides with
            ps.triggers = self.collision_boxes(scene::Collision::Trigger);
            ps.water = self.collision_boxes(scene::Collision::Water);
            ps.world_dirty = false;
            ps.world_t = Instant::now();
        }
        let k = |vk: i32| if win32::key_down(vk) { 1.0f32 } else { 0.0 };
        let input = (k(0x57) - k(0x53), k(0x44) - k(0x41));
        let key = (self.mesh.data.len(), self.mesh.indices.len(), self.mesh.data.as_ptr() as usize);
        if key != ps.world_key {
            ps.world = self.collision_world(ps.node); // e.g. new terrain chunks were streamed in
            ps.triggers = self.collision_boxes(scene::Collision::Trigger);
            ps.water = self.collision_boxes(scene::Collision::Water);
            ps.world_key = key;
        }
        // water: how deep the body is in a pool, and whether the camera is under the surface
        let (feet, eye) = (ps.walker.pos, ps.walker.eye(&ps.params));
        let (mut depth, mut under) = (0.0f32, false);
        for (_, b) in &ps.water {
            if feet[0] >= b[0] && feet[0] <= b[3] && feet[2] >= b[2] && feet[2] <= b[5] {
                let (top, bottom) = (b[4].min(feet[1] + ps.params.height), b[1].max(feet[1]));
                depth = depth.max((top - bottom).max(0.0));
                under |= eye[1] < b[4] && eye[1] > b[1];
            }
        }
        self.underwater = under;
        let r = ps.walker.step(&ps.params, input, ps.yaw, win32::key_down(0x10), win32::key_down(0x20), depth, dt.min(0.05), &ps.world);
        if r.stepped {
            if let Some(s) = &ps.sound {
                self.window.play_sound(s);
            }
        }
        // trigger objects: the player's body against each box -> on touch / on leave
        let (feet, r, h) = (ps.walker.pos, ps.params.radius, ps.params.height);
        let mut events: Vec<(String, bool)> = Vec::new();
        let overlapping: Vec<usize> = ps
            .triggers
            .iter()
            .filter(|(_, b)| feet[0] >= b[0] - r && feet[0] <= b[3] + r && feet[2] >= b[2] - r && feet[2] <= b[5] + r && feet[1] <= b[4] && feet[1] + h >= b[1])
            .map(|(i, _)| *i)
            .collect();
        for &i in &overlapping {
            if ps.inside.insert(i) {
                events.push((self.mesh.nodes[i].name.clone(), false));
            }
        }
        let gone: Vec<usize> = ps.inside.iter().copied().filter(|i| !overlapping.contains(i)).collect();
        for i in gone {
            ps.inside.remove(&i);
            if i < self.mesh.nodes.len() {
                events.push((self.mesh.nodes[i].name.clone(), true));
            }
        }
        if !events.is_empty() {
            self.run_scripts(&mut ps, ScriptRun::Events(events));
        }
        let fall_limit = ps.spawn[1] - self.scene_radius() * 10.0 - 100.0;
        if ps.walker.pos[1] < fall_limit {
            ps.walker = play::Walker::new(ps.spawn); // fell out of the world
        }
        self.cam = Cam { yaw: ps.yaw, pitch: ps.pitch, pos: ps.walker.eye(&ps.params) };
        self.play = Some(ps);
    }

    fn hide_selected(&mut self) {
        for &s in &self.ex.multi {
            if self.ex.handles[s] != 0 {
                self.window.tree_set_checked(self.ex.handles[s], false); // the next explorer poll applies it
            }
        }
        self.poll_t = Instant::now() - Duration::from_secs(1);
    }

    fn show_all(&mut self) {
        for (i, &h) in self.ex.handles.iter().enumerate() {
            if h != 0 {
                self.window.tree_set_checked(h, true);
                self.ex.checked[i] = true;
            }
        }
        self.objects_dirty = true;
    }

    /// Whether the mouse should be a pointer right now: a script's `cursor(..)` wins, otherwise the GUI mode decides.
    fn want_cursor_free(&mut self) -> bool {
        if let Some(v) = self.cursor_override {
            return v;
        }
        let tab = win32::key_down(0x09);
        if tab && !self.tab_prev {
            self.tab_free = !self.tab_free;
        }
        self.tab_prev = tab;
        match self.edits.gui_mode {
            gui::Mode::Look => false,
            gui::Mode::Cursor => true,
            gui::Mode::Toggle => self.tab_free,
            gui::Mode::Hold => win32::key_down(0x11),
        }
    }

    /// Draws on top of the 3D view: the orientation widget in the editor, the HUD text in a compiled game.
    fn draw_overlay(&self, c: &mut ui::style::Ctx, vp: [f32; 4]) {
        if self.play.is_none() && !self.game_mode {
            self.draw_compass(c, vp);
            return;
        }
        if self.play.is_some() && !self.edits.gui.is_empty() {
            let hover = if self.cursor_free { gui::hit_button(&self.edits.gui, [0.0, 0.0, vp[2], vp[3]], self.gui_mouse) } else { None };
            gui::draw(c, &self.edits.gui, vp, hover, self.gui_down);
            if self.cursor_override.is_none() && self.edits.gui.iter().any(|e| e.visible && e.kind == gui::Kind::Button) {
                if let Some(text) = self.edits.gui_mode.hint(self.cursor_free) {
                    gui::hint(c, vp, text);
                }
            }
        }
        if self.game_mode {
            if self.build.show_fps {
                let s = c.s;
                let text = format!("{:.0} fps", self.fps);
                let w = c.f.bold.measure(&text);
                let font = &c.f.bold;
                c.text_in(font, &text, vp[0] + vp[2] - w - 15.0 * s, vp[1] + 11.0 * s, 24.0 * s, [0.0, 0.0, 0.0, 0.7]);
                c.text_in(font, &text, vp[0] + vp[2] - w - 16.0 * s, vp[1] + 10.0 * s, 24.0 * s, [1.0, 1.0, 1.0, 1.0]);
            }
            if !self.hud.is_empty() {
                let s = c.s;
                let font = &c.f.bold;
                c.text_in(font, &self.hud, vp[0] + 17.0 * s, vp[1] + 11.0 * s, 24.0 * s, [0.0, 0.0, 0.0, 0.7]);
                c.text_in(font, &self.hud, vp[0] + 16.0 * s, vp[1] + 10.0 * s, 24.0 * s, [1.0, 1.0, 1.0, 1.0]);
            }
        }
    }

    /// "Compile Game...": asks where to put the executable and builds it.
    fn compile_command(&mut self) {
        if self.play.is_some() {
            return self.status_note("Stop playing first, then compile");
        }
        let stem = Path::new(&self.model_path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "game".into());
        // the build settings come first, then the place for the executable
        let Some((values, texts)) = self.window.form_dialog("Build settings", "Build", self.build.specs(&stem)) else { return };
        self.build = build::BuildSettings::from_values(&values, &texts);
        let Some(out) = self.window.file_dialog(true, &[("Game (*.exe)", "*.exe")], "exe", &format!("{}.exe", if self.build.name.is_empty() { stem.as_str() } else { self.build.name.as_str() })) else { return };
        match self.compile_game(Path::new(&out)) {
            Ok(summary) => self.window.message(&summary),
            Err(e) => self.window.message(&format!("Could not compile the game:\n\n{e}")),
        }
    }

    /// Builds a stand-alone game: this program with the scene, atlas, images, sounds and scripts attached.
    fn compile_game(&mut self, out: &Path) -> Result<String, String> {
        if !self.edits.extras.iter().any(|e| e.prim == prims::Prim::Player) {
            return Err("The scene has no Player object, so there is nothing to play. Add one with Add > Player.".into());
        }
        let project_dir = self.project_path.as_ref().and_then(|p| std::path::absolute(p).ok()).and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let mut packer = Packer { pak: pak::Pak::default(), vdir: PathBuf::from(r"C:\__polystation_pak__"), done: HashMap::new(), base: project_dir };
        let mut st = self.snapshot();
        for e in st.edits.base.values_mut() {
            if let Some(v) = e.texture.as_ref().and_then(|t| packer.add(t)) {
                e.texture = Some(v);
            }
        }
        for e in st.edits.extras.iter_mut() {
            if let Some(v) = e.texture.as_ref().and_then(|t| packer.add(t)) {
                e.texture = Some(v);
            }
            if !e.player.step_sound.is_empty() {
                if let Some(v) = packer.add(&e.player.step_sound) {
                    e.player.step_sound = v;
                }
            }
        }
        if let Some(v) = st.edits.env.sky.texture.as_ref().and_then(|t| packer.add(t)) {
            st.edits.env.sky.texture = Some(v);
        }
        // sounds that scripts play: the files are packed and the script points at the copy
        for src in st.edits.scripts.values_mut() {
            *src = rewrite_sounds(src, &mut |path| packer.add(path).map(|v| packer.relative(&v)));
        }
        (st.model, st.atlas, st.imports, st.selected, st.camera) = ("game".into(), None, Vec::new(), None, None);
        if st.build.name.is_empty() {
            st.build.name = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Game".into());
        }
        let mut pak = std::mem::take(&mut packer.pak);
        let files = pak.entries.len();
        pak.add("project.polystation", project::to_text(&packer.vdir, &st).into_bytes());
        pak.add("mesh.bin", pak::mesh_to_bytes(&self.base));
        pak.add("atlas.bin", pak::atlas_to_bytes(&self.base_atlas.clone().unwrap_or_else(atlas::Atlas::procedural)));
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let size = pak::write_game(&exe, &pak, out)?;
        Ok(format!("Compiled {}\n\n{:.1} MB, with the scene, its texture atlas, {} image / sound file(s) and {} script(s).\nRun it directly: it needs nothing else. Esc closes it.", out.display(), size as f64 / 1.0e6, files, st.edits.scripts.len()))
    }

    /// Opens a project file, or a model as a new scene.
    fn open_any(&mut self, path: &str) {
        if project::is_project_file(path) {
            self.open_project_path(path.to_string());
        } else {
            self.window.set_title("Loading... - Polystation");
            spawn_load(self.tx.clone(), path.to_string(), None, Vec::new(), Intent::Replace { state: None, project: None });
        }
    }

    /// The engine settings dialog (the settings are saved for every project).
    fn open_settings(&mut self) {
        use ui::settings_dialog::ItemSpec::{Number, Section, Toggle};
        let (s, d) = (self.settings.clone(), settings::Settings::default());
        let specs = vec![
            Section("Display".into()),
            Toggle("VSync".into(), "Wait for the monitor refresh. Off lets the frame rate run free.".into(), s.vsync, d.vsync),
            Number("FPS limit".into(), "Frame rate cap. 0 = unlimited.".into(), s.fps_limit as f32, d.fps_limit as f32, 0.0, 1000.0, true),
            Number("Interface size (%)".into(), "0 = follow Windows. Applies after a restart.".into(), s.ui_scale as f32, d.ui_scale as f32, 0.0, 300.0, true),
            Section("Editor camera".into()),
            Number("Field of view".into(), "Degrees, for the editor camera (30 - 120).".into(), s.editor_fov, d.editor_fov, 30.0, 120.0, false),
            Number("Look sensitivity".into(), "Multiplier for mouse look and orbit (0.1 - 5).".into(), s.look_sensitivity, d.look_sensitivity, 0.1, 5.0, false),
            Section("AI assistants (MCP)".into()),
            Toggle("Allow assistants to connect".into(), "Opens a local port for polystation --mcp. Restart to apply.".into(), s.mcp_enabled, d.mcp_enabled),
            Number("MCP port".into(), "Local port (1024 - 65535). Restart to apply.".into(), s.mcp_port as f32, d.mcp_port as f32, 1024.0, 65535.0, true),
        ];
        let Some(v) = self.window.settings_dialog(specs) else { return };
        let new = settings::Settings {
            vsync: v[0] > 0.5,
            fps_limit: v[1] as u32,
            ui_scale: if v[2] < 1.0 { 0 } else { (v[2] as u32).clamp(50, 300) },
            editor_fov: v[3],
            look_sensitivity: v[4],
            mcp_enabled: v[5] > 0.5,
            mcp_port: v[6] as u16,
        };
        self.window.set_vsync(new.vsync);
        let restart = new.ui_scale != s.ui_scale || new.mcp_enabled != s.mcp_enabled || new.mcp_port != s.mcp_port;
        self.settings = new;
        match self.settings.save() {
            Ok(()) => self.status_note(if restart { "Settings saved. Some changes apply after a restart" } else { "Settings saved" }),
            Err(e) => self.window.message(&e),
        }
    }

    fn status_note(&mut self, text: &str) {
        self.window.set_status(text);
        self.status_t = Instant::now() + Duration::from_secs(2); // keep the note visible a moment
    }

    /// Returns true when the app should quit.
    fn command(&mut self, id: u16) -> bool {
        let nb = self.base.nodes.len();
        match id {
            win32::CMD_OPEN_PROJECT => self.open_project(),
            win32::CMD_SAVE => self.save_project(false),
            win32::CMD_SAVE_AS => self.save_project(true),
            win32::CMD_IMPORT => self.import_model(),
            win32::CMD_SCREENSHOT => self.shot = self.window.file_dialog(true, &BMP_FILTER, "bmp", "screenshot.bmp"),
            win32::CMD_EXIT => return true,
            win32::CMD_SETTINGS => self.open_settings(),
            win32::CMD_UNINSTALL => {
                let setup = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("polystation-setup.exe")));
                match setup.filter(|s| s.is_file()) {
                    Some(s) => {
                        if std::process::Command::new(s).arg("--uninstall").spawn().is_ok() {
                            return true; // close so the files can be removed
                        }
                    }
                    None => self.window.message("Polystation was not installed with its installer, so there is nothing to uninstall.\n\nTo remove an installed copy, use Windows Settings > Apps, or run polystation-setup.exe --uninstall."),
                }
            }
            win32::CMD_UPDATE => {
                // the installer that came with this copy fetches the newest release
                let setup = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("polystation-setup.exe")));
                match setup.filter(|s| s.is_file()) {
                    Some(s) => {
                        if std::process::Command::new(s).spawn().is_ok() {
                            self.window.message("The updater opens now. Polystation closes so it can be replaced: save your work first if you have not.");
                            return true;
                        }
                    }
                    None => self.window.message("Updates come from the Polystation installer (polystation-setup.exe), which was not found next to this program.

Download it from the GitHub releases page to install and update."),
                }
            }
            win32::CMD_NEW_PROJECT | win32::CMD_START_PAGE => {
                if self.play.is_some() {
                    self.status_note("Stop playing first");
                    return false;
                }
                let page = if id == win32::CMD_NEW_PROJECT { ui::hub::Page::New } else { ui::hub::Page::Projects };
                if let Some(path) = run_hub(&self.window, page, true) {
                    self.open_any(&path);
                }
            }
            win32::CMD_COMPILE => self.compile_command(),
            win32::CMD_GUI_EDITOR => {
                if self.play.is_some() {
                    self.status_note("Stop playing first, then edit the GUI");
                    return false;
                }
                if let Some((new, mode)) = self.window.gui_dialog(self.edits.gui.clone(), self.edits.gui_mode) {
                    if new != self.edits.gui || mode != self.edits.gui_mode {
                        self.push_undo();
                        self.edits.gui = new;
                        self.edits.gui_mode = mode;
                    }
                }
            }
            win32::CMD_EXPLORER_UP => {
                self.explorer_dir.pop();
                self.explorer_refresh();
            }
            win32::CMD_EXPLORER_REFRESH => self.explorer_refresh(),
            win32::CMD_EXPLORER_OPEN => self.explorer_activate(),
            win32::CMD_TAB_SCRIPT => {}
            win32::CMD_TAB_GUIDE => {}
            c if (win32::CMD_PREFAB_BASE..win32::CMD_PREFAB_BASE + prefabs::PREFABS.len() as u16).contains(&c) => self.add_prefab((c - win32::CMD_PREFAB_BASE) as usize),
            win32::CMD_HELP_GUIDE => self.window.show_bottom_tab(2),
            c if (win32::CMD_LIGHT_COLOR_BASE..win32::CMD_LIGHT_COLOR_BASE + 4).contains(&c) => {
                const COLORS: [[f32; 3]; 4] = [[1.0, 0.82, 0.55], [0.6, 0.75, 1.0], [1.0, 0.2, 0.15], [0.3, 1.0, 0.4]];
                let targets = self.selected_objects();
                if !targets.is_empty() {
                    self.push_undo();
                    for t in targets {
                        self.edit_color(t, Some(COLORS[(c - win32::CMD_LIGHT_COLOR_BASE) as usize]));
                    }
                }
            }
            c if (win32::CMD_COLL_BASE..win32::CMD_COLL_BASE + scene::Collision::NAMES.len() as u16).contains(&c) => self.set_collision(scene::Collision::from_index((c - win32::CMD_COLL_BASE) as usize)),
            c if (win32::CMD_EXAMPLE_BASE..win32::CMD_EXAMPLE_BASE + win32::CMD_EXAMPLE_COUNT).contains(&c) => {
                let Some(name) = self.script_target() else {
                    self.status_note("Select an object first, then pick an example script for it");
                    return false;
                };
                let (_, code) = guide::EXAMPLES[(c - win32::CMD_EXAMPLE_BASE) as usize];
                self.push_undo();
                let old = self.edits.scripts.get(&name).cloned().unwrap_or_default();
                // keep what is already there; the example goes below it
                let text = if old.trim().is_empty() { code.to_string() } else { format!("{}\n\n{code}", old.trim_end()) };
                self.edits.graphs.remove(&name); // examples are code
                self.edits.scripts.insert(name.clone(), text);
                self.open_script_tab(&name);
            }
            win32::CMD_TAB_PROJECT => {}
            win32::CMD_SCRIPT_ADD => {
                let Some(name) = self.script_target() else {
                    self.status_note("Select an object first, then add a script to it");
                    return false;
                };
                if !self.edits.scripts.contains_key(&name) {
                    self.push_undo();
                    self.edits.scripts.insert(name.clone(), SCRIPT_TEMPLATE.to_string());
                }
                self.open_script_tab(&name);
            }
            win32::CMD_SCRIPT_REMOVE => {
                if let Some(name) = self.script_target() {
                    if self.edits.scripts.contains_key(&name) {
                        self.push_undo();
                        self.edits.scripts.remove(&name);
                        self.edits.graphs.remove(&name);
                        self.window.close_script_tab(&name);
                        self.edit_key = None;
                        self.sync_script_badges();
                    }
                }
            }
            win32::CMD_ADD_MENU => {
                if let Some((l, _, _, b)) = self.window.ctl_screen_rect(win32::CMD_ADD_MENU) {
                    let items = self.add_menu();
                    self.show_menu(items, (l, b));
                }
            }
            win32::CMD_PLAY => {
                if self.game_mode {
                    return true; // Esc / F5 in a compiled game closes it
                }
                if self.play.is_some() {
                    self.stop_play();
                } else {
                    self.start_play();
                }
            }
            win32::CMD_RESET_CAMERA => {
                self.cam = Cam::framed(&self.base);
                self.speed_mul = 1.0;
            }
            win32::CMD_UNDO => self.undo(),
            win32::CMD_REDO => self.redo(),
            win32::CMD_DELETE => self.delete_selected(),
            win32::CMD_DUPLICATE => self.duplicate_selected(),
            win32::CMD_HIDE_SEL => self.hide_selected(),
            win32::CMD_SHOW_ALL => self.show_all(),
            win32::CMD_FRAME_SEL => self.frame_selected(),
            win32::CMD_FRAME_ALL => self.frame_all(),
            win32::CMD_AFFINE => self.view.affine = !self.view.affine,
            win32::CMD_GRID => self.view.grid = !self.view.grid,
            win32::CMD_CULL => self.view.cull = !self.view.cull,
            win32::CMD_DITHER_FINER | win32::CMD_DITHER_CHUNKIER => {
                self.push_undo();
                let e = &mut self.edits.env;
                e.dither = (e.dither * if id == win32::CMD_DITHER_FINER { 0.8 } else { 1.25 }).clamp(1.0, 12.0);
                self.details_dirty = true;
            }
            win32::CMD_FOG_NEARER | win32::CMD_FOG_FARTHER => {
                self.push_undo();
                let k = if id == win32::CMD_FOG_NEARER { 0.8 } else { 1.25 };
                let e = &mut self.edits.env;
                (e.fog_start, e.fog_end) = ((e.fog_start * k).max(0.01), (e.fog_end * k).max(0.02));
                self.details_dirty = true;
            }
            c if (win32::CMD_FOG_BASE..win32::CMD_FOG_BASE + win32::FOG_MODES).contains(&c) => {
                self.push_undo();
                self.edits.env.fog = (c - win32::CMD_FOG_BASE) as u8;
                self.details_dirty = true;
            }
            win32::CMD_PLAYER_SOUND => {
                if let Some(j) = self.player_index() {
                    if let Some(path) = self.window.file_dialog(false, &[("Sounds (*.wav, *.ogg, *.mp3, *.flac)", "*.wav;*.ogg;*.mp3;*.flac"), ("All files", "*.*")], "", "") {
                        self.push_undo();
                        self.edits.extras[j].player.step_sound = path;
                        self.details_dirty = true;
                    }
                }
            }
            win32::CMD_PLAYER_SOUND_CLEAR => {
                if let Some(j) = self.player_index() {
                    self.push_undo();
                    self.edits.extras[j].player.step_sound.clear();
                    self.details_dirty = true;
                }
            }
            win32::CMD_SCATTER_ADD => {
                let Some(tj) = self.terrain_index() else { self.status_note("Select a terrain first"); return true };
                self.push_undo();
                let (tname, sx) = (self.edits.extras[tj].name.clone(), self.edits.extras[tj].xform.scale[0].abs().max(0.1));
                let name = scene::unique_name("Scatter", |n| self.mesh.nodes.iter().any(|x| x.name == n) || self.edits.extras.iter().any(|e| e.name == n));
                self.edits.extras.push(Extra {
                    light: Default::default(), tree: Default::default(), player: Default::default(), scatter: scatter::ScatterParams { target: tname, min_size: sx * 0.03, max_size: sx * 0.07, ..Default::default() },
                    texture: Some(trees::PALETTE.to_string()),
                    particles: Default::default(),
                    terrain: Default::default(),
                    prim: prims::Prim::Scatter,
                    name: name.clone(),
                    xform: Xform::at([0.0; 3]),
                    color: None,
                    alpha: 1.0,
                });
                let hidden = self.hidden_by_name();
                self.rebuild(&hidden, Some(&name));
            }
            win32::CMD_TREE_RANDOMIZE => {
                if let Some(j) = self.tree_index() {
                    self.push_undo();
                    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64);
                    self.edits.extras[j].tree.seed = (t.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as u32 % 1_000_000;
                    self.regenerate();
                }
            }
            win32::CMD_SCATTER_RANDOMIZE => {
                if let Some(j) = self.scatter_index() {
                    self.push_undo();
                    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64);
                    self.edits.extras[j].scatter.seed = (t.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as u32 % 1_000_000;
                    self.regenerate();
                }
            }
            win32::CMD_TERRAIN_CHUNKED => {
                if let Some(j) = self.terrain_index() {
                    self.push_undo();
                    let on = self.window.is_checked(win32::CMD_TERRAIN_CHUNKED);
                    let t = &mut self.edits.extras[j].terrain;
                    t.chunked = on;
                    if on {
                        t.res = t.res.min(32); // chunks are small: one chunk is a whole terrain width
                    }
                    self.regenerate();
                }
            }
            win32::CMD_TERRAIN_ADD_LAYER | win32::CMD_TERRAIN_REMOVE_LAYER | win32::CMD_TERRAIN_RANDOMIZE => {
                if let Some(j) = self.terrain_index() {
                    self.push_undo();
                    let t = &mut self.edits.extras[j].terrain;
                    match id {
                        win32::CMD_TERRAIN_ADD_LAYER if t.layers.len() < terrain::MAX_LAYERS => {
                            let last = t.layers.last().cloned();
                            t.layers.push(terrain::Layer {
                                kind: terrain::NoiseKind::Perlin,
                                seed: last.as_ref().map_or(1, |l| l.seed + 1),
                                scale: last.as_ref().map_or(0.3, |l| (l.scale * 0.5).max(0.01)),
                                amp: last.as_ref().map_or(0.2, |l| l.amp * 0.5),
                            });
                        }
                        win32::CMD_TERRAIN_REMOVE_LAYER if t.layers.len() > 1 => {
                            t.layers.pop();
                        }
                        win32::CMD_TERRAIN_RANDOMIZE => {
                            let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(12345, |d| d.as_nanos() as u64) | 1;
                            for l in &mut t.layers {
                                x ^= x << 13;
                                x ^= x >> 7;
                                x ^= x << 17;
                                l.seed = (x % 100_000) as u32;
                            }
                        }
                        _ => {}
                    }
                    self.regenerate();
                }
            }
            win32::CMD_TEXTURE_SET => {
                if let Some(path) = self.window.file_dialog(false, &IMAGE_FILTER, "", "") {
                    self.apply_texture_file(&path);
                }
            }
            win32::CMD_TEXTURE_CLEAR => {
                let objs = self.selected_objects();
                self.set_texture(&objs, None);
            }
            win32::CMD_SELECT_ALL => {
                let all: Vec<usize> = (0..self.mesh.nodes.len()).filter(|&i| self.is_selectable_object(i)).collect();
                self.set_multi(all);
            }
            win32::CMD_SELECT_NONE => self.set_selected(None),
            win32::CMD_SELECT_ENV => {
                let env = self.base.nodes.len();
                if self.mesh.nodes.len() > env {
                    self.set_selected(Some(env));
                }
            }
            win32::CMD_ENV_FOG_COLOR | win32::CMD_ENV_SKY_COLOR | win32::CMD_ENV_SKY_BOTTOM => {
                let sky = &self.edits.env.sky;
                let init = match id {
                    win32::CMD_ENV_FOG_COLOR => self.edits.env.fog_color,
                    win32::CMD_ENV_SKY_BOTTOM => sky.bottom,
                    _ if sky.kind == SkyKind::Gradient => sky.top,
                    _ => sky.solid,
                };
                if let Some(c) = self.window.choose_color(init) {
                    self.push_undo();
                    let e = &mut self.edits.env;
                    match id {
                        win32::CMD_ENV_FOG_COLOR => e.fog_color = c,
                        win32::CMD_ENV_SKY_BOTTOM => e.sky.bottom = c,
                        _ if e.sky.kind == SkyKind::Gradient => e.sky.top = c,
                        _ => e.sky.solid = c,
                    }
                    self.details_dirty = true;
                }
            }
            win32::CMD_ENV_AMBIENT => {
                if let Some(c) = self.window.choose_color(self.edits.env.ambient) {
                    self.push_undo();
                    self.edits.env.ambient = c;
                    self.details_dirty = true;
                }
            }
            win32::CMD_ENV_FOG_FROM_SKY => {
                self.push_undo();
                let e = &mut self.edits.env;
                e.fog_color = if e.sky.kind == SkyKind::Solid { e.sky.solid } else { e.sky.bottom };
                self.details_dirty = true;
            }
            win32::CMD_ENV_SKY_TEXTURE => {
                if let Some(path) = self.window.file_dialog(false, &BMP_FILTER, "bmp", "") {
                    self.push_undo();
                    let e = &mut self.edits.env;
                    e.sky.texture = Some(path);
                    e.sky.kind = SkyKind::Texture;
                    self.details_dirty = true;
                }
            }
            win32::CMD_COLOR | win32::CMD_CLEAR_COLOR | win32::CMD_RESET_XFORM => {
                if let Some(s) = self.ex.selected.filter(|&s| self.mesh.nodes[s].kind == obj::Kind::Object) {
                    match id {
                        win32::CMD_COLOR => {
                            let init = self.rt.color[s].unwrap_or([0.8, 0.8, 0.8]);
                            if let Some(c) = self.window.choose_color(init) {
                                self.push_undo();
                                for t in self.selected_objects() {
                                    self.edit_color(t, Some(c));
                                }
                            }
                        }
                        win32::CMD_CLEAR_COLOR => {
                            self.push_undo();
                            for t in self.selected_objects() {
                                self.edit_color(t, None);
                            }
                        }
                        _ => {
                            self.push_undo();
                            for t in self.selected_objects() {
                                match node_ref(nb, t) {
                                    NodeRef::Base => {
                                        if let Some(e) = self.edits.base.get_mut(&self.base.nodes[t].name) {
                                            e.xform = None;
                                        }
                                    }
                                    NodeRef::Extra(j) => self.edits.extras[j].xform = Xform::at([0.0; 3]),
                                    NodeRef::PrimRoot | NodeRef::Env => {}
                                }
                            }
                            self.rt = compute_runtime(nb, &self.mesh, &self.edits);
                            self.objects_dirty = true;
                            self.details_dirty = true;
                        }
                    }
                }
            }
            c if (win32::CMD_GIZMO_BASE..win32::CMD_GIZMO_BASE + 4).contains(&c) => {
                self.gizmo = GIZMOS[(c - win32::CMD_GIZMO_BASE) as usize];
                self.hover = None;
            }
            c if (win32::CMD_ADD_BASE..win32::CMD_ADD_BASE + prims::ALL.len() as u16).contains(&c) => {
                self.add_primitive(prims::ALL[(c - win32::CMD_ADD_BASE) as usize]);
            }
            c if (win32::CMD_MODE_BASE..win32::CMD_MODE_BASE + win32::MODE_COUNT).contains(&c) => self.view.mode = c - win32::CMD_MODE_BASE,
            _ => {}
        }
        self.sync_menu();
        false
    }

    // ---------------------------------------------------------------- details panel
    /// The selected object's index into `edits.extras` if it is a player.
    fn player_index(&self) -> Option<usize> {
        let s = self.ex.selected?;
        match node_ref(self.base.nodes.len(), s) {
            NodeRef::Extra(j) if self.edits.extras[j].prim == prims::Prim::Player => Some(j),
            _ => None,
        }
    }

    fn refresh_player(&self, j: usize) {
        let (w, e) = (&self.window, &self.edits.extras[j]);
        let p = &e.player;
        let vals = [p.scale, p.speed, p.sprint_mult, p.gravity, p.jump_height, p.height, p.radius, p.eye_height, p.fov, p.bob_amount, p.bob_speed, p.step_volume, p.step_distance, p.mouse_sens, p.max_step, p.air_control];
        for (i, v) in vals.into_iter().enumerate() {
            w.set_text(win32::ED_PL0 + i as u16, &fmt_f(v));
        }
        let name = Path::new(&p.step_sound).file_name().map(|f| f.to_string_lossy().into_owned());
        w.set_text(win32::ST_PLSOUND, &format!("Step sound: {}", name.as_deref().unwrap_or("none")));
    }

    /// A player edit box changed.
    fn on_player_edit(&mut self, id: u16, text: &str) {
        let (Some(j), Some(s)) = (self.player_index(), self.ex.selected) else { return };
        let Ok(v) = text.trim().replace(',', ".").parse::<f32>() else { return };
        if !v.is_finite() {
            return;
        }
        if self.edit_key != Some((s, id)) {
            self.push_undo();
            self.edit_key = Some((s, id));
        }
        let p = &mut self.edits.extras[j].player;
        match id - win32::ED_PL0 {
            0 => p.scale = v,
            1 => p.speed = v,
            2 => p.sprint_mult = v,
            3 => p.gravity = v,
            4 => p.jump_height = v,
            5 => p.height = v,
            6 => p.radius = v,
            7 => p.eye_height = v,
            8 => p.fov = v,
            9 => p.bob_amount = v,
            10 => p.bob_speed = v,
            11 => p.step_volume = v,
            12 => p.step_distance = v,
            13 => p.mouse_sens = v,
            14 => p.max_step = v,
            _ => p.air_control = v,
        }
        p.sanitize();
        self.regenerate(); // the stand-in follows the player's size
    }

    /// The selected object's index into `edits.extras` if it is a scatter layer.
    /// Names an object or collection can be planted from (Source list of a scatter layer).
    fn scatter_sources(&self) -> Vec<String> {
        let nb = self.base.nodes.len();
        let mut v = vec!["(built-in shape)".to_string()];
        for (i, n) in self.mesh.nodes.iter().enumerate().skip(1) {
            let ok = match node_ref(nb, i) {
                NodeRef::Base => matches!(n.kind, obj::Kind::Object | obj::Kind::Collection),
                NodeRef::Extra(j) => !matches!(self.edits.extras[j].prim, prims::Prim::Scatter | prims::Prim::Terrain | prims::Prim::Particles | prims::Prim::Light | prims::Prim::Player),
                _ => false,
            };
            if ok {
                v.push(n.name.clone());
            }
        }
        v
    }

    fn scatter_index(&self) -> Option<usize> {
        let s = self.ex.selected?;
        match node_ref(self.base.nodes.len(), s) {
            NodeRef::Extra(j) if self.edits.extras[j].prim == prims::Prim::Scatter => Some(j),
            _ => None,
        }
    }

    fn refresh_scatter(&self, j: usize) {
        let (w, e) = (&self.window, &self.edits.extras[j]);
        let s = &e.scatter;
        w.combo_set(win32::CB_SSHAPE, &scatter::Shape::NAMES, s.shape.index());
        let sources = self.scatter_sources();
        let sel = if s.shape == scatter::Shape::Custom { sources.iter().position(|n| *n == s.source).unwrap_or(0) } else { 0 };
        let names: Vec<&str> = sources.iter().map(String::as_str).collect();
        w.combo_set(win32::CB_SSOURCE, &names, sel);
        w.set_text(win32::ED_SCOUNT, &s.count.to_string());
        w.set_text(win32::ED_SMIN, &fmt_f(s.min_size));
        w.set_text(win32::ED_SMAX, &fmt_f(s.max_size));
        w.set_text(win32::ED_SSEED, &s.seed.to_string());
        w.set_text(win32::ED_SSLOPE, &fmt_f(s.max_slope));
        w.set_text(win32::ED_SSINK, &fmt_f(s.sink));
        let tex = e.texture.as_deref().map(|t| Path::new(t).file_name().map_or(t.to_string(), |f| f.to_string_lossy().into_owned()));
        let c = e.color.unwrap_or([1.0, 1.0, 1.0]);
        let on = self.edits.extras.iter().any(|t| t.prim == prims::Prim::Terrain && t.name == s.target);
        w.set_text(
            win32::ST_STARGET,
            &format!(
                "On terrain: {}\r\nTexture: {}   Color: #{:02X}{:02X}{:02X}",
                if on { s.target.as_str() } else { "(missing - nothing is placed)" },
                tex.as_deref().unwrap_or("none"),
                (c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8
            ),
        );
    }

    /// A scatter edit box changed.
    fn on_scatter_edit(&mut self, id: u16, text: &str) {
        let (Some(j), Some(sel)) = (self.scatter_index(), self.ex.selected) else { return };
        let Ok(v) = text.trim().replace(',', ".").parse::<f32>() else { return };
        if !v.is_finite() {
            return;
        }
        if self.edit_key != Some((sel, id)) {
            self.push_undo();
            self.edit_key = Some((sel, id));
        }
        let s = &mut self.edits.extras[j].scatter;
        match id {
            win32::ED_SCOUNT => s.count = v.max(0.0) as u32,
            win32::ED_SMIN => s.min_size = v,
            win32::ED_SMAX => s.max_size = v,
            win32::ED_SSEED => s.seed = v.max(0.0) as u32,
            win32::ED_SSLOPE => s.max_slope = v,
            _ => s.sink = v,
        }
        s.sanitize();
        self.regenerate();
    }

    /// A source object was picked for a scatter layer: plant copies of it.
    fn on_scatter_source(&mut self) {
        let Some(j) = self.scatter_index() else { return };
        let sel = self.window.combo_selected(win32::CB_SSOURCE);
        let names = self.scatter_sources();
        self.push_undo();
        // a source made in the editor (a Tree object, a colored shape) passes its look on to the copies
        let look = names.get(sel).and_then(|n| self.edits.extras.iter().find(|x| &x.name == n)).map(|x| (x.texture.clone(), x.color));
        let e = &mut self.edits.extras[j];
        if sel == 0 || sel >= names.len() {
            if e.scatter.shape == scatter::Shape::Custom {
                e.scatter.shape = scatter::Shape::Tree;
            }
            e.scatter.source.clear();
        } else {
            e.scatter.shape = scatter::Shape::Custom;
            e.scatter.source = names[sel].clone();
            match look {
                Some((texture, color)) => (e.texture, e.color) = (texture, color),
                None => (e.texture, e.color) = (None, None), // a model from the file brings its own materials
            }
        }
        self.regenerate();
        self.edit_key = None;
    }

    fn on_scatter_combo(&mut self) {
        let Some(j) = self.scatter_index() else { return };
        let sel = self.window.combo_selected(win32::CB_SSHAPE);
        self.push_undo();
        let shape = scatter::Shape::from_index(sel);
        let e = &mut self.edits.extras[j];
        e.scatter.shape = shape;
        // trees use the tree palette; other shapes get a plain color instead
        if shape.tree_kind().is_some() && e.texture.is_none() {
            e.texture = Some(trees::PALETTE.to_string());
            e.color = None;
        } else if shape.tree_kind().is_none() && e.texture.as_deref() == Some(trees::PALETTE) {
            e.texture = None;
            e.color = Some([0.5, 0.5, 0.45]);
        }
        self.regenerate();
        self.edit_key = None;
    }

    /// The selected object's index into `edits.extras` if it is a particle system.
    fn particles_index(&self) -> Option<usize> {
        let s = self.ex.selected?;
        match node_ref(self.base.nodes.len(), s) {
            NodeRef::Extra(j) if self.edits.extras[j].prim == prims::Prim::Particles => Some(j),
            _ => None,
        }
    }

    fn refresh_particles(&self, j: usize) {
        let (w, e) = (&self.window, &self.edits.extras[j]);
        let p = &e.particles;
        for (i, v) in p.velocity.iter().enumerate() {
            w.set_text(win32::ED_PVEL0 + i as u16, &fmt_f(*v));
        }
        w.set_text(win32::ED_PGRAV, &fmt_f(p.gravity));
        w.set_text(win32::ED_PSIZE, &fmt_f(p.size));
        w.set_text(win32::ED_PAMOUNT, &p.amount.to_string());
        w.set_text(win32::ED_PBOX, &fmt_f(p.box_size));
        w.set_text(win32::ED_PLIFE, &fmt_f(p.lifetime));
        let tex = e.texture.as_deref().map(|t| Path::new(t).file_name().map_or(t.to_string(), |f| f.to_string_lossy().into_owned()));
        let c = e.color.unwrap_or([1.0, 1.0, 1.0]);
        w.set_text(
            win32::ST_PTEXTURE,
            &format!("Texture: {}   Color: #{:02X}{:02X}{:02X}", tex.as_deref().unwrap_or("none (squares)"), (c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8),
        );
    }

    /// A particle edit box changed.
    fn on_particle_edit(&mut self, id: u16, text: &str) {
        let (Some(j), Some(s)) = (self.particles_index(), self.ex.selected) else { return };
        let Ok(v) = text.trim().replace(',', ".").parse::<f32>() else { return };
        if !v.is_finite() {
            return;
        }
        if self.edit_key != Some((s, id)) {
            self.push_undo();
            self.edit_key = Some((s, id));
        }
        let p = &mut self.edits.extras[j].particles;
        match id {
            i if (win32::ED_PVEL0..win32::ED_PVEL0 + 3).contains(&i) => p.velocity[(i - win32::ED_PVEL0) as usize] = v,
            win32::ED_PGRAV => p.gravity = v,
            win32::ED_PSIZE => p.size = v,
            win32::ED_PAMOUNT => p.amount = v.max(0.0) as u32,
            win32::ED_PBOX => p.box_size = v,
            _ => p.lifetime = v,
        }
        p.sanitize();
    }

    /// The selected object's index into `edits.extras` if it is a terrain.
    fn terrain_index(&self) -> Option<usize> {
        let s = self.ex.selected?;
        match node_ref(self.base.nodes.len(), s) {
            NodeRef::Extra(j) if self.edits.extras[j].prim == prims::Prim::Terrain => Some(j),
            _ => None,
        }
    }

    /// Recomposes the mesh after the terrain parameters changed (the node list stays the same, so the
    /// explorer tree and the selection are left alone).
    fn regenerate(&mut self) {
        self.reload_mesh();
        self.details_dirty = true;
    }

    fn reload_mesh(&mut self) {
        self.terrain_views = self.compute_terrain_views();
        self.mesh = scene::compose(&self.base, &self.edits.extras, &self.terrain_views);
        hier::apply(&mut self.mesh.nodes, &self.base.nodes, &self.edits.hier);
        self.rt = compute_runtime(self.base.nodes.len(), &self.mesh, &self.edits);
        unsafe { upload_mesh(&self.g, self.vbo, self.ebo, self.wire_ebo, &self.mesh) };
        self.objects_dirty = true;
    }

    /// For each added object, the terrain chunk the camera is in (the terrain's own space, so the object's
    /// position, rotation and scale are taken into account).
    fn compute_terrain_views(&self) -> Vec<(i32, i32)> {
        self.edits
            .extras
            .iter()
            .map(|e| {
                if e.prim == prims::Prim::Terrain && e.terrain.chunked {
                    let x = &e.xform;
                    let inv = math::inverse_model_matrix(x.pos, x.rot, x.scale, [0.0; 3]);
                    let l = math::transform_point(&inv, self.cam.pos);
                    terrain::chunk_of(l[0], l[2])
                } else {
                    (0, 0)
                }
            })
            .collect()
    }

    fn refresh_terrain(&self, j: usize) {
        let (w, t) = (&self.window, &self.edits.extras[j].terrain);
        w.set_text(win32::ED_TERRAIN_RES, &t.res.to_string());
        w.set_text(win32::ST_TERRAIN_RES, if t.chunked { "cells per chunk" } else { "cells per side" });
        w.set_checked(win32::CMD_TERRAIN_CHUNKED, t.chunked);
        w.set_text(win32::ED_TERRAIN_DIST, &t.view_dist.to_string());
        w.set_enabled(win32::ED_TERRAIN_DIST, t.chunked);
        for r in 0..terrain::MAX_LAYERS {
            let active = r < t.layers.len();
            let (kind, seed, scale, amp) = match t.layers.get(r) {
                Some(l) => (l.kind.index(), l.seed.to_string(), fmt_f(l.scale), fmt_f(l.amp)),
                None => (0, String::new(), String::new(), String::new()),
            };
            let r = r as u16;
            w.combo_set(win32::CB_TTYPE0 + r, &terrain::NoiseKind::NAMES, kind);
            w.set_text(win32::ED_TSEED0 + r, &seed);
            w.set_text(win32::ED_TSCALE0 + r, &scale);
            w.set_text(win32::ED_TAMP0 + r, &amp);
            for id in [win32::CB_TTYPE0 + r, win32::ED_TSEED0 + r, win32::ED_TSCALE0 + r, win32::ED_TAMP0 + r] {
                w.set_enabled(id, active);
            }
        }
        w.set_enabled(win32::CMD_TERRAIN_ADD_LAYER, t.layers.len() < terrain::MAX_LAYERS);
        w.set_enabled(win32::CMD_TERRAIN_REMOVE_LAYER, t.layers.len() > 1);
    }

    /// A terrain edit box changed (resolution, or one layer's seed / scale / amount).
    fn on_terrain_edit(&mut self, id: u16, text: &str) {
        let (Some(j), Some(s)) = (self.terrain_index(), self.ex.selected) else { return };
        let Ok(v) = text.trim().replace(',', ".").parse::<f32>() else { return };
        if !v.is_finite() {
            return;
        }
        if self.edit_key != Some((s, id)) {
            self.push_undo();
            self.edit_key = Some((s, id));
        }
        let t = &mut self.edits.extras[j].terrain;
        if id == win32::ED_TERRAIN_RES {
            t.res = (v as u32).clamp(terrain::MIN_RES, terrain::MAX_RES);
        } else if id == win32::ED_TERRAIN_DIST {
            t.view_dist = (v as u32).clamp(1, terrain::MAX_VIEW_DIST);
        } else {
            let (group, layer) = ((id - win32::ED_TSEED0) / 10, ((id - win32::ED_TSEED0) % 10) as usize);
            let Some(l) = t.layers.get_mut(layer) else { return };
            match group {
                0 => l.seed = v.max(0.0) as u32,
                1 => l.scale = v.max(0.01),
                _ => l.amp = v,
            }
        }
        self.regenerate();
    }

    fn on_terrain_combo(&mut self, id: u16) {
        let Some(j) = self.terrain_index() else { return };
        let sel = self.window.combo_selected(id);
        self.push_undo();
        if let Some(l) = self.edits.extras[j].terrain.layers.get_mut((id - win32::CB_TTYPE0) as usize) {
            l.kind = terrain::NoiseKind::from_index(sel);
        }
        self.regenerate();
        self.edit_key = None;
    }

    /// Details panel contents while the Environment node is selected.
    fn refresh_env(&self) {
        let (w, e) = (&self.window, &self.edits.env);
        let hex = |c: [f32; 3]| format!("#{:02X}{:02X}{:02X}", (c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8);
        w.combo_set(win32::CB_FOG, &["Off", "Smooth", "Dithered"], e.fog as usize);
        w.set_text(win32::ED_FOG_START, &fmt_f(e.fog_start));
        w.set_text(win32::ED_FOG_END, &fmt_f(e.fog_end));
        w.set_text(win32::ED_DITHER, &fmt_f(e.dither));
        w.set_text(win32::ED_STEPS, &e.steps.to_string());
        w.set_text(win32::ED_RENDER_DIST, &fmt_f(e.render_distance));
        w.set_text(win32::ED_AMBIENT, &fmt_f(e.ambient_strength));
        for id in [win32::ED_FOG_START, win32::ED_FOG_END, win32::CMD_ENV_FOG_COLOR, win32::CMD_ENV_FOG_FROM_SKY] {
            w.set_enabled(id, e.fog != 0);
        }
        for id in [win32::ED_DITHER, win32::ED_STEPS] {
            w.set_enabled(id, e.fog == 2);
        }
        let kind = match e.sky.kind {
            SkyKind::Solid => 0,
            SkyKind::Gradient => 1,
            SkyKind::Texture => 2,
        };
        w.combo_set(win32::CB_SKY, &["Solid color", "Gradient", "Texture (panorama)"], kind);
        w.set_text(win32::CMD_ENV_SKY_COLOR, if e.sky.kind == SkyKind::Gradient { "Top color..." } else { "Sky color..." });
        w.set_enabled(win32::CMD_ENV_SKY_COLOR, e.sky.kind != SkyKind::Texture);
        w.set_enabled(win32::CMD_ENV_SKY_BOTTOM, e.sky.kind == SkyKind::Gradient);
        w.set_enabled(win32::CMD_ENV_SKY_TEXTURE, e.sky.kind == SkyKind::Texture);
        let sky = match e.sky.kind {
            SkyKind::Solid => format!("Sky: solid {}", hex(e.sky.solid)),
            SkyKind::Gradient => format!("Sky: gradient {} (top) to {} (bottom)", hex(e.sky.top), hex(e.sky.bottom)),
            SkyKind::Texture => format!(
                "Sky: panorama\r\n{}",
                e.sky.texture.as_deref().map(|p| Path::new(p).file_name().map_or(p.to_string(), |f| f.to_string_lossy().into_owned())).unwrap_or_else(|| "(no texture - click Texture...)".into())
            ),
        };
        w.set_text(
            win32::ST_ENV_INFO,
            &format!("Fog color: {}\r\n{sky}\r\nAmbient light: {} x{} (lights everything; scene lights add to it)\r\n\r\nRange is in multiples of the scene radius. Dithered fog\r\nquantises to the given steps using blocks of px.\r\nKeys: [ ] fog distance, , . dither size.", hex(e.fog_color), hex(e.ambient), fmt_f(e.ambient_strength)),
        );
    }

    fn refresh_details(&mut self) {
        self.refresh_script_editor();
        let w = &self.window;
        let nb = self.base.nodes.len();
        let objects = self.mesh.nodes.iter().enumerate().filter(|(i, n)| n.kind == obj::Kind::Object && !self.rt.deleted[*i]).count();
        let root = &self.mesh.nodes[0];
        let env_selected = self.ex.selected.is_some_and(|s| matches!(node_ref(nb, s), NodeRef::Env));
        let particles = if env_selected { None } else { self.particles_index() };
        let player = if env_selected { None } else { self.player_index() };
        let light = if env_selected { None } else { self.light_index() };
        let tree = if env_selected { None } else { self.tree_index() };
        w.set_group_visible(12, tree.is_some());
        w.set_group_visible(6, player.is_some());
        w.set_group_visible(11, light.is_some());
        let scatter = if env_selected { None } else { self.scatter_index() };
        w.set_group_visible(1, !env_selected && particles.is_none() && scatter.is_none() && player.is_none() && light.is_none() && tree.is_none());
        w.set_group_visible(2, env_selected);
        w.set_group_visible(4, particles.is_some());
        w.set_group_visible(5, scatter.is_some());
        if let Some(j) = particles {
            w.set_group_visible(3, false);
            self.refresh_particles(j);
            return;
        }
        if let Some(j) = scatter {
            w.set_group_visible(3, false);
            self.refresh_scatter(j);
            return;
        }
        if let Some(j) = player {
            w.set_group_visible(3, false);
            self.refresh_player(j);
            return;
        }
        if let Some(j) = light {
            w.set_group_visible(3, false);
            self.refresh_light(j);
            return;
        }
        if let Some(j) = tree {
            w.set_group_visible(3, false);
            self.refresh_tree(j);
            return;
        }
        let terrain = if env_selected { None } else { self.terrain_index() };
        w.set_group_visible(3, terrain.is_some());
        if let Some(j) = terrain {
            self.refresh_terrain(j);
        }
        if env_selected {
            self.refresh_env();
            return;
        }
        let Some(s) = self.ex.selected else {
            w.set_text(win32::ED_NAME, "");
            w.set_text(
                win32::ST_INFO,
                &format!("No selection\r\nScene: {objects} objects\r\n{} triangles, {} quads\r\nClick an object to inspect it.", root.tris, root.quads),
            );
            for id in win32::ED_XFORM..=win32::ED_LAST {
                w.set_text(id, "");
                w.set_enabled(id, false);
            }
            for id in [win32::ED_NAME, win32::CMD_COLOR, win32::CMD_CLEAR_COLOR, win32::CMD_RESET_XFORM, win32::CMD_FRAME_SEL, win32::CMD_DUPLICATE, win32::CMD_DELETE, win32::CMD_TEXTURE_SET, win32::CMD_TEXTURE_CLEAR] {
                w.set_enabled(id, false);
            }
            return;
        };
        let node = &self.mesh.nodes[s];
        let is_obj = node.kind == obj::Kind::Object;
        let is_extra = matches!(node_ref(nb, s), NodeRef::Extra(_));
        let kind = match node_ref(nb, s) {
            NodeRef::Extra(j) => format!("Primitive ({})", self.edits.extras[j].prim.name()),
            _ if is_obj => "Object (mesh)".to_string(),
            _ => "Collection".to_string(),
        };
        let kind = if self.ex.multi.len() > 1 { format!("{kind}  [+{} selected]", self.ex.multi.len() - 1) } else { kind };
        let parent = node.parent.map_or("-".to_string(), |p| self.mesh.nodes[p].name.clone());
        let (wc, wr) = self.world_bounds(s);
        let color = self.rt.color[s].map_or("texture".to_string(), |c| format!("#{:02X}{:02X}{:02X}", (c[0] * 255.0 + 0.5) as u8, (c[1] * 255.0 + 0.5) as u8, (c[2] * 255.0 + 0.5) as u8));
        let verts = if is_obj { (node.tris * 3 + node.quads * 4).to_string() } else { "-".into() };
        w.set_text(win32::ED_NAME, &node.name);
        w.set_text(
            win32::ST_INFO,
            &format!(
                "Type: {kind}\r\nParent: {parent}\r\nFaces: {} quads, {} tris\r\nVertices: {verts}\r\nBounds: center ({}, {}, {}), radius {}\r\nColor: {color}",
                node.quads, node.tris, fmt_f(wc[0]), fmt_f(wc[1]), fmt_f(wc[2]), fmt_f(wr)
            ),
        );
        let x = &self.rt.xf[s];
        let vals = [x.pos, x.rot, x.scale];
        for (row, v) in vals.iter().enumerate() {
            for axis in 0..3 {
                let id = win32::ED_XFORM + (row * 3 + axis) as u16;
                if is_obj {
                    w.set_text(id, &fmt_f(v[axis]));
                } else {
                    w.set_text(id, "");
                }
                w.set_enabled(id, is_obj);
            }
        }
        w.set_text(win32::ED_ALPHA, &if is_obj { fmt_f(self.rt.alpha[s]) } else { String::new() });
        w.set_enabled(win32::ED_ALPHA, is_obj);
        let tex = self.rt.texture[s].as_deref().map(|p| Path::new(p).file_name().map_or(p.to_string(), |f| f.to_string_lossy().into_owned()));
        w.set_text(win32::ST_TEXTURE, &format!("Texture: {}   (the color tints it)", tex.as_deref().unwrap_or("none")));
        w.set_enabled(win32::ED_NAME, is_extra);
        for id in [win32::CMD_COLOR, win32::CMD_CLEAR_COLOR, win32::CMD_RESET_XFORM, win32::CMD_TEXTURE_SET, win32::CMD_TEXTURE_CLEAR] {
            w.set_enabled(id, is_obj);
        }
        w.set_enabled(win32::CMD_FRAME_SEL, true);
        w.set_enabled(win32::CMD_DUPLICATE, is_extra);
        w.set_enabled(win32::CMD_DELETE, s != 0);
        w.combo_set(win32::CB_COLLISION, &scene::Collision::NAMES, self.edits.collision_of(&node.name).index());
        w.set_enabled(win32::CB_COLLISION, is_obj);
    }

    /// A details-panel edit box changed because the user typed in it.
    fn on_edit(&mut self, id: u16) {
        let nb = self.base.nodes.len();
        let Some(s) = self.ex.selected else { return };
        if id == win32::ED_SCRIPT {
            self.on_script_edit();
            return;
        }
        let text = self.window.get_text(id);
        if (win32::ED_TRUNK..=win32::ED_TRSEED).contains(&id) {
            self.on_tree_edit(id, &text);
            return;
        }
        if (win32::ED_LINT..=win32::ED_LANGLE).contains(&id) {
            self.on_light_edit(id, &text);
            return;
        }
        if (win32::ED_SCOUNT..=win32::ED_SSINK).contains(&id) {
            self.on_scatter_edit(id, &text);
            return;
        }
        if (win32::ED_PL0..=win32::ED_PL_LAST).contains(&id) {
            self.on_player_edit(id, &text);
            return;
        }
        if (win32::ED_PVEL0..=win32::ED_PLIFE).contains(&id) {
            self.on_particle_edit(id, &text);
            return;
        }
        if (win32::ED_TERRAIN_DIST..=win32::ED_TAMP0 + 4).contains(&id) {
            self.on_terrain_edit(id, &text);
            return;
        }
        if (win32::ED_FOG_START..=win32::ED_AMBIENT).contains(&id) {
            if !matches!(node_ref(nb, s), NodeRef::Env) {
                return;
            }
            let Ok(v) = text.trim().replace(',', ".").parse::<f32>() else { return };
            if !v.is_finite() {
                return;
            }
            if self.edit_key != Some((s, id)) {
                self.push_undo();
                self.edit_key = Some((s, id));
            }
            let e = &mut self.edits.env;
            match id {
                win32::ED_FOG_START => e.fog_start = v.max(0.0),
                win32::ED_FOG_END => e.fog_end = v.max(0.01),
                win32::ED_DITHER => e.dither = v.clamp(1.0, 12.0),
                win32::ED_RENDER_DIST => e.render_distance = v.max(0.0),
                win32::ED_AMBIENT => e.ambient_strength = v.clamp(0.0, 8.0),
                _ => e.steps = (v as u32).clamp(2, 32),
            }
            return;
        }
        if id == win32::ED_NAME {
            let NodeRef::Extra(j) = node_ref(nb, s) else { return };
            let name = text.trim().to_string();
            if name.is_empty() || name == self.edits.extras[j].name || self.mesh.nodes.iter().any(|n| n.name == name) {
                return;
            }
            if self.edit_key != Some((s, id)) {
                self.push_undo();
            }
            let old = std::mem::replace(&mut self.edits.extras[j].name, name.clone());
            if let Some(src) = self.edits.scripts.remove(&old) {
                self.edits.scripts.insert(name.clone(), src); // the script goes with the object
            }
            if let Some(g) = self.edits.graphs.remove(&old) {
                self.edits.graphs.insert(name.clone(), g);
            }
            self.window.close_script_tab(&old);
            for e in &mut self.edits.extras {
                if e.prim == prims::Prim::Scatter && e.scatter.target == old {
                    e.scatter.target = name.clone(); // scatter layers follow their terrain's new name
                }
            }
            let hidden = self.hidden_by_name();
            self.rebuild(&hidden, Some(&name));
            self.edit_key = Some((self.ex.selected.unwrap_or(s), id));
            self.details_dirty = false; // the name box already shows what was typed
            return;
        }
        if self.mesh.nodes[s].kind != obj::Kind::Object {
            return;
        }
        let Ok(v) = text.trim().replace(',', ".").parse::<f32>() else { return };
        if !v.is_finite() {
            return;
        }
        if self.edit_key != Some((s, id)) {
            self.push_undo();
            self.edit_key = Some((s, id));
        }
        if id == win32::ED_ALPHA {
            for t in self.selected_objects() {
                self.edit_alpha(t, v.clamp(0.0, 1.0));
            }
            return;
        }
        let k = (id - win32::ED_XFORM) as usize;
        self.edit_xform(s, |x| match k / 3 {
            0 => x.pos[k % 3] = v,
            1 => x.rot[k % 3] = v,
            _ => x.scale[k % 3] = v,
        });
    }

    /// A combo box in the environment panel changed.
    fn on_combo(&mut self, id: u16) {
        if id == win32::CB_TKIND {
            self.on_tree_combo();
            return;
        }
        if id == win32::CB_LKIND {
            self.on_light_combo();
            return;
        }
        if id == win32::CB_COLLISION {
            let sel = self.window.combo_selected(id);
            self.set_collision(scene::Collision::from_index(sel));
            return;
        }
        if (win32::CB_TTYPE0..win32::CB_TTYPE0 + 5).contains(&id) {
            self.on_terrain_combo(id);
            return;
        }
        if id == win32::CB_SSHAPE {
            self.on_scatter_combo();
            return;
        }
        if id == win32::CB_SSOURCE {
            self.on_scatter_source();
            return;
        }
        let sel = self.window.combo_selected(id);
        self.push_undo();
        let e = &mut self.edits.env;
        match id {
            win32::CB_FOG => e.fog = sel.min(2) as u8,
            _ => {
                e.sky.kind = match sel {
                    1 => SkyKind::Gradient,
                    2 => SkyKind::Texture,
                    _ => SkyKind::Solid,
                }
            }
        }
        self.details_dirty = true;
        self.edit_key = None;
    }

    /// Uploads the sky panorama when the environment's texture changes.
    fn sync_sky_texture(&mut self) {
        let want = if self.edits.env.sky.kind == SkyKind::Texture { self.edits.env.sky.texture.clone() } else { None };
        if want == self.sky_loaded {
            return;
        }
        self.sky_loaded = want.clone();
        let Some(path) = want else { return };
        match atlas::Atlas::load_bmp(&path) {
            Ok(a) => unsafe { upload_sky(&self.g, self.sky_tex, &a) },
            Err(e) => self.window.message(&format!("Could not load the sky texture:\n\n{e}")),
        }
    }

    // ---------------------------------------------------------------- per frame
    fn frame(&mut self, input: &win32::Input, dt: f32) {
        // while playing, the editor's mouse interactions are off (the mouse looks around instead)
        let playing = self.play.is_some();
        self.gui_clicks.clear();
        if playing {
            let want = self.want_cursor_free();
            if want != self.cursor_free {
                self.cursor_free = want;
                if want && self.look_active {
                    self.window.look_end();
                    self.look_active = false;
                } else if !want && !self.look_active {
                    self.window.look_begin();
                    self.look_active = true;
                }
            }
            self.gui_mouse = (input.mouse.0 as f32, input.mouse.1 as f32);
            self.gui_down = input.left_down && self.cursor_free;
            if let (true, Some(p)) = (self.cursor_free, input.left_pressed) {
                let (w, h) = self.window.size();
                if let Some(i) = gui::hit_button(&self.edits.gui, [0.0, 0.0, w as f32, h as f32], (p.0 as f32, p.1 as f32)) {
                    self.gui_clicks.push(self.edits.gui[i].name.clone());
                }
            }
        }
        let play_input;
        let input = if playing {
            play_input = win32::Input { left_pressed: None, left_released: None, left_down: false, right_down: false, wheel: 0.0, tree_dbl_click: false, explorer_dbl_click: false, tree_rclick: None, explorer_rclick: None, ..*input };
            &play_input
        } else {
            input
        };
        // explorer: checkboxes and selection, ~10x per second (and right away on a double-click)
        if input.tree_dbl_click || input.tree_rclick.is_some() || self.poll_t.elapsed() >= Duration::from_millis(100) {
            self.poll_t = Instant::now();
            let before = self.ex.selected;
            if poll_explorer(&self.window, &self.mesh, &mut self.ex) {
                self.objects_dirty = true;
            }
            if self.ex.selected != before {
                self.details_dirty = true;
                self.edit_key = None;
                self.objects_dirty = true;
                match self.ex.selected {
                    None => self.ex.multi.clear(),
                    Some(n) if win32::key_down(0x11) => {
                        if !self.ex.multi.contains(&n) {
                            self.ex.multi.push(n); // Ctrl+click adds to the selection
                        }
                    }
                    Some(n) if win32::key_down(0x10) => {
                        // Shift+click selects the objects between the previous item and this one
                        let anchor = before.or(self.ex.multi.first().copied()).unwrap_or(n);
                        let (lo, hi) = (anchor.min(n), anchor.max(n));
                        let mut list: Vec<usize> = (lo..=hi).filter(|&i| self.is_selectable_object(i)).collect();
                        list.retain(|&i| i != n);
                        list.push(n);
                        self.ex.multi = list;
                    }
                    Some(n) => self.ex.multi = vec![n],
                }
                self.sync_selection_marks();
            }
        }
        if input.tree_dbl_click {
            self.frame_selected();
        }
        if input.explorer_dbl_click {
            self.explorer_activate();
        }
        if let Some((n, t, at)) = self.window.take_tree_drop() {
            let find = |h: isize| self.ex.handles.iter().position(|&x| x == h);
            if let (Some(node), Some(target)) = (find(n), find(t)) {
                let place = match at {
                    win32::DropAt::Before => hier::Place::Before(target),
                    win32::DropAt::After => hier::Place::After(target),
                    win32::DropAt::Inside => hier::Place::Inside(target),
                    win32::DropAt::First => hier::Place::First(target),
                };
                if self.reparent(node, place) {
                    self.status_note("Moved in the hierarchy");
                } else {
                    self.status_note("That move is not possible");
                }
            }
        }
        if let Some(p) = input.tree_rclick {
            let items = if self.ex.selected.is_some() { self.object_menu() } else { self.empty_menu() };
            self.show_menu(items, p);
        }
        if let Some(p) = input.explorer_rclick {
            let items = self.explorer_menu();
            self.show_menu(items, p);
        }

        if playing {
            self.play_frame(dt);
        }

        // gizmo handle under the cursor
        if self.drag.is_none() && !input.right_down {
            self.hover = self.gizmo_hit(input.mouse.0, input.mouse.1);
        }

        // click to select (a press and release that barely moved); Alt + drag orbits instead
        if let Some(p) = input.left_pressed {
            let alt = win32::key_down(0x12);
            let on_compass = if let Some(dir) = self.compass_hit(p.0 as f32, p.1 as f32) {
                self.view_along(dir);
                true
            } else {
                false
            };
            if !alt && !on_compass {
                if let Some(h) = self.gizmo_hit(p.0, p.1) {
                    self.begin_drag(p, h);
                }
            }
            if self.drag.is_none() && !on_compass {
                self.press = Some((p, alt));
            }
            if alt && !on_compass {
                let pivot = self.ex.selected.map_or(self.base.center, |s| self.world_bounds(s).0);
                let d = math::dot(math::sub(self.cam.pos, pivot), math::sub(self.cam.pos, pivot)).sqrt().max(self.base.radius * 0.02);
                self.orbit = Some((pivot, d));
                self.last_mouse = Some(p);
            }
        }
        if self.drag.is_some() && input.left_down {
            self.update_drag(input.mouse.0, input.mouse.1);
        }
        // dragging on empty space draws a selection rectangle
        if input.left_down && self.drag.is_none() && self.orbit.is_none() {
            if let Some((p, alt)) = self.press {
                if !alt && (input.mouse.0 - p.0).abs().max((input.mouse.1 - p.1).abs()) >= 5 {
                    self.box_sel = Some((p, input.mouse));
                }
            }
        }
        if let Some(r) = input.left_released {
            if self.drag.is_some() {
                self.end_drag();
            }
            if let Some((p, alt)) = self.press.take() {
                let moved = (r.0 - p.0).abs().max((r.1 - p.1).abs());
                let additive = win32::key_down(0x11) || win32::key_down(0x10);
                if let Some((a, b)) = self.box_sel.take() {
                    self.finish_box(a, b, additive);
                } else if !alt && moved < 5 {
                    let hit = self.pick(r.0, r.1);
                    match (hit, additive) {
                        (Some(h), true) => self.toggle_selected(h),
                        (Some(h), false) => self.set_selected(Some(h)),
                        (None, true) => {}
                        (None, false) => self.set_selected(None),
                    }
                }
            }
            self.orbit = None;
            self.last_mouse = None;
            self.box_sel = None;
        }
        if let (Some((pivot, d)), true) = (self.orbit, input.left_down) {
            if let Some((lx, ly)) = self.last_mouse {
                self.cam.yaw += (input.mouse.0 - lx) as f32 * 0.008 * self.settings.look_sensitivity;
                self.cam.pitch = (self.cam.pitch - (input.mouse.1 - ly) as f32 * 0.008 * self.settings.look_sensitivity).clamp(-1.5, 1.5);
                let f = self.cam.forward();
                self.cam.pos = [pivot[0] - f[0] * d, pivot[1] - f[1] * d, pivot[2] - f[2] * d];
            }
            self.last_mouse = Some(input.mouse);
        }

        // fly camera: hold the right mouse button, mouse = look, WASD move, Q down / E up,
        // Shift = fast, scroll = speed. Without the button, scroll dollies the camera.
        let scene_r = self.scene_radius();
        self.window.set_drag_scale(scene_r);
        let base_speed = scene_r * 0.5;
        if input.right_down {
            if !self.look_active {
                self.window.look_begin();
                self.look_active = true;
                self.rmb = Some((input.mouse, Instant::now(), 0));
            }
            let (dx, dy) = self.window.look_delta();
            if let Some(r) = &mut self.rmb {
                r.2 += dx.abs() + dy.abs();
            }
            self.cam.yaw += dx as f32 * 0.0025 * self.settings.look_sensitivity;
            self.cam.pitch = (self.cam.pitch - dy as f32 * 0.0025 * self.settings.look_sensitivity).clamp(-1.55, 1.55);
            self.speed_mul = (self.speed_mul * 1.25f32.powf(input.wheel)).clamp(0.02, 100.0);
            let k = |vk: i32| if win32::key_down(vk) { 1.0f32 } else { 0.0 };
            let (fwd, side, up) = (k(0x57) - k(0x53), k(0x44) - k(0x41), k(0x45) - k(0x51));
            let boost = if win32::key_down(0x10) { 3.0 } else { 1.0 };
            let (f, r) = (self.cam.forward(), self.cam.right());
            let step = base_speed * self.speed_mul * boost * dt;
            for a in 0..3 {
                self.cam.pos[a] += (f[a] * fwd + r[a] * side) * step;
            }
            self.cam.pos[1] += up * step;
        } else if !playing {
            if self.look_active {
                self.window.look_end();
                self.look_active = false;
                if let Some((pos, t0, moved)) = self.rmb.take() {
                    if moved <= 6 && t0.elapsed() < Duration::from_millis(450) {
                        self.canvas_context_menu(pos);
                    }
                }
            }
            if input.wheel != 0.0 {
                let f = self.cam.forward();
                let step = input.wheel * scene_r * 0.1;
                for a in 0..3 {
                    self.cam.pos[a] += f[a] * step;
                }
            }
        }

        // chunked terrain: load the chunks around the camera when it crosses into a new one
        if self.edits.extras.iter().any(|e| e.terrain.chunked && e.prim == prims::Prim::Terrain) {
            let views = self.compute_terrain_views();
            if views != self.terrain_views {
                self.reload_mesh();
            }
        }

        // details panel (never overwrite a box the user is typing in)
        for id in self.window.take_edit_events() {
            self.on_edit(id);
        }
        for id in self.window.take_combo_events() {
            self.on_combo(id);
        }
        self.sync_sky_texture();
        self.sync_user_textures();
        if self.details_dirty && !self.window.panel_has_focus() {
            self.refresh_details();
            self.details_dirty = false;
        }

        // frame-rate and status bar
        self.fps = self.fps * 0.9 + (1.0 / dt.max(1e-4)) * 0.1;
        if Instant::now() >= self.status_t + Duration::from_millis(250) {
            self.status_t = Instant::now();
            let objects = self.mesh.nodes.iter().enumerate().filter(|(i, n)| n.kind == obj::Kind::Object && !self.rt.deleted[*i]).count();
            let root = &self.mesh.nodes[0];
            let sel = match (self.ex.multi.len(), self.selected_name()) {
                (n, _) if n > 1 => format!(" | Selected: {n} objects"),
                (_, Some(name)) => format!(" | Selected: {name}"),
                _ => String::new(),
            };
            if let Some(ps) = &self.play {
                self.window.set_status(&format!(
                    "{}Playing | speed {:.1} | {} | {:.0} fps | Esc or F5 to stop",
                    if self.hud.is_empty() { String::new() } else { format!("{}   |   ", self.hud) },
                    ps.walker.vel[0].hypot(ps.walker.vel[2]),
                    if ps.walker.on_ground { "on ground" } else { "in the air" },
                    self.fps
                ));
            } else {
            self.window.set_status(&format!(
                "{} | {} UVs | {} | gizmo: {} | fly speed x{:.2} | {} objects, {} tris, {} quads | {:.0} fps{sel}",
                MODE_NAMES[self.view.mode as usize],
                if self.view.affine { "affine" } else { "perspective" },
                if self.edits.env.fog == 0 { FOG_NAMES[0].to_string() } else { format!("{}, dither {}px", FOG_NAMES[self.edits.env.fog as usize], (self.edits.env.dither * win32::dpi_scale()).round() as i32) },
                GIZMO_NAMES[self.gizmo.index() as usize],
                self.speed_mul,
                objects, root.tris, root.quads, self.fps,
            ));
            }
        }
    }

    fn draw(&mut self) {
        let (w, h) = self.window.size();
        if w <= 0 || h <= 0 {
            return;
        }
        if self.objects_dirty {
            let water: HashSet<usize> = self.mesh.nodes.iter().enumerate().filter(|(_, n)| n.kind == obj::Kind::Object && self.edits.collision_of(&n.name) == scene::Collision::Water).map(|(i, _)| i).collect();
            let nb = self.base.nodes.len();
            let emissive: HashSet<usize> = self.edits.extras.iter().enumerate().filter(|(_, e)| e.prim == prims::Prim::Light).map(|(j, _)| nb + 2 + j).collect();
            let fx = ObjFx { hidden: self.script_hidden.union(&self.play_hidden).copied().collect(), water, emissive };
            unsafe { upload_objects(&self.g, self.obj_tex, &self.mesh, &self.ex, &self.rt, &self.utex_rects, &fx) };
            self.objects_dirty = false;
        }
        let scene_r = self.scene_radius();
        let (g, cam, mesh) = (&self.g, &self.cam, &self.mesh);
        let f = cam.forward();
        let target = [cam.pos[0] + f[0], cam.pos[1] + f[1], cam.pos[2] + f[2]];
        let view = math::look_at(cam.pos, target, [0.0, 1.0, 0.0]);
        let (near, mut far) = (scene_r * 0.005, scene_r * 300.0);
        if self.edits.env.render_distance > 0.0 {
            far = self.edits.env.render_distance.max(near * 10.0);
        }
        let fov = self.play.as_ref().map_or(self.settings.editor_fov, |p| p.params.fov);
        let proj = math::perspective(fov.to_radians(), w as f32 / h as f32, near, far);
        let mvp = math::mul(&proj, &view);
        self.last_mvp = mvp;
        let v = self.view;
        let wire_only = v.mode == 2;
        let overlay = v.mode == 3;
        let fill_mode = match v.mode {
            1 => 1,
            4 => 2,
            5 => 3,
            _ => 0,
        };
        let lights_packed = self.gather_lights();
        let env = &self.edits.env;
        let dither_px = (env.dither * win32::dpi_scale()).round().max(1.0) as i32;
        let (fog_start, fog_end) = (scene_r * env.fog_start, scene_r * env.fog_end);
        let (mut fog_mode, mut fog_color, fog_steps) = (env.fog as i32, env.fog_color, env.steps as f32);
        let (mut fog_start, mut fog_end) = (fog_start, fog_end);
        if self.underwater {
            // the camera is under water: a blue murk close around it
            (fog_mode, fog_color, fog_start, fog_end) = (1, [0.08, 0.28, 0.42], 0.3, 14.0);
        }
        let (sky_kind, sky_solid, sky_top, sky_bottom) = (env.sky.kind, env.sky.solid, env.sky.top, env.sky.bottom);
        let (sky_f, sky_r) = (cam.forward(), cam.right());
        let sky_u = math::cross(sky_r, sky_f);
        let (vx, vy, _, _) = self.window.viewport();
        unsafe {
            (g.glViewport)(vx, vy, w, h);
            (g.glClear)(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
            // sky first, behind everything
            (g.glDisable)(gl::DEPTH_TEST);
            (g.glUseProgram)(self.sky_prog);
            let sl = &self.sky_locs;
            (g.glUniform3f)(sl.cam_f, sky_f[0], sky_f[1], sky_f[2]);
            (g.glUniform3f)(sl.cam_r, sky_r[0], sky_r[1], sky_r[2]);
            (g.glUniform3f)(sl.cam_u, sky_u[0], sky_u[1], sky_u[2]);
            (g.glUniform1f)(sl.tan_half, (60f32.to_radians() * 0.5).tan());
            (g.glUniform1f)(sl.aspect, w as f32 / h as f32);
            (g.glUniform1i)(sl.mode, match sky_kind { SkyKind::Solid => 0, SkyKind::Gradient => 1, SkyKind::Texture => 2 });
            (g.glUniform3f)(sl.solid, sky_solid[0], sky_solid[1], sky_solid[2]);
            (g.glUniform3f)(sl.top, sky_top[0], sky_top[1], sky_top[2]);
            (g.glUniform3f)(sl.bottom, sky_bottom[0], sky_bottom[1], sky_bottom[2]);
            (g.glBindVertexArray)(self.sky_vao);
            (g.glDrawArrays)(gl::TRIANGLES, 0, 3);
            (g.glEnable)(gl::DEPTH_TEST);
            if v.grid && self.play.is_none() {
                (g.glUseProgram)(self.grid_prog);
                (g.glUniformMatrix4fv)(self.grid_locs.mvp, 1, 0, mvp.as_ptr());
                (g.glUniform3f)(self.grid_locs.cam, cam.pos[0], cam.pos[1], cam.pos[2]);
                (g.glUniform1i)(self.grid_locs.fog, fog_mode);
                (g.glUniform2f)(self.grid_locs.fog_range, fog_start, fog_end);
                (g.glUniform1i)(self.grid_locs.dither, dither_px);
                (g.glUniform3f)(self.grid_locs.fog_color, fog_color[0], fog_color[1], fog_color[2]);
                (g.glUniform1f)(self.grid_locs.fog_steps, fog_steps);
                (g.glBindVertexArray)(self.grid_vao);
                (g.glDrawArrays)(gl::LINES, 0, self.grid_count);
            }
            (g.glUseProgram)(self.prog);
            (g.glBindVertexArray)(self.vao);
            (g.glUniformMatrix4fv)(self.locs.mvp, 1, 0, mvp.as_ptr());
            (g.glUniform1f)(self.locs.time, self.t0.elapsed().as_secs_f32());
            {
                let (count, pos, col, dir) = lights_packed;
                (g.glUniform1i)(self.locs.n_lights, count);
                if count > 0 {
                    (g.glUniform4fv)(self.locs.l_pos, count, pos.as_ptr());
                    (g.glUniform4fv)(self.locs.l_col, count, col.as_ptr());
                    (g.glUniform4fv)(self.locs.l_dir, count, dir.as_ptr());
                }
                let (a, k) = (self.edits.env.ambient, self.edits.env.ambient_strength);
                (g.glUniform3f)(self.locs.ambient, a[0] * k, a[1] * k, a[2] * k);
            }
            (g.glUniform1i)(self.locs.affine, v.affine as i32);
            (g.glUniform3f)(self.locs.cam, cam.pos[0], cam.pos[1], cam.pos[2]);
            (g.glUniform1i)(self.locs.fog, fog_mode);
            (g.glUniform2f)(self.locs.fog_range, fog_start, fog_end);
            (g.glUniform1i)(self.locs.dither, dither_px);
            (g.glUniform3f)(self.locs.fog_color, fog_color[0], fog_color[1], fog_color[2]);
            (g.glUniform1f)(self.locs.fog_steps, fog_steps);
            if v.cull {
                (g.glEnable)(gl::CULL_FACE);
                (g.glCullFace)(gl::BACK);
            } else {
                (g.glDisable)(gl::CULL_FACE);
            }
            if !wire_only {
                (g.glUniform1i)(self.locs.mode, fill_mode);
                if overlay {
                    // push the filled faces back a little so the wire lines drawn on top win the depth test
                    (g.glEnable)(gl::POLYGON_OFFSET_FILL);
                    (g.glPolygonOffset)(1.0, 1.0);
                }
                (g.glBindBuffer)(gl::ELEMENT_ARRAY_BUFFER, self.ebo);
                // Pass 1: opaque objects, opaque + alpha-cutout materials (depth-writing, discard below 50% alpha).
                (g.glUniform1i)(self.locs.draw_pass, 0);
                (g.glUniform1f)(self.locs.cutoff, 0.5);
                (g.glDrawElements)(gl::TRIANGLES, mesh.opaque_count as i32, gl::UNSIGNED_INT, std::ptr::null());
                // Pass 2: translucent materials, and every material of objects whose alpha is below 1,
                // blended over the scene without writing depth (unsorted).
                let blended = mesh.indices.len() - mesh.opaque_count;
                let translucent_objects = self.rt.alpha.iter().any(|&a| a < 0.999);
                if blended > 0 || translucent_objects {
                    (g.glEnable)(gl::BLEND);
                    (g.glDepthMask)(0);
                    (g.glUniform1f)(self.locs.cutoff, 0.004);
                    if blended > 0 {
                        (g.glUniform1i)(self.locs.draw_pass, 1);
                        (g.glDrawElements)(gl::TRIANGLES, blended as i32, gl::UNSIGNED_INT, (mesh.opaque_count * 4) as *const c_void);
                    }
                    if translucent_objects {
                        (g.glUniform1i)(self.locs.draw_pass, 2);
                        (g.glDrawElements)(gl::TRIANGLES, mesh.opaque_count as i32, gl::UNSIGNED_INT, std::ptr::null());
                    }
                    (g.glDepthMask)(1);
                    (g.glDisable)(gl::BLEND);
                }
                (g.glDisable)(gl::POLYGON_OFFSET_FILL);
            }
            if v.grid && self.play.is_none() {
                let gi = &self.grid_inf;
                (g.glUseProgram)(self.grid_inf_prog);
                (g.glUniformMatrix4fv)(gi.mvp, 1, 0, mvp.as_ptr());
                (g.glUniform3f)(gi.cam_pos, cam.pos[0], cam.pos[1], cam.pos[2]);
                (g.glUniform3f)(gi.cam_f, sky_f[0], sky_f[1], sky_f[2]);
                (g.glUniform3f)(gi.cam_r, sky_r[0], sky_r[1], sky_r[2]);
                (g.glUniform3f)(gi.cam_u, sky_u[0], sky_u[1], sky_u[2]);
                (g.glUniform1f)(gi.tan_half, (60f32.to_radians() * 0.5).tan());
                (g.glUniform1f)(gi.aspect, w as f32 / h as f32);
                (g.glUniform1f)(gi.step, nice_step(scene_r / 16.0));
                (g.glUniform1f)(gi.fade, cam.pos[1].abs().max(scene_r) * 50.0);
                (g.glUniform1i)(gi.fog, fog_mode);
                (g.glUniform2f)(gi.fog_range, fog_start, fog_end);
                (g.glUniform3f)(gi.fog_color, fog_color[0], fog_color[1], fog_color[2]);
                (g.glUniform1f)(gi.fog_steps, fog_steps);
                (g.glUniform1i)(gi.dither, dither_px);
                (g.glBindVertexArray)(self.sky_vao);
                (g.glEnable)(gl::BLEND);
                (g.glDisable)(gl::CULL_FACE);
                (g.glDrawArrays)(gl::TRIANGLES, 0, 3);
                (g.glDisable)(gl::BLEND);
                // back to the model program for the wireframe pass
                (g.glUseProgram)(self.prog);
                (g.glBindVertexArray)(self.vao);
            }
            // particle systems: blended billboards, no depth writes
            let mut particles_drawn = false;
            let nb = self.base.nodes.len();
            for (j, e) in self.edits.extras.iter().enumerate() {
                let node = nb + 2 + j;
                if e.prim != prims::Prim::Particles || e.particles.amount == 0 || self.rt.deleted[node] || !self.ex.checked[node] {
                    continue;
                }
                let pl = &self.particle_locs;
                if !particles_drawn {
                    particles_drawn = true;
                    (g.glUseProgram)(self.particle_prog);
                    (g.glUniformMatrix4fv)(pl.mvp, 1, 0, mvp.as_ptr());
                    (g.glUniform1f)(pl.time, self.t0.elapsed().as_secs_f32());
                    (g.glUniform3f)(pl.cam_pos, cam.pos[0], cam.pos[1], cam.pos[2]);
                    (g.glUniform3f)(pl.cam_r, sky_r[0], sky_r[1], sky_r[2]);
                    (g.glUniform3f)(pl.cam_u, sky_u[0], sky_u[1], sky_u[2]);
                    (g.glUniform1i)(pl.fog, fog_mode);
                    (g.glUniform2f)(pl.fog_range, fog_start, fog_end);
                    (g.glUniform3f)(pl.fog_color, fog_color[0], fog_color[1], fog_color[2]);
                    (g.glUniform1f)(pl.fog_steps, fog_steps);
                    (g.glUniform1i)(pl.dither, dither_px);
                    (g.glBindVertexArray)(self.sky_vao); // an empty VAO: the vertices come from gl_VertexID
                    (g.glEnable)(gl::BLEND);
                    (g.glDepthMask)(0);
                    (g.glDisable)(gl::CULL_FACE);
                }
                let p = &e.particles;
                let tint = e.color.unwrap_or([1.0, 1.0, 1.0]);
                let rect = e.texture.as_ref().and_then(|t| self.utex_rects.get(t)).copied().unwrap_or([0.0; 4]);
                (g.glUniform1f)(pl.box_size, p.box_size);
                (g.glUniform3f)(pl.vel, p.velocity[0], p.velocity[1], p.velocity[2]);
                (g.glUniform1f)(pl.grav, p.gravity);
                (g.glUniform1f)(pl.size, p.size);
                (g.glUniform1f)(pl.life, p.lifetime);
                (g.glUniform4f)(pl.tint, tint[0], tint[1], tint[2], e.alpha);
                (g.glUniform4f)(pl.tex_rect, rect[0], rect[1], rect[2], rect[3]);
                (g.glDrawArrays)(gl::TRIANGLES, 0, 6 * p.amount.min(particles::MAX_PARTICLES) as i32);
            }
            if particles_drawn {
                (g.glDepthMask)(1);
                (g.glDisable)(gl::BLEND);
                (g.glUseProgram)(self.prog);
                (g.glBindVertexArray)(self.vao);
            }
            if wire_only || overlay {
                (g.glUniform1i)(self.locs.mode, 4);
                (g.glBindBuffer)(gl::ELEMENT_ARRAY_BUFFER, self.wire_ebo);
                (g.glDrawElements)(gl::LINES, mesh.wire.len() as i32, gl::UNSIGNED_INT, std::ptr::null());
                (g.glBindBuffer)(gl::ELEMENT_ARRAY_BUFFER, self.ebo);
            }
            let geo = self.gizmo_geometry();
            if !geo.is_empty() {
                (g.glUseProgram)(self.grid_prog);
                (g.glUniformMatrix4fv)(self.grid_locs.mvp, 1, 0, mvp.as_ptr());
                (g.glUniform1i)(self.grid_locs.fog, 0);
                (g.glBindVertexArray)(self.gizmo_vao);
                (g.glBindBuffer)(gl::ARRAY_BUFFER, self.gizmo_vbo);
                (g.glBufferData)(gl::ARRAY_BUFFER, (geo.len() * 4) as isize, geo.as_ptr() as *const c_void, gl::DYNAMIC_DRAW);
                (g.glDisable)(gl::DEPTH_TEST); // always on top of the scene
                (g.glDisable)(gl::CULL_FACE);
                (g.glDrawArrays)(gl::TRIANGLES, 0, (geo.len() / 6) as i32);
                (g.glEnable)(gl::DEPTH_TEST);
            }
            if let Some((a, b)) = self.box_sel {
                let (cw, ch) = (w as f32, h as f32);
                let ndc = |p: (i32, i32)| (p.0 as f32 / cw * 2.0 - 1.0, 1.0 - p.1 as f32 / ch * 2.0);
                let (p0, p1) = (ndc(a), ndc(b));
                let (c0, c1, c2, c3) = (p0, (p1.0, p0.1), p1, (p0.0, p1.1));
                let (mut v, col, hw) = (Vec::new(), [1.0, 0.85, 0.3], 1.5 / cw.min(ch));
                for (s, e) in [(c0, c1), (c1, c2), (c2, c3), (c3, c0)] {
                    push_line2(&mut v, s, e, hw, col);
                }
                (g.glUseProgram)(self.grid_prog);
                (g.glUniformMatrix4fv)(self.grid_locs.mvp, 1, 0, math::identity().as_ptr());
                (g.glUniform1i)(self.grid_locs.fog, 0);
                (g.glBindVertexArray)(self.gizmo_vao);
                (g.glBindBuffer)(gl::ARRAY_BUFFER, self.gizmo_vbo);
                (g.glBufferData)(gl::ARRAY_BUFFER, (v.len() * 4) as isize, v.as_ptr() as *const c_void, gl::DYNAMIC_DRAW);
                (g.glDisable)(gl::DEPTH_TEST);
                (g.glDisable)(gl::CULL_FACE);
                (g.glDrawArrays)(gl::TRIANGLES, 0, (v.len() / 6) as i32);
                (g.glEnable)(gl::DEPTH_TEST);
            }
            if !self.shot_waiters.is_empty() {
                let mut px = vec![0u8; (w * h * 4) as usize];
                (g.glReadPixels)(vx, vy, w, h, gl::RGBA, gl::UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
                let reply = mcp::screenshot_reply(w as usize, h as usize, &px);
                for tx in self.shot_waiters.drain(..) {
                    let _ = tx.send(reply.clone());
                }
            }
            if let Some(path) = self.shot.take() {
                let mut px = vec![0u8; (w * h * 4) as usize];
                (g.glReadPixels)(vx, vy, w, h, gl::RGBA, gl::UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
                match atlas::write_bmp(&path, w as usize, h as usize, &px) {
                    Ok(()) => println!("screenshot saved to {path}"),
                    Err(e) => self.window.message(&format!("Could not save the screenshot:\n\n{e}")),
                }
            }
        }
        self.window.draw_ui(&mut |c, vp| self.draw_overlay(c, vp));
        self.window.swap();
    }
}

fn main() {
    // usage: polystation [project.polystation | model.blend | model.obj] [atlas.bmp]
    //        polystation --mcp        (the MCP server an AI assistant launches; see the mcp module)
    if std::env::args().nth(1).as_deref() == Some("--mcp") {
        return mcp::run_stdio();
    }
    // a compiled game carries its data at the end of its own executable
    if let Some(game) = pak::embedded() {
        return run_game(game);
    }
    // Without a project the start page opens: recent projects, new projects from templates.
    let mut args = std::env::args().skip(1);
    let cfg = settings::Settings::load();
    win32::set_scale_override(cfg.ui_scale);
    let window = win32::Window::new(mcp::NAME, 1500, 860).unwrap_or_else(|e| die(&e));
    window.set_vsync(cfg.vsync);
    let first_arg = match args.next() {
        Some(a) => a,
        None => {
            window.init_ui(unsafe { gl::Gl::load() }.unwrap_or_else(|e| die(&e)));
            match run_hub(&window, ui::hub::Page::Projects, false) {
                Some(p) => p,
                None => return,
            }
        }
    };

    let (mut model_path, mut atlas_arg, mut project_path, mut startup) = (first_arg.clone(), args.next(), None, None);
    if project::is_project_file(&first_arg) {
        let st = project::load(Path::new(&first_arg)).unwrap_or_else(|e| die(&e));
        (model_path, atlas_arg) = (st.model.clone(), st.atlas.clone());
        project_path = Some(PathBuf::from(&first_arg));
        settings::add_recent(&first_arg);
        startup = Some(st);
    }

    let imports = startup.as_ref().map(|s: &ProjectState| s.imports.clone()).unwrap_or_default();
    let loaded = load_with_imports(&model_path, atlas_arg.as_deref(), &imports).unwrap_or_else(|e| die(&e));
    let (tx, rx) = mpsc::channel();
    let watcher = spawn_watch(tx.clone(), model_path.clone(), atlas_arg.clone(), imports);
    let (tex_tx, tex_rx) = mpsc::channel();
    let mut app = App::new(window, loaded, model_path, atlas_arg, project_path, startup, tx, watcher, tex_tx);
    app.window.set_text(win32::ED_GUIDE, &guide::GUIDE.replace('\n', "\r\n"));
    app.window.set_title(&app.title());

    let (mcp_tx, mcp_rx) = mpsc::channel();
    if cfg.mcp_enabled {
        mcp::serve(mcp_tx, cfg.mcp_port);
    }
    event_loop(&mut app, &rx, &tex_rx, &mcp_rx);
}

/// Shows the start page until the user picks or creates something. Returns the project (or model) to open.
fn run_hub(window: &win32::Window, page: ui::hub::Page, cancellable: bool) -> Option<String> {
    loop {
        let recents = settings::recents()
            .into_iter()
            .map(|p| {
                let name = Path::new(&p).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let when = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or(String::new(), |d| {
                    let s = d.as_secs();
                    match s {
                        0..=89 => "just now".into(),
                        90..=5399 => format!("{} min ago", s / 60),
                        5400..=129_599 => format!("{} h ago", s / 3600),
                        _ => format!("{} days ago", s / 86400),
                    }
                });
                ui::hub::RecentItem { name, path: p, when }
            })
            .collect();
        let choice = window.hub_dialog(page, recents, cancellable)?;
        match choice {
            ui::hub::HubChoice::Open(p) => return Some(p),
            ui::hub::HubChoice::BrowseProject => {
                if let Some(f) = window.file_dialog(false, &PROJECT_FILTER, project::EXT, "") {
                    return Some(f);
                }
            }
            ui::hub::HubChoice::BrowseModel => {
                if let Some(f) = window.file_dialog(false, &MODEL_FILTER, "", "") {
                    return Some(f);
                }
            }
            ui::hub::HubChoice::New { template, name, dir } => {
                let Some(t) = templates::by_id(&template) else { continue };
                match templates::create(t, &name, Path::new(&dir)) {
                    Ok(file) => return Some(file.to_string_lossy().into_owned()),
                    Err(e) => window.message(&e),
                }
            }
        }
        if window.quit_requested() {
            return None;
        }
    }
}

/// A compiled game: unpack the data, open a window with just the 3D view and play.
fn run_game(game: pak::Pak) {
    let text = game.get("project.polystation").map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_else(|| die("the game data has no project"));
    let mesh = pak::mesh_from_bytes(game.get("mesh.bin").unwrap_or_else(|| die("the game data has no scene"))).unwrap_or_else(|e| die(&e));
    let atlas = pak::atlas_from_bytes(game.get("atlas.bin").unwrap_or_else(|| die("the game data has no texture atlas"))).unwrap_or_else(|e| die(&e));
    // images and sounds are unpacked next to each other; everything else reads them like ordinary files
    let id = text.bytes().chain(game.entries.iter().flat_map(|(n, b)| n.bytes().chain(b.len().to_le_bytes())).take(4096)).fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    let dir = std::env::temp_dir().join(format!("polystation_game_{id:016x}"));
    for (name, bytes) in game.entries.iter().filter(|(n, _)| n.starts_with("pak/")) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, bytes);
    }
    let st = project::from_text(&text, &dir).unwrap_or_else(|e| die(&e));
    let cfg = settings::Settings::load();
    win32::set_scale_override(cfg.ui_scale);
    let b = st.build.clone();
    let title = if b.name.is_empty() { std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| "Game".into()) } else { b.name.clone() };
    let window = win32::Window::new(&title, b.width as i32, b.height as i32).unwrap_or_else(|e| die(&e));
    window.set_vsync(b.vsync);
    window.set_game_mode(true);
    window.set_title(&title);
    if b.fullscreen {
        window.set_fullscreen(true);
    }
    let (tx, rx) = mpsc::channel();
    let (tex_tx, tex_rx) = mpsc::channel();
    let (_mcp_tx, mcp_rx) = mpsc::channel::<mcp::Request>();
    let stopped = Arc::new(AtomicBool::new(true)); // there is no model file to watch
    let mut app = App::new(window, Loaded { mesh, atlas: Some(atlas) }, "game".into(), None, Some(dir.join("game.polystation")), Some(st), tx, stopped, tex_tx);
    app.game_mode = true;
    app.settings.fps_limit = b.fps_limit; // a game has its own frame rate settings, not the editor's
    app.settings.vsync = b.vsync;
    app.start_play();
    if app.play.is_none() {
        die("the game could not start: it needs a Player object");
    }
    event_loop(&mut app, &rx, &tex_rx, &mcp_rx);
}

fn event_loop(app: &mut App, rx: &mpsc::Receiver<Msg>, tex_rx: &mpsc::Receiver<TexMsg>, mcp_rx: &mpsc::Receiver<mcp::Request>) {
    let mut last = Instant::now();
    loop {
        let input = app.window.poll();
        if input.quit {
            break;
        }
        let mut quit = false;
        for id in app.window.take_commands() {
            quit |= app.command(id);
        }
        if quit {
            break;
        }
        while let Ok(msg) = rx.try_recv() {
            app.apply(msg);
        }
        while let Ok(msg) = tex_rx.try_recv() {
            app.apply_tex_msg(msg);
        }
        while let Ok(rq) = mcp_rx.try_recv() {
            app.mcp_call(rq);
        }
        app.handle_script_events();
        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f32().min(0.1);
        last = now;
        app.frame(&input, dt);
        app.draw();
        // frame rate cap: sleep most of the wait, spin the rest
        if app.settings.fps_limit > 0 {
            let target = Duration::from_secs_f32(1.0 / app.settings.fps_limit as f32);
            loop {
                let left = target.saturating_sub(now.elapsed());
                if left.is_zero() {
                    break;
                }
                if left > Duration::from_millis(2) {
                    std::thread::sleep(Duration::from_millis(1));
                } else {
                    std::hint::spin_loop();
                }
            }
        }
    }
}

#[cfg(test)]
mod gizmo_tests {
    use super::*;

    #[test]
    fn axis_param_finds_the_crossing_point() {
        // a ray from (3,0,5) looking down -z crosses the x axis at x = 3
        let t = axis_param([0.0; 3], AXES[0], ([3.0, 0.0, 5.0], [0.0, 0.0, -1.0]));
        assert!((t - 3.0).abs() < 1e-5, "{t}");
        // and with the axis origin shifted, t is measured from the axis origin
        let t = axis_param([1.0, 0.0, 0.0], AXES[0], ([3.0, 0.0, 5.0], [0.0, 0.0, -1.0]));
        assert!((t - 2.0).abs() < 1e-5, "{t}");
    }

    #[test]
    fn plane_angle_is_right_handed() {
        // looking straight down on the XZ plane (ring around Y): u = z, v = x
        let a = plane_angle([0.0; 3], 1, ([1.0, 5.0, 0.0], [0.0, -1.0, 0.0])).unwrap();
        assert!((a - std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{a}");
        let b = plane_angle([0.0; 3], 1, ([0.0, 5.0, 1.0], [0.0, -1.0, 0.0])).unwrap();
        assert!(b.abs() < 1e-5, "{b}");
        // a ray parallel to the plane never hits it
        assert!(plane_angle([0.0; 3], 1, ([0.0, 5.0, 0.0], [1.0, 0.0, 0.0])).is_none());
    }

    #[test]
    fn rotating_about_a_world_axis_composes_after_the_current_rotation() {
        // 90 degrees about world Y from no rotation is just rot.y = 90
        let r = math::euler_from_matrix(&math::mul(&math::rot_y(90f32.to_radians()), &math::rot_matrix([0.0; 3])));
        assert!(r[0].abs() < 1e-3 && (r[1] - 90.0).abs() < 1e-3 && r[2].abs() < 1e-3, "{r:?}");
        // from 90 about X, a world-Y turn is NOT rot.y += angle, but the result still equals the composition
        let start = [90.0, 0.0, 0.0];
        let turned = math::mul(&math::rot_y(30f32.to_radians()), &math::rot_matrix(start));
        let back = math::rot_matrix(math::euler_from_matrix(&turned));
        for k in 0..16 {
            assert!((turned[k] - back[k]).abs() < 1e-4);
        }
    }

    #[test]
    fn script_sounds_are_found_and_rewritten() {
        let src = "on key(\"e\") {
  sound(\"C:/x/jump.wav\")
  sound( \"coin.wav\" )
  print(\"sound\")
}
sound(1)";
        let out = rewrite_sounds(src, &mut |p| (p != "coin.wav").then(|| format!("pak/{p}")));
        assert_eq!(out, "on key(\"e\") {
  sound(\"pak/C:/x/jump.wav\")
  sound( \"coin.wav\" )
  print(\"sound\")
}
sound(1)");
    }

    #[test]
    fn children_follow_a_moved_or_turned_parent() {
        let parent = Xform::at([0.0, 0.0, 0.0]);
        let child = Xform::at([1.0, 0.0, 0.0]);
        let moved = follow(&parent, &Xform::at([5.0, 0.0, 0.0]), &child);
        assert_eq!(moved.pos, [6.0, 0.0, 0.0]);
        let turned = follow(&parent, &Xform { pos: [0.0; 3], rot: [0.0, 90.0, 0.0], scale: [1.0; 3] }, &child);
        assert!((turned.pos[0].hypot(turned.pos[2]) - 1.0).abs() < 1e-4, "stays 1 unit from the parent");
        assert!(turned.pos[0].abs() < 1e-4, "swung a quarter turn");
        assert!((turned.rot[1].abs() - 90.0).abs() < 1e-3);
        let bigger = follow(&parent, &Xform { pos: [0.0; 3], rot: [0.0; 3], scale: [2.0; 3] }, &child);
        assert_eq!(bigger.pos, [2.0, 0.0, 0.0]);
        assert_eq!(bigger.scale, [2.0; 3]);
    }

    #[test]
    fn point_segment_distance() {
        assert!((dist_point_segment((0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)) - 1.0).abs() < 1e-6);
        assert!((dist_point_segment((3.0, 0.0), (-1.0, 0.0), (1.0, 0.0)) - 2.0).abs() < 1e-6); // beyond the end
    }
}
