//! Starting points for new projects: a few ready-made scenes, and the code that turns one into project
//! files on disk. Every template has the same bones: a ground model (`ground.obj`, written next to the
//! project) plus objects, lights, scripts, GUI and environment settings stored as edits.
use crate::gui::{GuiElem, Kind as GuiKind};
use crate::player::PlayerParams;
use crate::prefabs::PREFABS;
use crate::prims::Prim;
use crate::project::{CameraState, ProjectState, ViewState};
use crate::scene::{Collision, Edits, Extra, SkyKind, Xform};
use std::path::{Path, PathBuf};

pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    /// Colours of the card preview: sky, ground, accent.
    pub colors: [[f32; 3]; 3],
    /// Half the size of the ground square.
    pub ground: f32,
    pub build: fn(&mut Edits),
}

pub const TEMPLATES: [Template; 4] = [
    Template { id: "blank", name: "Blank Room", blurb: "A flat floor, a light and a player. Start from nothing.", colors: [[0.10, 0.12, 0.2], [0.35, 0.36, 0.42], [0.9, 0.9, 1.0]], ground: 20.0, build: blank },
    Template { id: "wetlands", name: "Wetlands Walk", blurb: "Rolling terrain, a pond, pine trees and fog.", colors: [[0.45, 0.62, 0.78], [0.3, 0.5, 0.25], [0.2, 0.4, 0.9]], ground: 60.0, build: wetlands },
    Template { id: "coins", name: "Coin Collector", blurb: "Collect the coins. Has a score counter, scripts and a GUI.", colors: [[0.12, 0.1, 0.25], [0.3, 0.2, 0.45], [1.0, 0.8, 0.1]], ground: 24.0, build: coins },
    Template { id: "hall", name: "Torchlit Hall", blurb: "A dark stone hall with torches, a door and moody fog.", colors: [[0.05, 0.04, 0.05], [0.28, 0.24, 0.22], [1.0, 0.55, 0.15]], ground: 14.0, build: hall },
];

pub fn by_id(id: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.id == id)
}

// ------------------------------------------------------------------------------------------------
// helpers
// ------------------------------------------------------------------------------------------------
fn extra(prim: Prim, name: &str, pos: [f32; 3], scale: [f32; 3], color: Option<[f32; 3]>) -> Extra {
    Extra {
        prim,
        name: name.to_string(),
        xform: Xform { pos, rot: [0.0; 3], scale },
        color,
        alpha: 1.0,
        terrain: Default::default(),
        particles: Default::default(),
        scatter: Default::default(),
        player: PlayerParams::default(),
        light: Default::default(),
        tree: Default::default(),
        texture: None,
    }
}

fn player(e: &mut Edits, at: [f32; 3]) {
    e.extras.push(extra(Prim::Player, "Player", at, [1.0; 3], Some([0.9, 0.55, 0.2])));
}

fn sun(e: &mut Edits, name: &str, rot: [f32; 3], intensity: f32, color: [f32; 3]) {
    let mut l = extra(Prim::Light, name, [0.0, 12.0, 0.0], [1.0; 3], Some(color));
    l.xform.rot = rot;
    l.light.kind = crate::light::LightKind::Sun;
    l.light.intensity = intensity;
    e.extras.push(l);
}

fn point(e: &mut Edits, name: &str, pos: [f32; 3], color: [f32; 3], intensity: f32, range: f32) {
    let mut l = extra(Prim::Light, name, pos, [1.0; 3], Some(color));
    l.light.intensity = intensity;
    l.light.range = range;
    e.extras.push(l);
}

fn solid(e: &mut Edits, prim: Prim, name: &str, pos: [f32; 3], scale: [f32; 3], color: [f32; 3]) {
    e.extras.push(extra(prim, name, pos, scale, Some(color)));
}

/// A ready-made gameplay object (see the prefabs module), placed at `pos`.
fn prefab(e: &mut Edits, which: &str, name: &str, pos: [f32; 3]) {
    let Some(p) = PREFABS.iter().find(|p| p.name == which) else { return };
    let mut x = extra(p.prim, name, pos, p.scale, Some(p.color));
    x.alpha = p.alpha;
    if let Some(l) = p.light {
        x.light = l;
    }
    e.extras.push(x);
    e.set_collision(name, p.collision);
    if !p.script.is_empty() {
        e.scripts.insert(name.to_string(), p.script.to_string());
    }
}

fn label(e: &mut Edits, name: &str, text: &str, anchor: u8, x: f32, y: f32, w: f32, size: u8) {
    let mut g = GuiElem::new(GuiKind::Text, name);
    (g.text, g.anchor, g.x, g.y, g.w, g.h, g.text_size) = (text.to_string(), anchor, x, y, w, 12.0 + 12.0 * size as f32, size);
    e.gui.push(g);
}

// ------------------------------------------------------------------------------------------------
// the templates
// ------------------------------------------------------------------------------------------------
fn blank(e: &mut Edits) {
    e.env.fog = 0;
    e.env.sky.kind = SkyKind::Gradient;
    e.env.sky.top = [0.18, 0.28, 0.5];
    e.env.sky.bottom = [0.6, 0.68, 0.8];
    sun(e, "Sun", [-50.0, 30.0, 0.0], 0.8, [1.0, 0.96, 0.88]);
    solid(e, Prim::Cube, "Crate", [4.0, 0.75, -6.0], [1.5, 1.5, 1.5], [0.75, 0.5, 0.3]);
    player(e, [0.0, 0.0, 6.0]);
    label(e, "Hint", "WASD move   SPACE jump   MOUSE look", 1, 0.0, 18.0, 520.0, 1);
}

fn wetlands(e: &mut Edits) {
    e.env.fog = 2;
    e.env.fog_start = 0.5;
    e.env.fog_end = 2.2;
    e.env.fog_color = [0.62, 0.72, 0.82];
    e.env.sky.kind = SkyKind::Gradient;
    e.env.sky.top = [0.3, 0.5, 0.82];
    e.env.sky.bottom = [0.78, 0.84, 0.9];
    sun(e, "Sun", [-60.0, 40.0, 0.0], 0.85, [1.0, 0.95, 0.85]);
    let mut t = extra(Prim::Terrain, "Hills", [0.0, 0.0, 0.0], [50.0, 3.0, 50.0], Some([0.36, 0.52, 0.28]));
    t.terrain.res = 48;
    e.extras.push(t);
    let mut pond = extra(Prim::Plane, "Pond", [8.0, 0.4, -10.0], [14.0, 1.0, 12.0], Some([0.15, 0.35, 0.75]));
    pond.alpha = 0.75;
    e.extras.push(pond);
    e.set_collision("Pond", Collision::Water);
    let spots = [[-14.0, -6.0], [-9.0, 6.0], [-18.0, 4.0], [14.0, 8.0], [20.0, -4.0], [4.0, 16.0], [-4.0, -18.0]];
    for (i, s) in spots.iter().enumerate() {
        let mut tree = extra(Prim::Tree, &format!("Pine {}", i + 1), [s[0], 0.0, s[1]], [4.0 + (i % 3) as f32, 4.0 + (i % 3) as f32, 4.0 + (i % 3) as f32], None);
        tree.texture = Some(crate::trees::PALETTE.to_string());
        tree.tree.seed = i as u32 * 7 + 1;
        e.extras.push(tree);
    }
    player(e, [0.0, 6.0, 12.0]);
}

fn coins(e: &mut Edits) {
    e.env.fog = 2;
    e.env.fog_color = [0.12, 0.1, 0.25];
    e.env.sky.kind = SkyKind::Solid;
    e.env.sky.solid = [0.12, 0.1, 0.25];
    e.env.ambient = [0.4, 0.38, 0.5];
    sun(e, "Moon", [-70.0, 20.0, 0.0], 0.6, [0.7, 0.75, 1.0]);
    point(e, "Glow", [0.0, 4.0, 0.0], [1.0, 0.85, 0.5], 1.4, 18.0);
    for (i, p) in [[-8.0, -6.0], [6.0, -10.0], [10.0, 4.0], [-4.0, 9.0], [0.0, -3.0], [-12.0, 6.0]].iter().enumerate() {
        prefab(e, "Coin", &format!("Coin {}", i + 1), [p[0], 1.0, p[1]]);
    }
    solid(e, Prim::Cube, "Pillar A", [-6.0, 1.5, 0.0], [1.5, 3.0, 1.5], [0.5, 0.35, 0.65]);
    solid(e, Prim::Cube, "Pillar B", [7.0, 1.5, -4.0], [1.5, 3.0, 1.5], [0.5, 0.35, 0.65]);
    solid(e, Prim::Ramp, "Ramp", [0.0, 0.0, -12.0], [4.0, 2.0, 5.0], [0.6, 0.5, 0.75]);
    prefab(e, "Jump Pad", "Jump Pad", [10.0, 0.1, 10.0]);
    player(e, [0.0, 0.0, 12.0]);
    label(e, "Title", "COIN COLLECTOR", 1, 0.0, 14.0, 520.0, 3);
    label(e, "Help", "Touch a coin to collect it (the count shows below)", 7, 0.0, 18.0, 640.0, 1);
}

fn hall(e: &mut Edits) {
    e.env.fog = 2;
    e.env.steps = 5;
    e.env.fog_start = 0.4;
    e.env.fog_end = 1.6;
    e.env.fog_color = [0.04, 0.03, 0.04];
    e.env.sky.kind = SkyKind::Solid;
    e.env.sky.solid = [0.04, 0.03, 0.04];
    e.env.ambient = [0.12, 0.1, 0.12];
    let stone = [0.45, 0.4, 0.36];
    solid(e, Prim::Cube, "Wall North", [0.0, 2.5, -14.0], [28.0, 5.0, 1.0], stone);
    solid(e, Prim::Cube, "Wall South", [0.0, 2.5, 14.0], [28.0, 5.0, 1.0], stone);
    solid(e, Prim::Cube, "Wall West", [-14.0, 2.5, 0.0], [1.0, 5.0, 28.0], stone);
    solid(e, Prim::Cube, "Wall East", [14.0, 2.5, 0.0], [1.0, 5.0, 28.0], stone);
    solid(e, Prim::Cube, "Ceiling", [0.0, 5.5, 0.0], [29.0, 1.0, 29.0], [0.3, 0.27, 0.25]);
    for (i, x) in [-8.0, 0.0, 8.0].iter().enumerate() {
        solid(e, Prim::Cylinder, &format!("Column {}", i + 1), [*x, 2.5, -4.0], [1.2, 5.0, 1.2], [0.55, 0.5, 0.45]);
    }
    for (i, p) in [[-12.5, -10.0], [12.5, -10.0], [-12.5, 10.0], [12.5, 10.0], [0.0, -12.8]].iter().enumerate() {
        prefab(e, "Torch", &format!("Torch {}", i + 1), [p[0], 2.2, p[1]]);
    }
    prefab(e, "Door", "Door", [0.0, 1.5, 12.8]);
    player(e, [0.0, 0.0, 9.0]);
    label(e, "Hint", "Press E near the door", 1, 0.0, 18.0, 520.0, 1);
}

// ------------------------------------------------------------------------------------------------
// creating a project on disk
// ------------------------------------------------------------------------------------------------
/// A file-system friendly version of a project name.
pub fn clean_name(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' }).collect();
    s.trim_matches('.').trim().to_string()
}

/// The ground model: one big quad (UVs inside the first tile of the default atlas).
fn ground_obj(half: f32) -> String {
    format!("# ground of a new Polystation project\no Ground\nvt 0.005 0.005\nvt 0.495 0.005\nvt 0.495 0.495\nvt 0.005 0.495\nv -{h} 0 {h}\nv {h} 0 {h}\nv {h} 0 -{h}\nv -{h} 0 -{h}\nf 1/1 2/2 3/3 4/4\n", h = half)
}

/// Creates `<dir>/<name>/<name>.polystation` with its ground model. Returns the project file.
pub fn create(template: &Template, name: &str, dir: &Path) -> Result<PathBuf, String> {
    let name = clean_name(name);
    if name.is_empty() {
        return Err("Give the project a name.".into());
    }
    let folder = dir.join(&name);
    if folder.join(format!("{name}.{}", crate::project::EXT)).exists() {
        return Err(format!("There is already a project called \"{name}\" in {}.", dir.display()));
    }
    std::fs::create_dir_all(&folder).map_err(|e| format!("could not create {}: {e}", folder.display()))?;
    let ground = folder.join("ground.obj");
    std::fs::write(&ground, ground_obj(template.ground)).map_err(|e| format!("{}: {e}", ground.display()))?;
    let mut edits = Edits::default();
    edits.base.entry("Ground".into()).or_default().color = Some(match template.id {
        "wetlands" => [0.3, 0.45, 0.25],
        "hall" => [0.3, 0.27, 0.25],
        "coins" => [0.3, 0.2, 0.45],
        _ => [0.42, 0.44, 0.5],
    });
    (template.build)(&mut edits);
    if template.id == "wetlands" {
        edits.base.entry("Ground".into()).or_default().deleted = true; // the terrain is the ground
    }
    let st = ProjectState {
        model: ground.to_string_lossy().into_owned(),
        atlas: None,
        imports: Vec::new(),
        build: Default::default(),
        camera: Some(CameraState { yaw: 0.0, pitch: -0.35, pos: [0.0, 9.0, 22.0], speed: 1.0 }),
        view: ViewState { mode: 0, affine: true, grid: true, cull: false },
        hidden: Vec::new(),
        selected: None,
        edits,
    };
    let file = folder.join(format!("{name}.{}", crate::project::EXT));
    crate::project::save(&file, &st)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_makes_a_loadable_project_with_a_player() {
        let dir = std::env::temp_dir().join("polystation_template_test");
        let _ = std::fs::remove_dir_all(&dir);
        for t in &TEMPLATES {
            let file = create(t, &format!("My {}", t.name), &dir).unwrap();
            let st = crate::project::load(&file).unwrap();
            assert!(st.edits.extras.iter().any(|e| e.prim == Prim::Player), "{} has no player", t.id);
            assert!(std::path::Path::new(&st.model).exists());
            let mesh = crate::obj::load(&st.model).unwrap();
            assert!(mesh.radius > 1.0, "{}: the ground model has a size", t.id);
            for s in st.edits.scripts.values() {
                crate::script::check(s).unwrap();
            }
            let names: Vec<&String> = st.edits.extras.iter().map(|e| &e.name).collect();
            for (i, n) in names.iter().enumerate() {
                assert!(!names[..i].contains(n), "{}: duplicate name {n}", t.id);
            }
        }
        // the same name twice is refused
        assert!(create(&TEMPLATES[0], "My Blank Room", &dir).is_err());
        assert!(create(&TEMPLATES[0], "  ", &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_become_safe_folder_names() {
        assert_eq!(clean_name("  a/b:c?  "), "a_b_c_");
        assert_eq!(clean_name("..."), "");
        assert_eq!(clean_name("My Game"), "My Game");
    }
}
