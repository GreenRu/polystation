//! The `.polystation` project file (older `.gpr` files open too): a small text format that remembers which model is open, how the scene
//! is being viewed and every edit made on top of the model. It references the model by path instead
//! of embedding it, so the source `.blend` stays the single source of truth (and hot reload keeps
//! working).
//!
//! ```text
//! GPURENDER-PROJECT 12
//! build<TAB><game title><TAB><width height fullscreen vsync fps_limit show_fps>   (settings of a compiled game)
//! import models/tree.blend         (a model imported into the scene; one line per import, in order)
//! model models/scene.blend          (relative to the project file when possible)
//! atlas textures/atlas.bmp          (optional)
//! camera2 <yaw> <pitch> <x> <y> <z> <speed multiplier>
//! view <mode 0-5> <affine 0|1> <grid 0|1> <cull 0|1>
//! env <fog 0-2> <fog r g b> <fog start> <fog end> <dither px> <steps> [<render distance>]
//! sky <kind 0 solid|1 gradient|2 texture> <solid r g b> <top r g b> <bottom r g b> <texture path>
//! hidden <node name>                (one line per unchecked node)
//! selected <node name>
//! edit3 <pos x3> <rot x3> <scale x3> <has xform 0|1> <deleted 0|1> <has color 0|1> <r g b> <has alpha 0|1> <alpha> <node name>
//! prim3 <kind> <pos x3> <rot x3> <scale x3> <has color 0|1> <r g b> <alpha> <name>
//! terrain2 <pos x3> <rot x3> <scale x3> <has color 0|1> <r g b> <alpha> <resolution> <chunked 0|1> <chunk render distance> <layers n> <kind seed scale amp>*n <name>
//! (`terrain`, without chunk settings, is still read)
//! particles <has color 0|1> <r g b> <alpha> <velocity x3> <gravity> <size> <amount> <box size> <lifetime> <name>
//! scatter<TAB><name><TAB><terrain name><TAB><has color 0|1> <r g b> <alpha> <shape> <count> <seed> <min size> <max size> <max slope> <sink>
//! player<TAB><name><TAB><speed sprint gravity jump height radius eye fov bob_amount bob_speed volume step_distance mouse_sens max_step air_control [scale]><TAB><step sound path>
//! ambient <r g b> [strength]      (light that reaches everything, with or without lights in the scene)
//! tree<TAB><name><TAB><0 pine|1 oak|2 birch|3 palm|4 dead> <trunk> <trunk width> <crown> <detail> <seed>
//! light<TAB><name><TAB><0 point|1 spot|2 sun> <intensity> <range> <spot angle>
//! collision<TAB><object name><TAB><0 solid | 1 passable | 2 trigger>   (solid is the default and not written)
//! graph<TAB><object name><TAB><the visual script as one line of JSON>   (see the graph module)
//! guimode <0 look | 1 cursor | 2 tab toggle | 3 hold ctrl>   (how the mouse works while the game plays)
//! gui<TAB><name><TAB><kind anchor x y w h r g b alpha text_r text_g text_b size value visible><TAB><text, escaped>   (a game GUI element, back to front)
//! hier<TAB><node name><TAB><parent name, empty = scene>   (the explorer arrangement; one line per node, in display order)
//! script<TAB><object name><TAB><source, with \\ \n \t escapes>
//! texture<TAB><image path><TAB><node name>     (a tab-separated line, one per textured object)
//! (`edit` / `prim` without alpha, from version 2 files, are still read)
//! ```
//! Version 1 files (orbit camera, no edits) still open.
use crate::prims::Prim;
use crate::particles::ParticleParams;
use crate::light::{LightKind, LightParams};
use crate::player::PlayerParams;
use crate::scatter::{ScatterParams, Shape};
use crate::scene::{self, Edits, Extra, NodeEdit, Xform};
use crate::terrain::{Layer, NoiseKind, TerrainParams};
use crate::trees::{TreeKind, TreeParams};
use std::path::{Path, PathBuf};

const MAGIC: &str = "POLYSTATION-PROJECT";
/// Files written before the engine was renamed start with this.
const LEGACY_MAGIC: &str = "GPURENDER-PROJECT";
const VERSION: u32 = 19;

/// The extension of project files.
pub const EXT: &str = "polystation";

/// Project files: `.polystation`, and the old `.gpr` that still opens.
pub fn is_project_file(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with(&format!(".{EXT}")) || n.ends_with(".gpr")
}

#[derive(Clone, Debug, PartialEq)]
pub struct CameraState {
    pub yaw: f32,
    pub pitch: f32,
    pub pos: [f32; 3],
    pub speed: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewState {
    pub mode: u8,
    pub affine: bool,
    pub grid: bool,
    pub cull: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectState {
    /// Absolute (or cwd-relative) path of the model once loaded; stored relative on disk.
    pub model: String,
    pub atlas: Option<String>,
    /// Models imported into the scene on top of `model`, in order.
    pub imports: Vec<String>,
    /// How a compiled game behaves.
    pub build: crate::build::BuildSettings,
    pub camera: Option<CameraState>,
    pub view: ViewState,
    /// Names of nodes whose checkbox is off.
    pub hidden: Vec<String>,
    pub selected: Option<String>,
    pub edits: Edits,
}

fn project_dir(project: &Path) -> PathBuf {
    let abs = std::path::absolute(project).unwrap_or_else(|_| project.to_path_buf());
    abs.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Path as stored in the file: relative to the project directory if the file lives below it.
fn to_stored(path: &str, dir: &Path) -> String {
    if path.starts_with("builtin:") {
        return path.to_string(); // built-in textures are not files
    }
    let abs = std::path::absolute(path).unwrap_or_else(|_| PathBuf::from(path));
    let shown = abs.strip_prefix(dir).map(Path::to_path_buf).unwrap_or(abs);
    shown.to_string_lossy().replace('\\', "/")
}

fn from_stored(path: &str, dir: &Path) -> String {
    if path.starts_with("builtin:") {
        return path.to_string();
    }
    let p = Path::new(path);
    if p.is_absolute() {
        path.to_string()
    } else {
        // collecting the components normalises the separators of the joined path
        dir.join(p).components().collect::<PathBuf>().to_string_lossy().into_owned()
    }
}

fn nums(x: &Xform) -> String {
    let [a, b, c] = [x.pos, x.rot, x.scale];
    format!("{} {} {} {} {} {} {} {} {}", a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2])
}

/// The project as text; paths below `dir` are written relative to it.
pub fn to_text(dir: &Path, st: &ProjectState) -> String {
    let mut out = format!("{MAGIC} {VERSION}\nmodel {}\n", to_stored(&st.model, &dir));
    if let Some(a) = &st.atlas {
        out += &format!("atlas {}\n", to_stored(a, &dir));
    }
    for i in &st.imports {
        out += &format!("import {}\n", to_stored(i, &dir));
    }
    out += &format!("build\t{}\t{}\n", st.build.name.replace(['\t', '\n', '\r'], " "), st.build.numbers_text());
    if let Some(c) = &st.camera {
        out += &format!("camera2 {} {} {} {} {} {}\n", c.yaw, c.pitch, c.pos[0], c.pos[1], c.pos[2], c.speed);
    }
    let v = &st.view;
    out += &format!("view {} {} {} {}\n", v.mode, v.affine as u8, v.grid as u8, v.cull as u8);
    let e = &st.edits.env;
    out += &format!("env {} {} {} {} {} {} {} {} {}\n", e.fog, e.fog_color[0], e.fog_color[1], e.fog_color[2], e.fog_start, e.fog_end, e.dither, e.steps, e.render_distance);
    let (s, kind) = (&e.sky, e.sky.kind as u8);
    out += &format!(
        "sky {kind} {} {} {} {} {} {} {} {} {} {}\n",
        s.solid[0], s.solid[1], s.solid[2], s.top[0], s.top[1], s.top[2], s.bottom[0], s.bottom[1], s.bottom[2],
        s.texture.as_deref().map(|t| to_stored(t, &dir)).unwrap_or_default()
    );
    for h in &st.hidden {
        out += &format!("hidden {h}\n");
    }
    if let Some(s) = &st.selected {
        out += &format!("selected {s}\n");
    }
    let mut names: Vec<&String> = st.edits.base.keys().collect();
    names.sort();
    for name in names {
        let e = &st.edits.base[name];
        let x = e.xform.unwrap_or(Xform::at([0.0; 3]));
        let has_color = e.color.is_some() as u8;
        let c = e.color.unwrap_or([0.0; 3]);
        out += &format!(
            "edit3 {} {} {} {has_color} {} {} {} {} {} {name}\n",
            nums(&x), e.xform.is_some() as u8, e.deleted as u8, c[0], c[1], c[2], e.alpha.is_some() as u8, e.alpha.unwrap_or(1.0)
        );
    }
    for e in &st.edits.extras {
        let c = e.color.unwrap_or([0.0; 3]);
        if e.prim == Prim::Scatter {
            let s = &e.scatter;
            out += &format!(
                "scatter\t{}\t{}\t{} {} {} {} {} {} {} {} {} {} {} {}\t{}\n",
                e.name, s.target, e.color.is_some() as u8, c[0], c[1], c[2], e.alpha, s.shape.index(), s.count, s.seed, s.min_size, s.max_size, s.max_slope, s.sink, s.source
            );
            continue;
        }
        if e.prim == Prim::Particles {
            let p = &e.particles;
            out += &format!(
                "particles {} {} {} {} {} {} {} {} {} {} {} {} {} {}\n",
                e.color.is_some() as u8, c[0], c[1], c[2], e.alpha, p.velocity[0], p.velocity[1], p.velocity[2], p.gravity, p.size, p.amount, p.box_size, p.lifetime, e.name
            );
            continue;
        }
        if e.prim == Prim::Terrain {
            let t = &e.terrain;
            let mut line = format!("terrain2 {} {} {} {} {} {} {} {} {} {}", nums(&e.xform), e.color.is_some() as u8, c[0], c[1], c[2], e.alpha, t.res, t.chunked as u8, t.view_dist, t.layers.len());
            for l in &e.terrain.layers {
                line += &format!(" {} {} {} {}", l.kind.index(), l.seed, l.scale, l.amp);
            }
            out += &format!("{line} {}\n", e.name);
            continue;
        }
        out += &format!("prim3 {} {} {} {} {} {} {} {}\n", e.prim.name(), nums(&e.xform), e.color.is_some() as u8, c[0], c[1], c[2], e.alpha, e.name);
    }
    for e in st.edits.extras.iter().filter(|e| e.prim == Prim::Player) {
        let p = &e.player;
        out += &format!(
            "player\t{}\t{} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}\t{}\n",
            e.name, p.speed, p.sprint_mult, p.gravity, p.jump_height, p.height, p.radius, p.eye_height, p.fov, p.bob_amount, p.bob_speed, p.step_volume, p.step_distance, p.mouse_sens, p.max_step, p.air_control, p.scale,
            if p.step_sound.is_empty() { String::new() } else { to_stored(&p.step_sound, &dir) }
        );
    }
    out += &format!("ambient {} {} {} {}\n", st.edits.env.ambient[0], st.edits.env.ambient[1], st.edits.env.ambient[2], st.edits.env.ambient_strength);
    for e in st.edits.extras.iter().filter(|e| e.prim == Prim::Light) {
        let l = &e.light;
        out += &format!("light\t{}\t{} {} {} {}\n", e.name, l.kind.index(), l.intensity, l.range, l.angle);
    }
    for e in st.edits.extras.iter().filter(|e| e.prim == Prim::Tree) {
        let t = &e.tree;
        out += &format!("tree\t{}\t{} {} {} {} {} {}\n", e.name, t.kind.index(), t.trunk, t.trunk_width, t.crown, t.detail, t.seed);
    }
    let mut collided: Vec<(&String, &crate::scene::Collision)> = st.edits.collision.iter().collect();
    collided.sort_by(|a, b| a.0.cmp(b.0));
    for (name, c) in collided {
        out += &format!("collision\t{}\t{}\n", name, c.index());
    }
    out += &format!("guimode {}\n", st.edits.gui_mode.index());
    for e in &st.edits.gui {
        out += &format!(
            "gui\t{}\t{} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}\t{}\n",
            e.name.replace(['\t', '\n', '\r'], " "), e.kind.index(), e.anchor, e.x, e.y, e.w, e.h, e.color[0], e.color[1], e.color[2], e.alpha, e.text_color[0], e.text_color[1], e.text_color[2], e.text_size, e.value,
            e.visible as u8, escape(&e.text)
        );
    }
    for (child, parent) in &st.edits.hier {
        out += &format!("hier\t{child}\t{parent}\n");
    }
    let mut graphed: Vec<&String> = st.edits.graphs.keys().collect();
    graphed.sort();
    for name in graphed {
        out += &format!("graph\t{}\t{}\n", name, st.edits.graphs[name].to_json().to_string());
    }
    let mut scripted: Vec<&String> = st.edits.scripts.keys().collect();
    scripted.sort();
    for name in scripted {
        out += &format!("script\t{}\t{}\n", name, escape(&st.edits.scripts[name]));
    }
    let mut textured: Vec<(&String, &String)> = st.edits.base.iter().filter_map(|(n, e)| e.texture.as_ref().map(|t| (n, t))).collect();
    textured.sort();
    for (name, tex) in textured {
        out += &format!("texture\t{}\t{name}\n", to_stored(tex, &dir));
    }
    for e in &st.edits.extras {
        if let Some(tex) = &e.texture {
            out += &format!("texture\t{}\t{}\n", to_stored(tex, &dir), e.name);
        }
    }
    out
}

pub fn save(project: &Path, st: &ProjectState) -> Result<(), String> {
    let text = to_text(&project_dir(project), st);
    std::fs::write(project, text).map_err(|e| format!("{}: {e}", project.display()))
}

/// One-line form of a multi-line text: backslash, newline and tab are escaped.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Splits `line` into `n` whitespace-separated tokens and the remaining text (a name that may contain spaces).
fn take_fields(line: &str, n: usize) -> Option<(Vec<&str>, &str)> {
    let mut rest = line.trim_start();
    let mut fields = Vec::with_capacity(n);
    for i in 0..n {
        let end = match rest.find(char::is_whitespace) {
            Some(e) => e,
            None if i + 1 == n => rest.len(), // the last field may end the line (e.g. an empty trailing path)
            None => return None,
        };
        fields.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    Some((fields, rest.trim()))
}

fn floats(f: &[&str]) -> Option<Vec<f32>> {
    f.iter().map(|s| s.parse().ok()).collect()
}

pub fn load(project: &Path) -> Result<ProjectState, String> {
    let text = std::fs::read_to_string(project).map_err(|e| format!("{}: {e}", project.display()))?;
    from_text(&text, &project_dir(project))
}

/// Reads project text; relative paths are resolved against `dir`.
pub fn from_text(text: &str, dir: &Path) -> Result<ProjectState, String> {
    let dir = dir.to_path_buf();
    let mut lines = text.lines();
    let version: u32 = lines
        .next()
        .and_then(|l| l.strip_prefix(MAGIC).or_else(|| l.strip_prefix(LEGACY_MAGIC)))
        .and_then(|v| v.trim().parse().ok())
        .ok_or("not a Polystation project file")?;
    if version > VERSION {
        return Err(format!("project was saved by a newer version (format {version})"));
    }
    let mut st = ProjectState {
        model: String::new(),
        atlas: None,
        imports: Vec::new(),
        build: Default::default(),
        camera: None,
        view: ViewState { mode: 0, affine: true, grid: true, cull: false },
        hidden: Vec::new(),
        selected: None,
        edits: Edits::default(),
    };
    let mut legacy_fog: Option<(u8, f32)> = None; // version 3 stored fog in the view line
    let mut saw_env = false;
    for (n, line) in lines.enumerate() {
        let bad = |what: &str| format!("line {}: bad {what}", n + 2);
        if let Some(r) = line.strip_prefix("ambient ") {
            let v: Vec<f32> = r.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            if v.len() == 3 || v.len() == 4 {
                st.edits.env.ambient = [v[0].clamp(0.0, 1.0), v[1].clamp(0.0, 1.0), v[2].clamp(0.0, 1.0)];
                // older files only used the ambient light in scenes with lights, and always wrote the old default
                if v.len() == 3 && st.edits.env.ambient == [0.25, 0.25, 0.3] {
                    st.edits.env.ambient = [0.35, 0.35, 0.35];
                }
                if let Some(k) = v.get(3) {
                    st.edits.env.ambient_strength = k.clamp(0.0, 8.0);
                }
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("tree\t") {
            if let Some((name, nums)) = r.split_once('\t') {
                let v: Vec<f32> = nums.split_whitespace().filter_map(|t| t.parse().ok()).collect();
                if let (v, Some(e)) = (v, st.edits.extras.iter_mut().find(|e| e.prim == Prim::Tree && e.name == name)) {
                    if v.len() == 6 {
                        e.tree = TreeParams { kind: TreeKind::from_index(v[0] as usize), trunk: v[1], trunk_width: v[2], crown: v[3], detail: v[4] as u32, seed: v[5] as u32 };
                        e.tree.sanitize();
                    }
                }
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("light\t") {
            if let Some((name, nums)) = r.split_once('\t') {
                let v: Vec<f32> = nums.split_whitespace().filter_map(|t| t.parse().ok()).collect();
                if let (v, Some(e)) = (v, st.edits.extras.iter_mut().find(|e| e.prim == Prim::Light && e.name == name)) {
                    if v.len() == 4 {
                        e.light = LightParams { kind: LightKind::from_index(v[0] as usize), intensity: v[1], range: v[2], angle: v[3] };
                        e.light.sanitize();
                    }
                }
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("collision\t") {
            if let Some((name, mode)) = r.split_once('\t') {
                if let Ok(m) = mode.trim().parse::<usize>() {
                    st.edits.set_collision(name, crate::scene::Collision::from_index(m));
                }
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("gui\t") {
            let mut it = r.splitn(3, '\t');
            if let (Some(name), Some(nums), Some(text)) = (it.next(), it.next(), it.next()) {
                let v: Vec<f32> = nums.split_whitespace().filter_map(|t| t.parse().ok()).collect();
                if v.len() == 16 {
                    let mut e = crate::gui::GuiElem::new(crate::gui::Kind::from_index(v[0] as usize), name);
                    (e.anchor, e.x, e.y, e.w, e.h) = ((v[1] as u8).min(8), v[2], v[3], v[4].max(1.0), v[5].max(1.0));
                    (e.color, e.alpha, e.text_color) = ([v[6], v[7], v[8]].map(|c| c.clamp(0.0, 1.0)), v[9].clamp(0.0, 1.0), [v[10], v[11], v[12]].map(|c| c.clamp(0.0, 1.0)));
                    (e.text_size, e.value, e.visible, e.text) = ((v[13] as u8).clamp(1, 4), v[14].clamp(0.0, 1.0), v[15] != 0.0, unescape(text));
                    st.edits.gui.push(e);
                }
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("build\t") {
            if let Some((name, nums)) = r.split_once('\t') {
                st.build = crate::build::BuildSettings::parse(name, nums);
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("hier\t") {
            if let Some((child, parent)) = r.split_once('\t') {
                st.edits.hier.push((child.to_string(), parent.to_string()));
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("graph\t") {
            if let Some((name, body)) = r.split_once('\t') {
                if let Ok(j) = crate::json::parse(body) {
                    st.edits.graphs.insert(name.to_string(), crate::graph::Graph::from_json(&j));
                }
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("script\t") {
            if let Some((name, body)) = r.split_once('\t') {
                st.edits.scripts.insert(name.to_string(), unescape(body));
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("player\t") {
            let mut parts = r.splitn(3, '\t');
            let (name, nums, sound) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let f: Vec<&str> = nums.split_whitespace().collect();
            let v = floats(&f).filter(|v| v.len() == 15 || v.len() == 16).ok_or_else(|| bad("player"))?;
            let mut p = PlayerParams {
                speed: v[0], sprint_mult: v[1], gravity: v[2], jump_height: v[3], height: v[4], radius: v[5], eye_height: v[6], fov: v[7], bob_amount: v[8], bob_speed: v[9],
                step_sound: if sound.is_empty() { String::new() } else { from_stored(sound, &dir) },
                step_volume: v[10], step_distance: v[11], mouse_sens: v[12], max_step: v[13], air_control: v[14], scale: v.get(15).copied().unwrap_or(1.0),
            };
            p.sanitize();
            if let Some(e) = st.edits.extras.iter_mut().find(|e| e.prim == Prim::Player && e.name == name) {
                e.player = p;
            }
            continue;
        }
        if let Some(r) = line.strip_prefix("scatter\t") {
            let mut parts = r.splitn(4, '\t');
            let (name, target, nums, source) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let f: Vec<&str> = nums.split_whitespace().collect();
            let v = floats(&f).filter(|v| v.len() == 12).ok_or_else(|| bad("scatter"))?;
            let mut s = ScatterParams { target: target.to_string(), shape: Shape::from_index(v[5] as usize), count: v[6] as u32, seed: v[7] as u32, min_size: v[8], max_size: v[9], max_slope: v[10], sink: v[11], source: source.to_string() };
            s.sanitize();
            st.edits.extras.push(Extra {
                light: Default::default(), tree: Default::default(), player: Default::default(), scatter: s,
                texture: None,
                particles: Default::default(),
                terrain: Default::default(),
                prim: Prim::Scatter,
                name: name.to_string(),
                xform: Xform::at([0.0; 3]),
                color: (v[0] > 0.5).then_some([v[1], v[2], v[3]]),
                alpha: v[4].clamp(0.0, 1.0),
            });
            continue;
        }
        if let Some(r) = line.strip_prefix("texture\t") {
            // tab separated, because both the path and the node name may contain spaces
            let (path, name) = r.split_once('\t').ok_or_else(|| bad("texture"))?;
            let path = from_stored(path, &dir);
            match st.edits.extras.iter_mut().find(|e| e.name == name) {
                Some(e) => e.texture = Some(path),
                None => st.edits.base.entry(name.to_string()).or_default().texture = Some(path),
            }
            continue;
        }
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        match key {
            "model" => st.model = from_stored(rest, &dir),
            "atlas" => st.atlas = Some(from_stored(rest, &dir)),
            "guimode" => st.edits.gui_mode = crate::gui::Mode::from_index(rest.parse().unwrap_or(2)),
            "import" => st.imports.push(from_stored(rest, &dir)),
            "camera" => {
                // version 1: orbit camera (yaw pitch distance focus.xyz); convert to the fly camera
                let v = floats(&rest.split_whitespace().collect::<Vec<_>>()).filter(|v| v.len() == 6).ok_or_else(|| bad("camera"))?;
                let (yaw, pitch, dist, focus) = (v[0], v[1], v[2], [v[3], v[4], v[5]]);
                let eye = [focus[0] + dist * pitch.cos() * yaw.sin(), focus[1] + dist * pitch.sin(), focus[2] + dist * pitch.cos() * yaw.cos()];
                st.camera = Some(CameraState { yaw: -yaw, pitch: -pitch, pos: eye, speed: 1.0 });
            }
            "camera2" => {
                let v = floats(&rest.split_whitespace().collect::<Vec<_>>()).filter(|v| v.len() == 6).ok_or_else(|| bad("camera2"))?;
                st.camera = Some(CameraState { yaw: v[0], pitch: v[1], pos: [v[2], v[3], v[4]], speed: v[5] });
            }
            "view" => {
                let t: Vec<&str> = rest.split_whitespace().collect();
                if t.len() != 4 && t.len() != 6 {
                    return Err(bad("view"));
                }
                let flag = |i: usize| t[i].parse::<u8>().map(|v| v != 0).map_err(|_| bad("view"));
                st.view.mode = t[0].parse::<u8>().map_err(|_| bad("view"))?.min(5);
                st.view.affine = flag(1)?;
                st.view.grid = flag(2)?;
                st.view.cull = flag(3)?;
                if t.len() == 6 {
                    let fog = t[4].parse::<u8>().map_err(|_| bad("view"))?.min(2);
                    legacy_fog = Some((fog, t[5].parse::<f32>().map_err(|_| bad("view"))?.clamp(0.05, 20.0)));
                }
            }
            "env" => {
                let (f, extra) = take_fields(rest, 8).ok_or_else(|| bad("env"))?;
                let v = floats(&f[1..]).ok_or_else(|| bad("env"))?;
                let e = &mut st.edits.env;
                e.fog = f[0].parse::<u8>().map_err(|_| bad("env"))?.min(2);
                e.fog_color = [v[0], v[1], v[2]];
                e.fog_start = v[3].max(0.0);
                e.fog_end = v[4].max(e.fog_start + 0.01);
                e.dither = v[5].clamp(1.0, 12.0);
                e.steps = (v[6] as u32).clamp(2, 32);
                e.render_distance = extra.split_whitespace().next().and_then(|t| t.parse::<f32>().ok()).unwrap_or(0.0).max(0.0);
                saw_env = true;
            }
            "sky" => {
                let (f, path) = take_fields(rest, 10).ok_or_else(|| bad("sky"))?;
                let v = floats(&f[1..]).ok_or_else(|| bad("sky"))?;
                let s = &mut st.edits.env.sky;
                s.kind = match f[0] {
                    "1" => scene::SkyKind::Gradient,
                    "2" => scene::SkyKind::Texture,
                    _ => scene::SkyKind::Solid,
                };
                (s.solid, s.top, s.bottom) = ([v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]);
                s.texture = (!path.is_empty()).then(|| from_stored(path, &dir));
            }
            "hidden" => st.hidden.push(rest.to_string()),
            "selected" => st.selected = Some(rest.to_string()),
            "edit" => {
                let (f, name) = take_fields(rest, 15).ok_or_else(|| bad("edit"))?;
                let x = floats(&f[..9]).ok_or_else(|| bad("edit"))?;
                let c = floats(&f[12..15]).ok_or_else(|| bad("edit color"))?;
                let xf = Xform { pos: [x[0], x[1], x[2]], rot: [x[3], x[4], x[5]], scale: [x[6], x[7], x[8]] };
                st.edits.base.insert(
                    name.to_string(),
                    NodeEdit { texture: None, xform: (f[9] == "1").then_some(xf), deleted: f[10] == "1", color: (f[11] == "1").then_some([c[0], c[1], c[2]]), alpha: None },
                );
            }
            "prim" => {
                let (f, name) = take_fields(rest, 14).ok_or_else(|| bad("prim"))?;
                let prim = Prim::from_name(f[0]).ok_or_else(|| bad("primitive kind"))?;
                let x = floats(&f[1..10]).ok_or_else(|| bad("prim"))?;
                let c = floats(&f[11..14]).ok_or_else(|| bad("prim color"))?;
                st.edits.extras.push(Extra { light: Default::default(), tree: Default::default(), player: Default::default(), scatter: Default::default(), texture: None, particles: Default::default(), terrain: Default::default(),
                    prim,
                    name: name.to_string(),
                    xform: Xform { pos: [x[0], x[1], x[2]], rot: [x[3], x[4], x[5]], scale: [x[6], x[7], x[8]] },
                    color: (f[10] == "1").then_some([c[0], c[1], c[2]]),
                    alpha: 1.0,
                });
            }
            "terrain" | "terrain2" => {
                let off = if key == "terrain2" { 2 } else { 0 };
                let t: Vec<&str> = rest.split_whitespace().collect();
                if t.len() < 16 + off {
                    return Err(bad("terrain"));
                }
                let n: usize = t[15 + off].parse().map_err(|_| bad("terrain layers"))?;
                if n > crate::terrain::MAX_LAYERS || t.len() < 16 + off + 4 * n {
                    return Err(bad("terrain layers"));
                }
                let x = floats(&t[..9]).ok_or_else(|| bad("terrain"))?;
                let c = floats(&t[10..13]).ok_or_else(|| bad("terrain color"))?;
                let mut layers = Vec::new();
                for k in 0..n {
                    let f = &t[16 + off + 4 * k..20 + off + 4 * k];
                    layers.push(Layer {
                        kind: NoiseKind::from_index(f[0].parse().map_err(|_| bad("terrain layer"))?),
                        seed: f[1].parse().map_err(|_| bad("terrain layer"))?,
                        scale: f[2].parse().map_err(|_| bad("terrain layer"))?,
                        amp: f[3].parse().map_err(|_| bad("terrain layer"))?,
                    });
                }
                st.edits.extras.push(Extra { light: Default::default(), tree: Default::default(), player: Default::default(), scatter: Default::default(), texture: None, particles: Default::default(),
                    terrain: TerrainParams { res: t[14].parse::<u32>().map_err(|_| bad("terrain resolution"))?.clamp(crate::terrain::MIN_RES, crate::terrain::MAX_RES),
                        layers,
                        chunked: off > 0 && t[15] == "1",
                        view_dist: if off > 0 { t[16].parse::<u32>().unwrap_or(3).clamp(1, crate::terrain::MAX_VIEW_DIST) } else { 3 },
                    },
                    prim: Prim::Terrain,
                    name: t[16 + off + 4 * n..].join(" "),
                    xform: Xform { pos: [x[0], x[1], x[2]], rot: [x[3], x[4], x[5]], scale: [x[6], x[7], x[8]] },
                    color: (t[9] == "1").then_some([c[0], c[1], c[2]]),
                    alpha: t[13].parse::<f32>().map_err(|_| bad("terrain alpha"))?.clamp(0.0, 1.0),
                });
            }
            "particles" => {
                let (f, name) = take_fields(rest, 13).ok_or_else(|| bad("particles"))?;
                let v = floats(&f[1..]).ok_or_else(|| bad("particles"))?;
                let mut p = ParticleParams { velocity: [v[4], v[5], v[6]], gravity: v[7], size: v[8], amount: v[9] as u32, box_size: v[10], lifetime: v[11] };
                p.sanitize();
                st.edits.extras.push(Extra {
                    light: Default::default(), tree: Default::default(), player: Default::default(), scatter: Default::default(), texture: None,
                    particles: p,
                    terrain: Default::default(),
                    prim: Prim::Particles,
                    name: name.to_string(),
                    xform: Xform::at([0.0; 3]),
                    color: (f[0] == "1").then_some([v[0], v[1], v[2]]),
                    alpha: v[3].clamp(0.0, 1.0),
                });
            }
            "edit3" => {
                let (f, name) = take_fields(rest, 17).ok_or_else(|| bad("edit3"))?;
                let x = floats(&f[..9]).ok_or_else(|| bad("edit3"))?;
                let c = floats(&f[12..15]).ok_or_else(|| bad("edit3 color"))?;
                let a: f32 = f[16].parse().map_err(|_| bad("edit3 alpha"))?;
                let xf = Xform { pos: [x[0], x[1], x[2]], rot: [x[3], x[4], x[5]], scale: [x[6], x[7], x[8]] };
                st.edits.base.insert(
                    name.to_string(),
                    NodeEdit { texture: None,
                        xform: (f[9] == "1").then_some(xf),
                        deleted: f[10] == "1",
                        color: (f[11] == "1").then_some([c[0], c[1], c[2]]),
                        alpha: (f[15] == "1").then_some(a.clamp(0.0, 1.0)),
                    },
                );
            }
            "prim3" => {
                let (f, name) = take_fields(rest, 15).ok_or_else(|| bad("prim3"))?;
                let prim = Prim::from_name(f[0]).ok_or_else(|| bad("primitive kind"))?;
                let x = floats(&f[1..10]).ok_or_else(|| bad("prim3"))?;
                let c = floats(&f[11..14]).ok_or_else(|| bad("prim3 color"))?;
                let a: f32 = f[14].parse().map_err(|_| bad("prim3 alpha"))?;
                st.edits.extras.push(Extra { light: Default::default(), tree: Default::default(), player: Default::default(), scatter: Default::default(), texture: None, particles: Default::default(), terrain: Default::default(),
                    prim,
                    name: name.to_string(),
                    xform: Xform { pos: [x[0], x[1], x[2]], rot: [x[3], x[4], x[5]], scale: [x[6], x[7], x[8]] },
                    color: (f[10] == "1").then_some([c[0], c[1], c[2]]),
                    alpha: a.clamp(0.0, 1.0),
                });
            }
            _ => {} // unknown keys are skipped so newer files still open
        }
    }
    if let (false, Some((fog, scale))) = (saw_env, legacy_fog) {
        let e = &mut st.edits.env;
        e.fog = fog;
        e.fog_start *= scale;
        e.fog_end *= scale;
    }
    if st.model.is_empty() {
        return Err("project has no model".into());
    }
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path) -> ProjectState {
        let mut edits = Edits::default();
        edits.base.insert(
            "floater pad 1".into(),
            NodeEdit { texture: None, xform: Some(Xform { pos: [1.0, 2.0, 3.0], rot: [10.0, 20.0, 30.0], scale: [1.0, 2.0, 1.0] }), color: Some([0.5, 0.25, 1.0]), alpha: Some(0.4), deleted: false },
        );
        edits.base.insert("Cylinder".into(), NodeEdit { texture: None, xform: None, color: None, alpha: None, deleted: true });
        edits.extras.push(Extra { light: Default::default(), tree: Default::default(), player: Default::default(), scatter: Default::default(), texture: None, particles: Default::default(), terrain: Default::default(), prim: Prim::Torus, name: "My Torus".into(), xform: Xform::at([4.0, 0.0, -2.0]), color: Some([0.1, 0.2, 0.3]), alpha: 0.25 });
        edits.extras.push(Extra { light: Default::default(), tree: Default::default(), player: Default::default(), scatter: Default::default(), texture: Some(dir.join("tex dir").join("brick.bmp").to_string_lossy().into_owned()), particles: Default::default(), terrain: Default::default(), prim: Prim::Cube, name: "Cube".into(), xform: Xform::at([0.0; 3]), color: None, alpha: 1.0 });
        edits.extras.push(Extra {
            light: Default::default(), tree: Default::default(), player: Default::default(), scatter: Default::default(), texture: Some(dir.join("flake.bmp").to_string_lossy().into_owned()),
            particles: ParticleParams { velocity: [0.5, -2.0, 0.25], gravity: 3.0, size: 0.2, amount: 4321, box_size: 33.0, lifetime: 7.5 },
            terrain: Default::default(),
            prim: Prim::Particles,
            name: "Snow Fall".into(),
            xform: Xform::at([0.0; 3]),
            color: Some([0.9, 0.95, 1.0]),
            alpha: 0.6,
        });
        edits.env.ambient = [0.1, 0.2, 0.3];
        edits.extras.push(Extra {
            light: LightParams { kind: LightKind::Spot, intensity: 2.5, range: 30.0, angle: 70.0 },
            tree: Default::default(),
            player: Default::default(),
            scatter: Default::default(),
            texture: None,
            particles: Default::default(),
            terrain: Default::default(),
            prim: Prim::Light,
            name: "Lamp".into(),
            xform: Xform::at([0.0, 5.0, 0.0]),
            color: Some([1.0, 0.8, 0.5]),
            alpha: 1.0,
        });
        edits.set_collision("Cube", crate::scene::Collision::Trigger);
        edits.gui_mode = crate::gui::Mode::Hold;
        edits.graphs.insert("Hero".into(), crate::graph::Graph::starter());
        edits.gui = vec![
            { let mut e = crate::gui::GuiElem::new(crate::gui::Kind::Bar, "Health"); e.value = 0.25; e.text = "HP\tline".into(); e },
            { let mut b = crate::gui::GuiElem::new(crate::gui::Kind::Button, "Play"); (b.anchor, b.visible, b.x) = (7, false, -12.5); b },
        ];
        edits.hier = vec![("Hero".into(), "Rolling Hills".into()), ("Cube".into(), String::new())];
        edits.set_collision("Rolling Hills", crate::scene::Collision::Passable);
        edits.scripts.insert("Hero".into(), "on update {\n\trotation.y += 90 * dt // back\\slash \\n\n}\n".into());
        edits.base.entry("floater pad 1".into()).or_default().texture = Some(dir.join("moss.bmp").to_string_lossy().into_owned());
        edits.extras.push(Extra {
            light: Default::default(), tree: Default::default(), player: PlayerParams { speed: 7.5, gravity: 14.0, jump_height: 2.0, fov: 90.0, scale: 2.5, bob_amount: 0.12, step_sound: dir.join("step.wav").to_string_lossy().into_owned(), ..Default::default() },
            scatter: Default::default(),
            texture: None,
            particles: Default::default(),
            terrain: Default::default(),
            prim: Prim::Player,
            name: "Hero".into(),
            xform: Xform::at([1.0, 0.0, 2.0]),
            color: Some([0.9, 0.5, 0.2]),
            alpha: 1.0,
        });
        edits.extras.push(Extra {
            light: Default::default(), tree: Default::default(), player: Default::default(), scatter: ScatterParams { target: "Rolling Hills".into(), shape: Shape::Rock, count: 77, seed: 12, min_size: 0.4, max_size: 1.1, max_slope: 33.0, sink: 0.05, source: "Oak".into() },
            texture: Some(dir.join("rock.bmp").to_string_lossy().into_owned()),
            particles: Default::default(),
            terrain: Default::default(),
            prim: Prim::Scatter,
            name: "Rocks".into(),
            xform: Xform::at([0.0; 3]),
            color: Some([0.5, 0.5, 0.45]),
            alpha: 1.0,
        });
        edits.extras.push(Extra { light: Default::default(), tree: Default::default(), player: Default::default(), scatter: Default::default(), texture: None, particles: Default::default(),
            terrain: TerrainParams {
                res: 48,
                chunked: true,
                view_dist: 5,
                layers: vec![
                    Layer { kind: NoiseKind::Ridged, seed: 7, scale: 0.25, amp: 0.4 },
                    Layer { kind: NoiseKind::Cellular, seed: 99, scale: 0.05, amp: 0.02 },
                ],
            },
            prim: Prim::Terrain,
            name: "Rolling Hills".into(),
            xform: Xform { pos: [3.0, 0.0, 4.0], rot: [0.0, 15.0, 0.0], scale: [10.0, 4.0, 10.0] },
            color: Some([0.3, 0.5, 0.3]),
            alpha: 0.8,
        });
        edits.env.fog = 1;
        edits.env.fog_color = [0.5, 0.25, 0.75];
        edits.env.fog_start = 2.5;
        edits.env.fog_end = 9.0;
        edits.env.dither = 6.0;
        edits.env.steps = 7;
        edits.env.render_distance = 250.0;
        edits.env.sky.kind = scene::SkyKind::Texture;
        edits.env.sky.top = [0.1, 0.2, 0.3];
        edits.env.sky.texture = Some(dir.join("sky maps").join("dusk.bmp").to_string_lossy().into_owned());
        ProjectState {
            model: dir.join("models").join("my scene.blend").to_string_lossy().into_owned(),
            atlas: None,
            imports: vec![dir.join("models").join("tree.blend").to_string_lossy().into_owned()],
            build: crate::build::BuildSettings { name: "My Game".into(), width: 640, height: 480, fullscreen: true, vsync: false, fps_limit: 30, show_fps: true },
            camera: Some(CameraState { yaw: 1.25, pitch: -0.5, pos: [1.0, 2.0, -3.0], speed: 2.5 }),
            view: ViewState { mode: 3, affine: false, grid: true, cull: true },
            hidden: vec!["Cylinder".into(), "floater pad 1".into()],
            selected: Some("Plane.001".into()),
            edits,
        }
    }

    #[test]
    fn round_trip_with_relative_paths_and_edits() {
        let dir = std::env::temp_dir().join("gpr_project_test");
        std::fs::create_dir_all(dir.join("models")).unwrap();
        let st = sample(&dir);
        let file = dir.join("test.gpr");
        save(&file, &st).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("model models/my scene.blend"), "model should be stored relative:\n{text}");
        let back = load(&file).unwrap();
        assert_eq!(back.edits, st.edits);
        assert_eq!(back.camera, st.camera);
        assert_eq!(back.view, st.view);
        assert_eq!(back.hidden, st.hidden);
        assert_eq!(back.selected, st.selected);
        assert_eq!(back.imports, st.imports);
        assert_eq!(back.build, st.build);
        assert_eq!(Path::new(&back.model), Path::new(&st.model));
    }

    #[test]
    fn version_three_fog_moves_into_the_environment() {
        let f = std::env::temp_dir().join("gpr_v3.gpr");
        std::fs::write(&f, "GPURENDER-PROJECT 3\nmodel a.obj\nview 2 1 1 0 1 2.0\n").unwrap();
        let st = load(&f).unwrap();
        assert_eq!(st.view.mode, 2);
        assert_eq!(st.edits.env.fog, 1);
        assert!((st.edits.env.fog_start - 3.0).abs() < 1e-5 && (st.edits.env.fog_end - 16.0).abs() < 1e-5);
    }

    #[test]
    fn sky_without_texture_round_trips() {
        let dir = std::env::temp_dir().join("gpr_sky_test");
        std::fs::create_dir_all(&dir).unwrap();
        let mut st = sample(&dir);
        st.edits.env.sky.texture = None;
        st.edits.env.sky.kind = scene::SkyKind::Gradient;
        let file = dir.join("sky.gpr");
        save(&file, &st).unwrap();
        assert_eq!(load(&file).unwrap().edits.env, st.edits.env);
    }

    #[test]
    fn version_one_orbit_camera_converts() {
        let f = std::env::temp_dir().join("gpr_v1.gpr");
        std::fs::write(&f, "GPURENDER-PROJECT 1\nmodel a.obj\ncamera 0 0 10 0 0 0\nhidden X\n").unwrap();
        let st = load(&f).unwrap();
        let c = st.camera.unwrap();
        // eye at +z looking at the origin: yaw 0 -> forward -z
        assert!((c.pos[2] - 10.0).abs() < 1e-4 && c.yaw.abs() < 1e-6);
        assert_eq!(st.hidden, vec!["X".to_string()]);
    }

    #[test]
    fn rejects_other_files() {
        let f = std::env::temp_dir().join("gpr_not_a_project.gpr");
        std::fs::write(&f, "hello\n").unwrap();
        assert!(load(&f).is_err());
    }
}
