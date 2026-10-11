//! Editor-side scene state layered on top of the loaded model.
//!
//! The loaded model (`base`) is never modified. Everything the user does is recorded as [`Edits`]:
//! per-node overrides (keyed by node name, so they survive hot reloads) and added primitives. The
//! displayed mesh is [`compose`]d from the model plus those primitives; cheap whole-state snapshots
//! of `Edits` give undo/redo.
use crate::obj::{self, Kind, Mesh, Node};
use crate::particles::ParticleParams;
use crate::prims::{self, Prim};
use crate::light::LightParams;
use crate::player::PlayerParams;
use crate::trees::TreeParams;
use crate::scatter::{self, ScatterParams};
use crate::terrain::{self, TerrainParams};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    /// World position of the node's pivot.
    pub pos: [f32; 3],
    /// Euler angles in degrees (applied X, then Y, then Z).
    pub rot: [f32; 3],
    pub scale: [f32; 3],
}

impl Xform {
    pub fn at(pos: [f32; 3]) -> Self {
        Xform { pos, rot: [0.0; 3], scale: [1.0; 3] }
    }
}

/// Overrides for a node that came from the model file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeEdit {
    pub xform: Option<Xform>,
    /// Replaces the node's texture with a flat colour.
    pub color: Option<[f32; 3]>,
    /// Opacity 0..1 (None = 1).
    pub alpha: Option<f32>,
    /// An image (BMP) drawn on the object; the colour then tints it.
    pub texture: Option<String>,
    pub deleted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Extra {
    pub prim: Prim,
    pub name: String,
    pub xform: Xform,
    pub color: Option<[f32; 3]>,
    pub alpha: f32,
    /// Only used when `prim` is [`Prim::Terrain`].
    pub terrain: TerrainParams,
    /// Only used when `prim` is [`Prim::Particles`].
    pub particles: ParticleParams,
    /// Only used when `prim` is [`Prim::Scatter`].
    pub scatter: ScatterParams,
    /// Only used when `prim` is [`Prim::Player`].
    pub player: PlayerParams,
    /// Only used when `prim` is [`Prim::Light`].
    pub light: LightParams,
    /// Only used when `prim` is [`Prim::Tree`].
    pub tree: TreeParams,
    /// Image drawn on the object (for particles: on each billboard); the colour tints it.
    pub texture: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edits {
    pub base: HashMap<String, NodeEdit>,
    pub extras: Vec<Extra>,
    pub env: Environment,
    /// Script source per object name (see the script module).
    pub scripts: HashMap<String, String>,
    /// How the player collides with an object, by object name (absent = solid).
    pub collision: HashMap<String, Collision>,
    /// The hierarchy the user arranged in the explorer: (node name, parent name) in display order,
    /// parent "" = the scene root. Empty = the hierarchy of the model (see the hier module).
    pub hier: Vec<(String, String)>,
    /// The game's GUI elements, drawn back to front (see the gui module).
    pub gui: Vec<crate::gui::GuiElem>,
    /// How the mouse works while playing (see the gui module).
    pub gui_mode: crate::gui::Mode,
    /// The Camera object the game looks through from the start (None: the player's eyes).
    pub game_camera: Option<String>,
    /// Edited materials (see [`MaterialEdit`]); also holds the settings of the materials made in Polystation.
    pub materials: HashMap<String, MaterialEdit>,
    /// Materials made in Polystation (not from a model), in the order they were made.
    pub user_materials: Vec<String>,
    /// The material assigned to an object, by object name.
    pub assign: HashMap<String, String>,
    /// The visual scripts, by object name (the script text of such an object is generated from its graph).
    pub graphs: HashMap<String, crate::graph::Graph>,
}

/// How a texture covers the faces that use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TexMode {
    /// The texture tiles wherever the UVs leave 0..1.
    #[default]
    Repeat,
    /// One copy of the texture is stretched over all the UVs of the material.
    Stretch,
    /// Like repeat, but every other tile is flipped.
    Mirror,
    /// UVs outside 0..1 keep the colour of the texture's edge.
    Clamp,
}

impl TexMode {
    pub const NAMES: [&'static str; 4] = ["Repeat", "Stretch", "Mirror", "Clamp"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> TexMode {
        [TexMode::Repeat, TexMode::Stretch, TexMode::Mirror, TexMode::Clamp][i.min(3)]
    }
}

/// What the user set on a material. Keyed by the material's name (so it survives a reload of the Blender
/// file). For a material of a model a field of None is still what the model says; for a material made in
/// Polystation it is the default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialEdit {
    pub color: Option<[f32; 3]>,
    pub texture: Option<String>,
    pub alpha: Option<f32>,
    pub mode: Option<TexMode>,
    /// A Camera object whose view is shown on the surface (instead of the texture).
    pub camera: Option<String>,
    /// How many times the texture repeats over the UVs (u, v).
    pub tile: Option<[f32; 2]>,
    /// Shifts the texture (in texture widths).
    pub offset: Option<[f32; 2]>,
    /// Slides the texture, in texture widths per second.
    pub scroll: Option<[f32; 2]>,
    /// Turns the texture, degrees.
    pub rotate: Option<f32>,
    /// 0 = lit by the scene, 1 = shines by itself.
    pub glow: Option<f32>,
    /// Multiplies the texture / color.
    pub bright: Option<f32>,
    /// Pixels of the texture below this alpha are cut out (0 = off).
    pub cutout: Option<f32>,
    /// Hide the back faces of this material (even when the view's backface culling is off).
    pub back_cull: Option<bool>,
    /// Not hidden by fog.
    pub no_fog: Option<bool>,
}

impl MaterialEdit {
    pub fn is_empty(&self) -> bool {
        *self == MaterialEdit::default()
    }

    /// Puts the surface settings of this material over `s`.
    pub fn apply(&self, s: &mut Surface) {
        if let Some(v) = self.tile {
            s.tile = v;
        }
        if let Some(v) = self.offset {
            s.offset = v;
        }
        if let Some(v) = self.scroll {
            s.scroll = v;
        }
        if let Some(v) = self.rotate {
            s.rotate = v;
        }
        if let Some(v) = self.glow {
            s.glow = v;
        }
        if let Some(v) = self.bright {
            s.bright = v;
        }
        if let Some(v) = self.cutout {
            s.cutout = v;
        }
        if let Some(v) = self.back_cull {
            s.back_cull = v;
        }
        if let Some(v) = self.no_fog {
            s.no_fog = v;
        }
    }
}

/// The settings of a material that decide how a surface is shaded, with their defaults filled in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub tile: [f32; 2],
    pub offset: [f32; 2],
    pub scroll: [f32; 2],
    pub rotate: f32,
    pub glow: f32,
    pub bright: f32,
    pub cutout: f32,
    pub back_cull: bool,
    pub no_fog: bool,
}

impl Default for Surface {
    fn default() -> Surface {
        Surface { tile: [1.0, 1.0], offset: [0.0; 2], scroll: [0.0; 2], rotate: 0.0, glow: 0.0, bright: 1.0, cutout: 0.0, back_cull: false, no_fog: false }
    }
}

/// What happens when the player meets an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Collision {
    /// Blocks the player.
    #[default]
    Solid,
    /// The player walks straight through.
    Passable,
    /// Passable, and the object's script gets `on touch` / `on leave`.
    Trigger,
    /// A body of water: passable, the player wades and swims in it, and its surface ripples.
    Water,
}

impl Collision {
    pub const NAMES: [&'static str; 4] = ["Solid", "Passable", "Trigger", "Water"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Collision {
        [Collision::Solid, Collision::Passable, Collision::Trigger, Collision::Water][i.min(3)]
    }

    pub fn blocks(self) -> bool {
        self == Collision::Solid
    }
}

impl Edits {
    pub fn collision_of(&self, name: &str) -> Collision {
        self.collision.get(name).copied().unwrap_or_default()
    }

    /// Sets an object's collision (solid is the default, so it is stored as the absence of an entry).
    pub fn set_collision(&mut self, name: &str, c: Collision) {
        if c == Collision::Solid {
            self.collision.remove(name);
        } else {
            self.collision.insert(name.to_string(), c);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkyKind {
    Solid,
    Gradient,
    /// An equirectangular (360 degree panorama) image.
    Texture,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sky {
    pub kind: SkyKind,
    pub solid: [f32; 3],
    pub top: [f32; 3],
    pub bottom: [f32; 3],
    pub texture: Option<String>,
}

/// World settings: fog and sky. Part of [`Edits`], so it is undoable and saved with the project.
#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
    /// 0 off, 1 smooth, 2 dithered.
    pub fog: u8,
    pub fog_color: [f32; 3],
    /// Fog start / full-fog distances, in multiples of the scene radius.
    pub fog_start: f32,
    pub fog_end: f32,
    /// Dither block size in pixels at 96 dpi.
    pub dither: f32,
    /// Number of fog brightness steps in dithered mode.
    pub steps: u32,
    /// Nothing farther than this (world units) is drawn; 0 = no limit.
    pub render_distance: f32,
    pub sky: Sky,
    /// Light that reaches everything, added to the lights of the scene.
    pub ambient: [f32; 3],
    /// Multiplier for the ambient light (above 1 brightens past the colour).
    pub ambient_strength: f32,
    /// Colour depth of the picture: 4, 8, 16 or 32 bits (see the shaders).
    pub color_bits: u8,
}

pub const BACKGROUND: [f32; 3] = [0.08, 0.09, 0.125];

impl Default for Environment {
    fn default() -> Self {
        Environment {
            fog: 2,
            fog_color: BACKGROUND,
            fog_start: 1.5,
            fog_end: 8.0,
            dither: 3.0,
            steps: 4,
            render_distance: 0.0,
            ambient: [0.35, 0.35, 0.35],
            ambient_strength: 1.0,
            color_bits: 32,
            sky: Sky { kind: SkyKind::Solid, solid: BACKGROUND, top: [0.12, 0.2, 0.42], bottom: [0.55, 0.62, 0.75], texture: None },
        }
    }
}

pub const ENV_NODE: &str = "Environment";

pub const DEFAULT_PRIM_COLOR: [f32; 3] = [0.72, 0.74, 0.78];
pub const PRIMS_COLLECTION: &str = "Primitives";

/// Model + added primitives as one mesh. Added primitives live under a "Primitives" collection
/// placed right after the model's own nodes.
/// The geometry of the scene object (or collection) called `name`, scaled to one unit tall and
/// standing on y = 0 around the origin, with each polygon's material rectangle.
pub fn custom_template(base: &Mesh, extras: &[Extra], name: &str) -> Option<Vec<scatter::Placed>> {
    let mut out: Vec<scatter::Placed> = Vec::new();
    if let Some(idx) = base.nodes.iter().position(|n| n.name == name && matches!(n.kind, Kind::Object | Kind::Collection)) {
        let inside = |mut i: usize| loop {
            if i == idx {
                return true;
            }
            match base.nodes[i].parent {
                Some(p) => i = p,
                None => return false,
            }
        };
        for t in base.indices.chunks_exact(3) {
            let id = base.data[t[0] as usize * obj::FLOATS + obj::FLOATS - 1] as usize;
            if id >= base.nodes.len() || !inside(id) {
                continue;
            }
            let v = |k: usize| {
                let b = t[k] as usize * obj::FLOATS;
                ([base.data[b], base.data[b + 1], base.data[b + 2]], [base.data[b + 3], base.data[b + 4]])
            };
            let b = t[0] as usize * obj::FLOATS;
            let rect = [base.data[b + 8], base.data[b + 9], base.data[b + 10], base.data[b + 11]];
            out.push((vec![v(0), v(1), v(2)], rect));
        }
    } else if let Some(e) = extras.iter().find(|e| e.name == name && !matches!(e.prim, Prim::Scatter | Prim::Terrain | Prim::Particles | Prim::Light | Prim::Player | Prim::Camera)) {
        let faces = if e.prim == Prim::Tree { crate::trees::faces(&e.tree) } else { e.prim.faces() };
        out = faces.into_iter().map(|f| (f, [0.0; 4])).collect();
    }
    if out.is_empty() {
        return None;
    }
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for (f, _) in &out {
        for (p, _) in f {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
    }
    let k = 1.0 / (hi[1] - lo[1]).max(1e-3);
    let (cx, cz) = ((lo[0] + hi[0]) * 0.5, (lo[2] + hi[2]) * 0.5);
    for (f, _) in &mut out {
        for (p, _) in f.iter_mut() {
            *p = [(p[0] - cx) * k, (p[1] - lo[1]) * k, (p[2] - cz) * k];
        }
    }
    Some(out)
}

pub fn compose(base: &Mesh, extras: &[Extra], views: &[(i32, i32)]) -> Mesh {
    let mut nodes = base.nodes.clone();
    nodes.push(Node::new(Kind::Environment, Some(0), ENV_NODE, false)); // always right after the model's nodes
    let mut data = base.data.clone();
    let mut opaque = base.indices[..base.opaque_count].to_vec();
    let blend = base.indices[base.opaque_count..].to_vec();
    let mut wire = base.wire.clone();

    if !extras.is_empty() {
        let root = nodes.len();
        nodes.push(Node::new(Kind::Collection, Some(0), PRIMS_COLLECTION, false));
        let mut max_r = 0.0f32;
        let (mut tris, mut quads) = (0, 0);
        for (idx, e) in extras.iter().enumerate() {
            let node = nodes.len();
            let mut rects: Vec<[f32; 4]> = Vec::new(); // per-face material rectangles (planted custom objects)
            let faces: Vec<obj::Face> = if e.prim == Prim::Terrain {
                if e.terrain.chunked {
                    let (cx, cz) = views.get(idx).copied().unwrap_or((0, 0)); // chunk the camera is in
                    terrain::faces_around(&e.terrain, cx, cz)
                } else {
                    terrain::faces(&e.terrain)
                }
            } else if e.prim == Prim::Scatter {
                // objects on the terrain this layer points at (in that terrain's local space)
                match extras.iter().position(|t| t.prim == Prim::Terrain && t.name == e.scatter.target) {
                    Some(ti) => {
                        let t = &extras[ti];
                        let view = if t.terrain.chunked { views.get(ti).copied().unwrap_or((0, 0)) } else { (0, 0) };
                        let template = (e.scatter.shape == scatter::Shape::Custom).then(|| custom_template(base, extras, &e.scatter.source)).flatten();
                        let placed = scatter::place(&e.scatter, &t.terrain, t.xform.scale, view, template.as_deref());
                        rects = placed.iter().map(|(_, r)| *r).collect();
                        placed.into_iter().map(|(f, _)| f).collect()
                    }
                    None => Vec::new(),
                }
            } else if e.prim == Prim::Player {
                crate::player::faces(&e.player)
            } else if e.prim == Prim::Light {
                crate::light::marker(&e.light)
            } else if e.prim == Prim::Tree {
                crate::scatter::shifted(crate::trees::faces(&e.tree), [0.0, -0.5, 0.0])
            } else {
                e.prim.faces()
            };
            let mut n = Node::new(Kind::Object, Some(root), &e.name, false);
            n.radius = if e.prim == Prim::Camera { 0.6 } else { prims::radius(&faces).max(1e-3) };
            for (fi, f) in faces.iter().enumerate() {
                match f.len() {
                    4 => n.quads += 1,
                    k => n.tris += k - 2,
                }
                obj::push_face(&mut data, &mut opaque, &mut wire, f, rects.get(fi).copied().unwrap_or([0.0; 4]), node);
            }
            max_r = max_r.max(n.radius);
            tris += n.tris;
            quads += n.quads;
            nodes.push(n);
        }
        nodes[root].radius = max_r;
        nodes[root].tris = tris;
        nodes[root].quads = quads;
        nodes[0].tris += tris;
        nodes[0].quads += quads;
    }

    let opaque_count = opaque.len();
    opaque.extend(blend);
    Mesh { nodes, data, indices: opaque, opaque_count, wire, center: base.center, radius: base.radius }
}

/// "Cube" -> "Cube.001" style unique name.
pub fn unique_name(wanted: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(wanted) {
        return wanted.to_string();
    }
    (1..).map(|i| format!("{wanted}.{i:03}")).find(|n| !taken(n)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_appends_primitive_nodes_and_geometry() {
        let base = obj::parse("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
        let extras = vec![Extra { light: Default::default(), tree: Default::default(), player: Default::default(), scatter: Default::default(), texture: None, particles: Default::default(), terrain: Default::default(), prim: Prim::Cube, name: "Cube".into(), xform: Xform::at([0.0; 3]), color: None, alpha: 1.0 }];
        let m = compose(&base, &extras, &[]);
        assert_eq!(m.nodes.len(), base.nodes.len() + 3); // environment + collection + object
        assert_eq!(m.nodes.last().unwrap().name, "Cube");
        assert_eq!(m.nodes.last().unwrap().quads, 54); // a cube is cut into 3x3 pieces per side so vertex lighting has vertices to work with
        assert_eq!(m.indices.len(), base.indices.len() + 54 * 6);
        // vertex node ids of the cube point at its node
        let id = m.data[base.data.len() + obj::FLOATS - 1] as usize;
        assert_eq!(id, m.nodes.len() - 1);
    }

    #[test]
    fn a_custom_template_is_one_unit_tall_and_standing_on_the_ground() {
        let base = obj::parse("v 0 0 0\nv 2 0 0\nv 0 4 0\nv 0 0 2\ngpr_node object 0 0 Tree\ngpr_use 1\nf 1 2 3\nf 1 3 4\n").unwrap();
        let t = custom_template(&base, &[], "Tree").expect("the Tree object is in the model");
        assert_eq!(t.len(), 2);
        let ys: Vec<f32> = t.iter().flat_map(|(f, _)| f.iter().map(|(p, _)| p[1])).collect();
        assert!((ys.iter().cloned().fold(f32::MIN, f32::max) - 1.0).abs() < 1e-4 && ys.iter().cloned().fold(f32::MAX, f32::min).abs() < 1e-4);
        assert!(custom_template(&base, &[], "Nope").is_none());
    }

    #[test]
    fn names_get_numbered() {
        let taken = ["Cube", "Cube.001"];
        assert_eq!(unique_name("Cube", |n| taken.contains(&n)), "Cube.002");
        assert_eq!(unique_name("Sphere", |n| taken.contains(&n)), "Sphere");
    }
}
