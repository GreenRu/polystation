//! MCP (Model Context Protocol) support: lets an AI assistant build and test scenes in Polystation.
//!
//! Two halves:
//!  * The editor listens on `127.0.0.1:<mcp_port>` (47800 unless changed in Settings) for one-line JSON requests `{"tool":..., "args":{...}}`
//!    (`serve`). Each is answered on the editor's main thread (`App::mcp_call`), so a tool does exactly
//!    what the matching menu action or panel edit would do, undo included.
//!  * `polystation --mcp` is the MCP server an assistant launches (JSON-RPC over stdin / stdout,
//!    `run_stdio`). It forwards every tool call to the editor, starting the editor first if needed.
use crate::json::{self, J};
use crate::{hier, obj, prefabs, prims, scene, script, App, NodeRef};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 47800;
pub const NAME: &str = "Polystation";

/// A tool call waiting for the editor's main thread.
pub struct Request {
    pub tool: String,
    pub args: J,
    pub reply: Sender<J>,
}

// ------------------------------------------------------------------------------------------------
// editor side: the control port
// ------------------------------------------------------------------------------------------------

/// Starts listening (silently does nothing when another editor already owns the port).
pub fn serve(tx: Sender<Request>, port: u16) {
    let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) else { return };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || connection(stream, tx));
        }
    });
}

fn connection(stream: TcpStream, tx: Sender<Request>) {
    let Ok(mut out) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match json::parse(&line) {
            Ok(req) => {
                let (rtx, rrx) = mpsc::channel();
                let r = Request { tool: req.str("tool").unwrap_or("").to_string(), args: req.get("args").cloned().unwrap_or(J::obj()), reply: rtx };
                if tx.send(r).is_err() {
                    return;
                }
                rrx.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| err("the editor did not answer in time (is a dialog open?)"))
            }
            Err(e) => err(&format!("bad request: {e}")),
        };
        if writeln!(out, "{}", reply.to_string()).is_err() {
            return;
        }
    }
}

fn ok(result: J) -> J {
    J::obj().with("ok", true).with("result", result)
}

fn err(msg: &str) -> J {
    J::obj().with("ok", false).with("error", msg)
}

// ------------------------------------------------------------------------------------------------
// encoders for screenshots
// ------------------------------------------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// An uncompressed (stored deflate) 8-bit RGB PNG.
pub fn png_rgb(w: usize, h: usize, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for row in rgb.chunks(w * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        z.push(chunks.peek().is_none() as u8);
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

/// Turns the bottom-up RGBA read-back of the 3D view into a (downscaled) PNG reply.
pub fn screenshot_reply(w: usize, h: usize, rgba: &[u8]) -> J {
    let f = w.div_ceil(960).max(1);
    let (ow, oh) = (w / f, h / f);
    let mut rgb = Vec::with_capacity(ow * oh * 3);
    for y in 0..oh {
        for x in 0..ow {
            let mut sum = [0u32; 3];
            for dy in 0..f {
                let sy = h - 1 - (y * f + dy); // the read-back is upside down
                for dx in 0..f {
                    let i = (sy * w + x * f + dx) * 4;
                    for k in 0..3 {
                        sum[k] += rgba[i + k] as u32;
                    }
                }
            }
            let n = (f * f) as u32;
            rgb.extend(sum.iter().map(|&s| (s / n) as u8));
        }
    }
    ok(J::obj().with("width", ow).with("height", oh)).with("image", base64(&png_rgb(ow, oh, &rgb)))
}

// ------------------------------------------------------------------------------------------------
// editor side: what each tool does
// ------------------------------------------------------------------------------------------------

fn parse_color(args: &J, key: &str) -> Option<[f32; 3]> {
    if let Some(c) = args.vec3(key) {
        return Some(c.map(|v| v.clamp(0.0, 1.0)));
    }
    let h = args.str(key)?.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some([((n >> 16) & 255) as f32 / 255.0, ((n >> 8) & 255) as f32 / 255.0, (n & 255) as f32 / 255.0])
}

fn gui_json(e: &crate::gui::GuiElem) -> J {
    J::obj()
        .with("name", e.name.as_str())
        .with("kind", crate::gui::Kind::NAMES[e.kind.index()])
        .with("anchor", crate::gui::ANCHOR_NAMES[e.anchor as usize % 9])
        .with("x", e.x)
        .with("y", e.y)
        .with("width", e.w)
        .with("height", e.h)
        .with("text", e.text.as_str())
        .with("color", crate::gui::to_hex(e.color))
        .with("text_color", crate::gui::to_hex(e.text_color))
        .with("opacity", e.alpha)
        .with("text_size", e.text_size as usize)
        .with("value", e.value)
        .with("visible", e.visible)
}

/// Applies the fields of a tool call to a GUI element.
fn gui_apply(e: &mut crate::gui::GuiElem, a: &J) -> Result<(), String> {
    if let Some(v) = a.str("text") {
        e.text = v.to_string();
    }
    if let Some(v) = a.str("anchor") {
        e.anchor = crate::gui::ANCHOR_NAMES.iter().position(|n| n.eq_ignore_ascii_case(v)).ok_or("anchor must be one of: top left, top, top right, left, center, right, bottom left, bottom, bottom right")? as u8;
    }
    for (key, slot) in [("x", &mut e.x), ("y", &mut e.y)] {
        if let Some(v) = a.num(key) {
            *slot = v as f32;
        }
    }
    if let Some(v) = a.num("width") {
        e.w = (v as f32).max(1.0);
    }
    if let Some(v) = a.num("height") {
        e.h = (v as f32).max(1.0);
    }
    if a.get("color").is_some() {
        e.color = parse_color(a, "color").ok_or("color must be [r,g,b] (0..1) or \"#rrggbb\"")?;
    }
    if a.get("text_color").is_some() {
        e.text_color = parse_color(a, "text_color").ok_or("bad text_color")?;
    }
    if let Some(v) = a.num("opacity") {
        e.alpha = (v as f32).clamp(0.0, 1.0);
    }
    if let Some(v) = a.num("text_size") {
        e.text_size = (v as i32).clamp(1, 4) as u8;
    }
    if let Some(v) = a.num("value") {
        e.value = (v as f32).clamp(0.0, 1.0);
    }
    if let Some(v) = a.boolean("visible") {
        e.visible = v;
    }
    Ok(())
}

fn parse_collision(s: &str) -> Option<scene::Collision> {
    match s.to_ascii_lowercase().as_str() {
        "solid" => Some(scene::Collision::Solid),
        "passable" => Some(scene::Collision::Passable),
        "trigger" => Some(scene::Collision::Trigger),
        "water" => Some(scene::Collision::Water),
        _ => None,
    }
}

impl App {
    pub(crate) fn find_object(&self, name: &str) -> Option<usize> {
        self.mesh.nodes.iter().enumerate().position(|(i, n)| n.kind == obj::Kind::Object && n.name == name && !self.rt.deleted[i])
    }

    fn find_node_any(&self, name: &str) -> Option<usize> {
        self.mesh.nodes.iter().position(|n| n.name == name)
    }

    fn describe(&self, i: usize) -> J {
        let nb = self.base.nodes.len();
        let n = &self.mesh.nodes[i];
        let (source, kind) = match crate::node_ref(nb, i) {
            NodeRef::Extra(j) => ("added", self.edits.extras[j].prim.name().to_string()),
            _ => ("model", "Mesh".to_string()),
        };
        let x = self.rt.xf[i];
        J::obj()
            .with("name", n.name.as_str())
            .with("source", source)
            .with("kind", kind)
            .with("position", x.pos)
            .with("rotation", x.rot)
            .with("scale", x.scale)
            .with("color", self.rt.color[i].map_or(J::Null, J::from))
            .with("alpha", self.rt.alpha[i])
            .with("collision", scene::Collision::NAMES[self.edits.collision_of(&n.name) as usize])
            .with("light", match crate::node_ref(nb, i) {
                NodeRef::Extra(j) if self.edits.extras[j].prim == prims::Prim::Light => {
                    let l = &self.edits.extras[j].light;
                    J::obj().with("kind", crate::light::LightKind::NAMES[l.kind.index()]).with("intensity", l.intensity).with("range", l.range).with("angle", l.angle)
                }
                _ => J::Null,
            })
            .with("has_script", self.edits.scripts.contains_key(&n.name))
            .with("visible", self.ex.checked[i])
            .with("selected", self.ex.multi.contains(&i))
            .with("parent", n.parent.map_or(J::Null, |p| J::from(self.mesh.nodes[p].name.as_str())))
            .with("children", hier::children(&self.mesh.nodes, &self.edits.hier)[i].iter().filter(|&&k| !self.rt.deleted[k]).map(|&k| J::from(self.mesh.nodes[k].name.as_str())).collect::<Vec<_>>())
    }

    /// The explorer tree, nested.
    fn hierarchy_json(&self) -> J {
        let kids = hier::children(&self.mesh.nodes, &self.edits.hier);
        fn rec(app: &App, kids: &[Vec<usize>], i: usize) -> J {
            let n = &app.mesh.nodes[i];
            let kind = match n.kind {
                obj::Kind::Object => "object",
                obj::Kind::Collection => "collection",
                obj::Kind::Environment => "environment",
            };
            let sub: Vec<J> = kids[i].iter().filter(|&&k| !app.rt.deleted[k]).map(|&k| rec(app, kids, k)).collect();
            J::obj().with("name", n.name.as_str()).with("kind", kind).with("children", sub)
        }
        rec(self, &kids, 0)
    }

    fn require_editing(&self) -> Result<(), String> {
        if self.play.is_some() {
            Err("the game is playing: stop it first (play with action \"stop\")".into())
        } else {
            Ok(())
        }
    }

    fn mcp_apply_props(&mut self, name: &str, args: &J) -> Result<(), String> {
        let i = self.find_object(name).ok_or_else(|| format!("no object named \"{name}\" (use get_scene to list the objects)"))?;
        let mut x = self.rt.xf[i];
        if let Some(p) = args.vec3("position") {
            x.pos = p;
        }
        if let Some(r) = args.vec3("rotation") {
            x.rot = r;
        }
        if let Some(s) = args.vec3("scale") {
            x.scale = s;
        } else if let Some(k) = args.num("scale") {
            x.scale = [k as f32; 3];
        }
        if x != self.rt.xf[i] {
            self.edit_xform(i, |t| *t = x);
        }
        if args.get("color").is_some() {
            let c = parse_color(args, "color").ok_or("color must be [r,g,b] (0..1) or \"#rrggbb\"")?;
            self.edit_color(i, Some(c));
        }
        if let Some(a) = args.num("alpha") {
            self.edit_alpha(i, (a as f32).clamp(0.0, 1.0));
        }
        if let NodeRef::Extra(j) = crate::node_ref(self.base.nodes.len(), i) {
            if self.edits.extras[j].prim == prims::Prim::Light {
                let l = &mut self.edits.extras[j].light;
                if let Some(k) = args.str("light_kind") {
                    l.kind = match k.to_ascii_lowercase().as_str() {
                        "point" => crate::light::LightKind::Point,
                        "spot" => crate::light::LightKind::Spot,
                        "sun" => crate::light::LightKind::Sun,
                        _ => return Err("light_kind must be point, spot or sun".into()),
                    };
                }
                if let Some(v) = args.num("intensity") {
                    l.intensity = v as f32;
                }
                if let Some(v) = args.num("range") {
                    l.range = v as f32;
                }
                if let Some(v) = args.num("angle") {
                    l.angle = v as f32;
                }
                l.sanitize();
                self.objects_dirty = true;
            }
        }
        if let Some(c) = args.str("collision") {
            let c = parse_collision(c).ok_or("collision must be solid, passable, trigger or water")?;
            self.edits.set_collision(name, c);
            self.objects_dirty = true;
        }
        self.details_dirty = true;
        Ok(())
    }

    /// Answers one tool call. `None` means the reply comes later (screenshots, after the next frame).
    pub fn mcp_call(&mut self, rq: Request) {
        let Request { tool, args, reply } = rq;
        if tool == "screenshot" {
            self.shot_waiters.push(reply);
            return;
        }
        let r = match self.mcp_tool(&tool, &args) {
            Ok(v) => ok(v),
            Err(e) => err(&e),
        };
        let _ = reply.send(r);
    }

    fn mcp_tool(&mut self, tool: &str, args: &J) -> Result<J, String> {
        let nb = self.base.nodes.len();
        match tool {
            "get_info" => {
                let objects = (0..self.mesh.nodes.len()).filter(|&i| self.mesh.nodes[i].kind == obj::Kind::Object && !self.rt.deleted[i]).count();
                Ok(J::obj()
                    .with("engine", NAME)
                    .with("project", self.project_path.as_ref().map_or(J::Null, |p| J::from(p.to_string_lossy().into_owned())))
                    .with("model", self.model_path.as_str())
                    .with("objects", objects)
                    .with("playing", self.play.is_some())
                    .with("camera", J::obj().with("position", self.cam.pos).with("yaw_degrees", self.cam.yaw.to_degrees()).with("pitch_degrees", self.cam.pitch.to_degrees()))
                    .with("object_kinds", prims::ALL.iter().map(|p| J::from(p.name())).collect::<Vec<_>>())
                    .with("prefabs", prefabs::PREFABS.iter().map(|p| J::from(p.name)).collect::<Vec<_>>())
                    .with("collision_modes", scene::Collision::NAMES.iter().map(|&s| J::from(s)).collect::<Vec<_>>()))
            }
            "get_scene" => {
                let list: Vec<J> = (0..self.mesh.nodes.len()).filter(|&i| self.mesh.nodes[i].kind == obj::Kind::Object && !self.rt.deleted[i]).map(|i| self.describe(i)).collect();
                let env = &self.edits.env;
                Ok(J::obj().with("objects", list).with(
                    "environment",
                    J::obj().with("fog", ["off", "smooth", "dithered"][env.fog.min(2) as usize]).with("fog_color", env.fog_color).with("sky_color", env.sky.solid).with("ambient", env.ambient).with("ambient_strength", env.ambient_strength),
                ))
            }
            "add_object" => {
                self.require_editing()?;
                let kind = args.str("kind").ok_or("kind is required (see get_info for the list)")?;
                if let Some(i) = prefabs::PREFABS.iter().position(|p| p.name.eq_ignore_ascii_case(kind)) {
                    self.add_prefab(i);
                } else if let Some(p) = prims::Prim::from_name(kind) {
                    self.add_primitive(p);
                } else {
                    return Err(format!("unknown kind \"{kind}\" (see get_info: object_kinds and prefabs)"));
                }
                let j = self.edits.extras.len() - 1;
                if let Some(want) = args.str("name") {
                    let taken: Vec<String> = self.mesh.nodes.iter().map(|n| n.name.clone()).chain(self.edits.extras[..j].iter().map(|e| e.name.clone())).collect();
                    let new = scene::unique_name(want, |n| taken.iter().any(|t| t == n));
                    let old = std::mem::replace(&mut self.edits.extras[j].name, new.clone());
                    if let Some(s) = self.edits.scripts.remove(&old) {
                        self.edits.scripts.insert(new.clone(), s);
                    }
                    if let Some(c) = self.edits.collision.remove(&old) {
                        self.edits.collision.insert(new, c);
                    }
                    let hidden = self.hidden_by_name();
                    let n = self.edits.extras[j].name.clone();
                    self.rebuild(&hidden, Some(&n));
                }
                let name = self.edits.extras[j].name.clone();
                self.mcp_apply_props(&name, args)?;
                let i = self.find_object(&name).ok_or("the object was not created")?;
                Ok(self.describe(i))
            }
            "modify_object" => {
                self.require_editing()?;
                let name = args.str("name").ok_or("name is required")?.to_string();
                self.push_undo();
                self.mcp_apply_props(&name, args)?;
                let i = self.find_object(&name).ok_or("object not found")?;
                Ok(self.describe(i))
            }
            "delete_object" => {
                self.require_editing()?;
                let name = args.str("name").ok_or("name is required")?.to_string();
                let i = self.find_object(&name).ok_or_else(|| format!("no object named \"{name}\""))?;
                if matches!(crate::node_ref(nb, i), NodeRef::Env | NodeRef::PrimRoot) {
                    return Err("that node cannot be deleted".into());
                }
                self.select_names(&[name.clone()]);
                self.delete_selected();
                Ok(J::obj().with("deleted", name))
            }
            "compile" => {
                self.require_editing()?;
                let path = args.str("path").ok_or("path is required (the .exe to create)")?;
                let b = &mut self.build;
                if let Some(v) = args.str("title") {
                    b.name = v.to_string();
                }
                if let Some(v) = args.num("width") {
                    b.width = (v as u32).clamp(320, 7680);
                }
                if let Some(v) = args.num("height") {
                    b.height = (v as u32).clamp(200, 4320);
                }
                if let Some(v) = args.boolean("fullscreen") {
                    b.fullscreen = v;
                }
                if let Some(v) = args.boolean("vsync") {
                    b.vsync = v;
                }
                if let Some(v) = args.num("fps_limit") {
                    b.fps_limit = (v as u32).min(1000);
                }
                if let Some(v) = args.boolean("show_fps") {
                    b.show_fps = v;
                }
                let summary = self.compile_game(std::path::Path::new(path))?;
                Ok(J::obj().with("path", path).with("summary", summary))
            }
            "get_gui" => Ok(J::obj().with("mode", crate::gui::Mode::NAMES[self.edits.gui_mode.index()]).with("elements", self.edits.gui.iter().map(gui_json).collect::<Vec<_>>())),
            "set_gui_mode" => {
                self.require_editing()?;
                let m = args.str("mode").and_then(crate::gui::Mode::from_name).ok_or("mode must be look, cursor, toggle (Tab) or hold (Ctrl)")?;
                self.push_undo();
                self.edits.gui_mode = m;
                Ok(J::obj().with("mode", crate::gui::Mode::NAMES[m.index()]).with("meaning", m.describe()))
            }
            "add_gui" => {
                self.require_editing()?;
                let kind = args.str("kind").and_then(crate::gui::Kind::from_name).ok_or("kind must be text, panel, bar or button")?;
                let name = crate::gui::unique_name(&self.edits.gui, args.str("name").unwrap_or(crate::gui::Kind::NAMES[kind.index()]));
                let mut e = crate::gui::GuiElem::new(kind, &name);
                gui_apply(&mut e, args)?;
                self.push_undo();
                self.edits.gui.push(e.clone());
                Ok(gui_json(&e))
            }
            "modify_gui" => {
                self.require_editing()?;
                let name = args.str("name").ok_or("name is required")?;
                let i = self.edits.gui.iter().position(|e| e.name == name).ok_or_else(|| format!("no GUI element named \"{name}\" (see get_gui)"))?;
                let mut e = self.edits.gui[i].clone();
                gui_apply(&mut e, args)?;
                self.push_undo();
                self.edits.gui[i] = e.clone();
                Ok(gui_json(&e))
            }
            "delete_gui" => {
                self.require_editing()?;
                let name = args.str("name").ok_or("name is required")?;
                let i = self.edits.gui.iter().position(|e| e.name == name).ok_or_else(|| format!("no GUI element named \"{name}\""))?;
                self.push_undo();
                self.edits.gui.remove(i);
                Ok(J::obj().with("deleted", name))
            }
            "get_hierarchy" => Ok(self.hierarchy_json()),
            "set_parent" => {
                self.require_editing()?;
                let name = args.str("name").ok_or("name is required")?;
                let find = |n: &str| self.mesh.nodes.iter().enumerate().position(|(i, x)| x.name == n && x.kind != obj::Kind::Environment && !self.rt.deleted[i]).ok_or_else(|| format!("no node named \"{n}\" (see get_hierarchy)"));
                let node = find(name)?;
                let place = if let Some(b) = args.str("before") {
                    hier::Place::Before(find(b)?)
                } else if let Some(a) = args.str("after") {
                    hier::Place::After(find(a)?)
                } else if let Some(p) = args.str("parent") {
                    if p.is_empty() { hier::Place::Inside(0) } else { hier::Place::Inside(find(p)?) }
                } else {
                    return Err("pass parent (\"\" = top level), before or after".into());
                };
                if !self.reparent(node, place) {
                    return Err("that move is not possible (into itself, into its own child, or of a fixed node)".into());
                }
                let i = self.find_node_any(name).ok_or("node not found after the move")?;
                Ok(J::obj().with("name", name).with("parent", self.mesh.nodes[i].parent.map_or(J::Null, |p| J::from(self.mesh.nodes[p].name.as_str()))).with("hierarchy", self.hierarchy_json()))
            }
            "select_object" => {
                let name = args.str("name").ok_or("name is required")?.to_string();
                self.find_object(&name).ok_or_else(|| format!("no object named \"{name}\""))?;
                self.select_names(&[name.clone()]);
                Ok(J::obj().with("selected", name))
            }
            "set_script" => {
                self.require_editing()?;
                let name = args.str("object").ok_or("object is required")?.to_string();
                let src = args.str("source").ok_or("source is required (use an empty string to remove the script)")?.replace("\r\n", "\n");
                self.find_object(&name).ok_or_else(|| format!("no object named \"{name}\""))?;
                self.push_undo();
                let verdict = if src.trim().is_empty() {
                    self.edits.scripts.remove(&name);
                    Ok(())
                } else {
                    self.edits.scripts.insert(name.clone(), src.clone());
                    script::check(&src)
                };
                self.refresh_script_editor();
                Ok(J::obj().with("object", name).with("valid", verdict.is_ok()).with("error", verdict.err().map_or(J::Null, J::from)))
            }
            "get_script" => {
                let name = args.str("object").ok_or("object is required")?;
                let src = self.edits.scripts.get(name).cloned().unwrap_or_default();
                let verdict = if src.trim().is_empty() { Ok(()) } else { script::check(&src) };
                Ok(J::obj().with("object", name).with("source", src).with("valid", verdict.is_ok()).with("error", verdict.err().map_or(J::Null, J::from)))
            }
            "get_guide" => {
                let examples: Vec<J> = crate::guide::EXAMPLES.iter().map(|(t, s)| J::obj().with("title", *t).with("source", *s)).collect();
                Ok(J::obj().with("guide", crate::guide::GUIDE).with("examples", examples))
            }
            "set_environment" => {
                self.require_editing()?;
                self.push_undo();
                let e = &mut self.edits.env;
                if let Some(f) = args.str("fog") {
                    e.fog = match f.to_ascii_lowercase().as_str() {
                        "off" => 0,
                        "smooth" => 1,
                        "dithered" => 2,
                        _ => return Err("fog must be off, smooth or dithered".into()),
                    };
                }
                if args.get("fog_color").is_some() {
                    e.fog_color = parse_color(args, "fog_color").ok_or("bad fog_color")?;
                }
                if args.get("sky_color").is_some() {
                    e.sky.solid = parse_color(args, "sky_color").ok_or("bad sky_color")?;
                    e.sky.kind = scene::SkyKind::Solid;
                }
                if args.get("ambient").is_some() {
                    e.ambient = parse_color(args, "ambient").ok_or("bad ambient")?;
                }
                if let Some(k) = args.num("ambient_strength") {
                    e.ambient_strength = (k as f32).clamp(0.0, 8.0);
                }
                if let Some(v) = args.num("fog_start") {
                    e.fog_start = v as f32;
                }
                if let Some(v) = args.num("fog_end") {
                    e.fog_end = v as f32;
                }
                self.details_dirty = true;
                self.refresh();
                Ok(J::obj().with("ok", true))
            }
            "set_camera" => {
                if let Some(p) = args.vec3("position") {
                    self.cam.pos = p;
                }
                if let Some(y) = args.num("yaw_degrees") {
                    self.cam.yaw = (y as f32).to_radians();
                }
                if let Some(p) = args.num("pitch_degrees") {
                    self.cam.pitch = (p as f32).to_radians().clamp(-1.55, 1.55);
                }
                match args.str("frame") {
                    Some("all") => self.frame_all(),
                    Some("selected") => self.frame_selected(),
                    Some(name) => {
                        self.find_object(name).ok_or_else(|| format!("no object named \"{name}\""))?;
                        self.select_names(&[name.to_string()]);
                        self.frame_selected();
                    }
                    None => {}
                }
                Ok(J::obj().with("position", self.cam.pos).with("yaw_degrees", self.cam.yaw.to_degrees()).with("pitch_degrees", self.cam.pitch.to_degrees()))
            }
            "play" => {
                match args.str("action").unwrap_or("start") {
                    "start" if self.play.is_none() => self.start_play(),
                    "stop" if self.play.is_some() => self.stop_play(),
                    "start" | "stop" => {}
                    _ => return Err("action must be start or stop".into()),
                }
                Ok(J::obj().with("playing", self.play.is_some()))
            }
            "history" => {
                self.require_editing()?;
                match args.str("action").ok_or("action is required (undo or redo)")? {
                    "undo" => self.undo(),
                    "redo" => self.redo(),
                    _ => return Err("action must be undo or redo".into()),
                }
                Ok(J::obj().with("ok", true))
            }
            "open_project" => {
                let path = args.str("path").ok_or("path is required (.polystation, .blend, .obj ...)")?;
                if !std::path::Path::new(path).exists() {
                    return Err(format!("{path} does not exist"));
                }
                self.open_project_path(path.to_string());
                Ok(J::obj().with("loading", path))
            }
            "import_model" => {
                let path = args.str("path").ok_or("path is required")?;
                if !std::path::Path::new(path).exists() {
                    return Err(format!("{path} does not exist"));
                }
                self.import_model_path(path.to_string());
                Ok(J::obj().with("loading", path))
            }
            "save_project" => {
                let path = match (args.str("path"), &self.project_path) {
                    (Some(p), _) => std::path::PathBuf::from(p),
                    (None, Some(p)) => p.clone(),
                    (None, None) => return Err("this scene has no project file yet: pass a path ending in .polystation".into()),
                };
                crate::project::save(&path, &self.snapshot())?;
                self.project_path = Some(path.clone());
                self.window.set_title(&self.title());
                Ok(J::obj().with("saved", path.to_string_lossy().into_owned()))
            }
            _ => Err(format!("unknown tool \"{tool}\"")),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// MCP server (stdin / stdout)
// ------------------------------------------------------------------------------------------------

fn tool(name: &str, desc: &str, props: &[(&str, &str, &str)], required: &[&str]) -> J {
    let mut p = J::obj();
    for &(key, ty, d) in props {
        let schema = match ty {
            "vec3" => J::obj().with("type", "array").with("items", J::obj().with("type", "number")).with("minItems", 3).with("maxItems", 3),
            "color" => J::obj().with("type", "array").with("items", J::obj().with("type", "number")).with("minItems", 3).with("maxItems", 3),
            t => J::obj().with("type", t),
        };
        p = p.with(key, schema.with("description", d));
    }
    let schema = J::obj().with("type", "object").with("properties", p).with("required", required.iter().map(|&r| J::from(r)).collect::<Vec<_>>());
    J::obj().with("name", name).with("description", desc).with("inputSchema", schema)
}

fn gui_props(first: &'static str) -> Vec<(&'static str, &'static str, &'static str)> {
    let mut v = vec![if first == "kind" { ("kind", "string", "text | panel | bar | button") } else { ("name", "string", "element to change") }];
    if first == "kind" {
        v.push(("name", "string", "name of the new element"));
    }
    v.extend([
        ("text", "string", "text shown (text, bar, button)"),
        ("anchor", "string", "top left | top | top right | left | center | right | bottom left | bottom | bottom right"),
        ("x", "number", "distance from the anchor edge, design pixels"),
        ("y", "number", "distance from the anchor edge, design pixels"),
        ("width", "number", "design pixels"),
        ("height", "number", "design pixels"),
        ("color", "color", "fill color [r,g,b] or \"#rrggbb\""),
        ("text_color", "color", "text color"),
        ("opacity", "number", "0..1"),
        ("text_size", "number", "pixel size 1 .. 4"),
        ("value", "number", "bar fill, 0..1"),
        ("visible", "boolean", "shown or hidden"),
    ]);
    v
}

pub fn tool_list() -> Vec<J> {
    let obj_props: [(&str, &str, &str); 10] = [
        ("position", "vec3", "world position [x,y,z] (Y is up)"),
        ("rotation", "vec3", "Euler degrees [x,y,z]"),
        ("scale", "vec3", "size [x,y,z]"),
        ("color", "color", "[r,g,b] each 0..1"),
        ("alpha", "number", "opacity 0..1"),
        ("collision", "string", "solid | passable | trigger | water"),
        ("light_kind", "string", "lights only: point | spot | sun (the object color is the light color)"),
        ("intensity", "number", "lights only: brightness"),
        ("range", "number", "lights only: where a point / spot light fades out"),
        ("angle", "number", "lights only: spot light cone angle in degrees"),
    ];
    let mut add = vec![("kind", "string", "a shape (Cube, Sphere, Terrain, Tree, Light, Player ...) or a gameplay prefab (Coin, Door ...): see get_info"), ("name", "string", "name for the new object")];
    add.extend(obj_props);
    let mut modify = vec![("name", "string", "object to change")];
    modify.extend(obj_props);
    vec![
        tool("get_info", "Engine and project overview: the open project, camera, play state and every object kind / prefab that add_object accepts.", &[], &[]),
        tool("get_scene", "Lists every object (position, rotation, scale, color, collision, whether it has a script) plus fog / sky / ambient settings.", &[], &[]),
        tool("add_object", "Adds a shape, light, terrain, tree, player, particle system or gameplay prefab to the scene. Without a position it appears in front of the camera.", &add, &["kind"]),
        tool("modify_object", "Changes an object's transform, color, opacity or collision mode (only the fields you pass).", &modify, &["name"]),
        tool("compile", "Builds the stand-alone game (an .exe with the scene, texture atlas, images, sounds and scripts inside; it plays at start-up). The scene needs a Player object.", &[("path", "string", "where to write the .exe"), ("title", "string", "window title"), ("width", "number", "window width"), ("height", "number", "window height"), ("fullscreen", "boolean", "start in full screen"), ("vsync", "boolean", "wait for the monitor refresh"), ("fps_limit", "number", "frame rate cap, 0 = unlimited"), ("show_fps", "boolean", "frame counter in the game")], &["path"]),
        tool("get_gui", "The game's GUI elements (text, panels, bars, buttons) in drawing order.", &[], &[]),
        tool("set_gui_mode", "How the mouse works while the game plays: look (mouse turns the camera, no clicking), cursor (a pointer that clicks buttons), toggle (look, Tab switches the pointer) or hold (look, hold Ctrl for the pointer).", &[("mode", "string", "look | cursor | toggle | hold")], &["mode"]),
        tool("add_gui", "Adds a GUI element shown on screen while the game plays. Positions are in a 1280x720 design space, measured inward from the anchor edge.", &gui_props("kind"), &["kind"]),
        tool("modify_gui", "Changes a GUI element (only the fields you pass).", &gui_props("name"), &["name"]),
        tool("delete_gui", "Deletes a GUI element.", &[("name", "string", "element to delete")], &["name"]),
        tool("get_hierarchy", "The explorer tree: collections, objects and which object is inside which (nested).", &[], &[]),
        tool(
            "set_parent",
            "Rearranges the hierarchy like dragging in the explorer: make `name` a child of `parent` (\"\" = top level), or put it right before / after a sibling. Moving a parent moves its children, and deleting it deletes them.",
            &[("name", "string", "node to move"), ("parent", "string", "new parent (appended last); \"\" = top level"), ("before", "string", "place right above this node (same parent)"), ("after", "string", "place right below this node (same parent)")],
            &["name"],
        ),
        tool("delete_object", "Deletes an object (and everything inside it).", &[("name", "string", "object to delete")], &["name"]),
        tool("select_object", "Selects an object in the hierarchy.", &[("name", "string", "object to select")], &["name"]),
        tool("set_script", "Sets the GScript program of an object (an empty source removes it) and reports whether it compiles. Read get_guide first.", &[("object", "string", "object name"), ("source", "string", "GScript source")], &["object", "source"]),
        tool("get_script", "Reads an object's script and whether it compiles.", &[("object", "string", "object name")], &["object"]),
        tool("get_guide", "The GScript scripting guide and the example scripts.", &[], &[]),
        tool(
            "set_environment",
            "Sets fog, sky and ambient light.",
            &[("fog", "string", "off | smooth | dithered"), ("fog_color", "color", "[r,g,b] or \"#rrggbb\""), ("sky_color", "color", "solid sky color"), ("ambient", "color", "ambient light color"), ("ambient_strength", "number", "ambient multiplier, 0 - 8 (1 = as is)"), ("fog_start", "number", "in scene radii"), ("fog_end", "number", "in scene radii")],
            &[],
        ),
        tool(
            "set_camera",
            "Moves the editor camera. yaw 0 looks down -Z; pitch is up/down.",
            &[("position", "vec3", "camera position"), ("yaw_degrees", "number", "left/right"), ("pitch_degrees", "number", "up/down"), ("frame", "string", "\"all\", \"selected\" or an object name to frame")],
            &[],
        ),
        tool("screenshot", "Returns a PNG of the 3D view (what the editor camera sees, or the game while playing).", &[], &[]),
        tool("play", "Starts or stops play mode (needs a Player object in the scene).", &[("action", "string", "start | stop")], &[]),
        tool("history", "Undo or redo the last edit.", &[("action", "string", "undo | redo")], &["action"]),
        tool("open_project", "Opens a .polystation project or a model file (.blend, .obj, ...).", &[("path", "string", "absolute path")], &["path"]),
        tool("import_model", "Imports a model file into the scene.", &[("path", "string", "absolute path")], &["path"]),
        tool("save_project", "Saves the project (to path when given, else to the current file).", &[("path", "string", "target .polystation path")], &[]),
    ]
}

fn editor_stream() -> Result<TcpStream, String> {
    let port = crate::settings::Settings::load().mcp_port; // the editor reads the same file
    let addr = ("127.0.0.1", port);
    if let Ok(s) = TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(500)) {
        return Ok(s);
    }
    // not running: start it (detached, no console output so stdout stays clean)
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    if let Ok(p) = std::env::var("POLYSTATION_PROJECT") {
        cmd.arg(p);
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    cmd.spawn().map_err(|e| format!("could not start the editor: {e}"))?;
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(40) {
        std::thread::sleep(Duration::from_millis(400));
        if let Ok(s) = TcpStream::connect(addr) {
            return Ok(s);
        }
    }
    Err("the editor did not start".into())
}

fn call_editor(tool: &str, args: &J) -> Result<J, String> {
    let mut s = editor_stream()?;
    s.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let req = J::obj().with("tool", tool).with("args", args.clone());
    writeln!(s, "{}", req.to_string()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| format!("no answer from the editor: {e}"))?;
    json::parse(&line)
}

fn tool_result(tool: &str, args: &J) -> J {
    let content = |items: Vec<J>, is_error: bool| J::obj().with("content", items).with("isError", is_error);
    let text = |t: String| J::obj().with("type", "text").with("text", t);
    match call_editor(tool, args) {
        Err(e) => content(vec![text(e)], true),
        Ok(r) if r.boolean("ok") == Some(true) => {
            let mut items = vec![text(r.get("result").map_or("{}".to_string(), |v| v.to_string()))];
            if let Some(img) = r.str("image") {
                items.push(J::obj().with("type", "image").with("data", img).with("mimeType", "image/png"));
            }
            content(items, false)
        }
        Ok(r) => content(vec![text(r.str("error").unwrap_or("unknown error").to_string())], true),
    }
}

/// Answers one JSON-RPC message; `None` for notifications.
pub fn handle_rpc(msg: &J) -> Option<J> {
    let id = msg.get("id").cloned();
    let method = msg.str("method").unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(J::obj());
    let result = match method {
        "initialize" => J::obj()
            .with("protocolVersion", params.str("protocolVersion").unwrap_or("2024-11-05"))
            .with("capabilities", J::obj().with("tools", J::obj()))
            .with("serverInfo", J::obj().with("name", "polystation").with("version", env!("CARGO_PKG_VERSION")))
            .with("instructions", "Polystation is a PlayStation-1 style 3D engine and editor. Use get_info and get_scene first, get_guide before writing scripts, and screenshot to look at the result."),
        "ping" => J::obj(),
        "tools/list" => J::obj().with("tools", tool_list()),
        "tools/call" => tool_result(params.str("name").unwrap_or(""), &params.get("arguments").cloned().unwrap_or(J::obj())),
        _ => {
            let id = id?; // notifications get no answer
            return Some(J::obj().with("jsonrpc", "2.0").with("id", id).with("error", J::obj().with("code", -32601).with("message", format!("method not found: {method}"))));
        }
    };
    let id = id?;
    Some(J::obj().with("jsonrpc", "2.0").with("id", id).with("result", result))
}

/// `polystation --mcp`: the stdio server an assistant launches.
pub fn run_stdio() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match json::parse(&line) {
            Ok(m) => handle_rpc(&m),
            Err(e) => Some(J::obj().with("jsonrpc", "2.0").with("id", J::Null).with("error", J::obj().with("code", -32700).with("message", e))),
        };
        if let Some(r) = reply {
            if writeln!(out, "{}", r.to_string()).is_err() || out.flush().is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_base64_known_values() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn png_has_a_valid_structure() {
        let png = png_rgb(2, 2, &[255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9]);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 13, 10, 26, 10]);
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn protocol_basics() {
        let init = json::parse(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#).unwrap();
        let r = handle_rpc(&init).unwrap();
        assert_eq!(r.get("result").unwrap().str("protocolVersion"), Some("2025-03-26"));
        let note = json::parse(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap();
        assert!(handle_rpc(&note).is_none());
        let list = handle_rpc(&json::parse(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap()).unwrap();
        let tools = list.get("result").unwrap().get("tools").unwrap().as_arr().unwrap();
        assert!(tools.len() >= 15);
        assert!(tools.iter().all(|t| t.str("name").is_some() && t.get("inputSchema").is_some()));
    }

    #[test]
    fn colors_parse_from_hex_and_arrays() {
        let a = json::parse(r##"{"c":"#ff8000"}"##).unwrap();
        assert_eq!(parse_color(&a, "c"), Some([1.0, 128.0 / 255.0, 0.0]));
        let b = json::parse(r#"{"c":[0.5,2,0]}"#).unwrap();
        assert_eq!(parse_color(&b, "c"), Some([0.5, 1.0, 0.0]));
    }
}
