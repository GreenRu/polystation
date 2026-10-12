//! The script view: what a script tab shows in place of the 3D view. Two ways to write the same script:
//! a code editor, or a graph of blocks joined by wires (see the graph module). The graph is compiled to
//! code every time it changes, so the game only ever runs code.
use super::popup::MenuItem;
use super::render::{alpha, Color};
use super::style::*;
use super::widgets::{Clipboard, Field, Highlight};
use super::{inside, Rect};
use crate::graph::{self, Graph, ParamKind, PinKind, Ty};

// context menu commands (kept out of the range of the application's own)
pub const CMD_ADD: u16 = 5000;
pub const CMD_CHOICE: u16 = 5200;
pub const CMD_NODE_DELETE: u16 = 5300;
pub const CMD_NODE_DUPLICATE: u16 = 5301;
pub const CMD_DISCONNECT: u16 = 5302;
pub const CMD_PASTE_NAME: u16 = 5303;

pub fn is_menu_command(id: u16) -> bool {
    (CMD_ADD..5400).contains(&id)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Visual,
    Code,
}

/// What the view wants the UI around it to do.
pub enum ViewEvent {
    /// The code was edited.
    Code(String),
    /// The graph changed; the result of compiling it.
    Graph(Graph, Result<String, String>),
    /// "Convert to code": the graph is dropped, the generated code stays.
    ToCode,
    Popup(Vec<MenuItem>, (f32, f32)),
}

enum Edit {
    Value(u32, usize),
    Param(u32, usize),
}

enum Drag {
    None,
    Pan { start: (f32, f32), pan: (f32, f32) },
    Move { start: (f32, f32), origin: Vec<(u32, f32, f32)> },
    /// A wire being pulled from a pin: (block, pin, is an output), and where the mouse went down.
    Wire { from: (u32, usize, bool), start: (f32, f32) },
    CodeSelect,
    CodeScroll,
}

enum Pending {
    None,
    Choice(u32, usize),
    Node(u32),
    Pin(u32, usize, bool),
}

/// The add-a-block panel: a search box above a list of blocks.
struct Search {
    /// Top left of the panel in the window, and where the new block goes on the canvas.
    screen: (f32, f32),
    canvas: (f32, f32),
    field: Field,
    sel: usize,
    scroll: usize,
    /// A wire was pulled out of this pin and let go on empty canvas: (block, pin, is an output).
    from: Option<(u32, usize, bool)>,
}

const SEARCH_W: f32 = 300.0;
const SEARCH_HEAD: f32 = 42.0;
const SEARCH_ROW: f32 = 26.0;
const SEARCH_ROWS: usize = 12;

// design-space sizes of a block
const NODE_W: f32 = 214.0;
const HEADER_H: f32 = 26.0;
const ROW_H: f32 = 26.0;
const PARAM_H: f32 = 28.0;
const TOOLBAR_H: f32 = 36.0;

struct Layout {
    rect: Rect,
    header: Rect,
    /// Input pin positions and output pin positions (centres).
    ins: Vec<(f32, f32)>,
    outs: Vec<(f32, f32)>,
    /// The typed-in value boxes of unconnected data inputs.
    values: Vec<Option<Rect>>,
    params: Vec<Rect>,
    /// The eyedropper buttons of object parameters.
    droppers: Vec<Option<Rect>>,
}

pub struct ScriptView {
    pub object: String,
    pub mode: Mode,
    pub code: Field,
    pub graph: Option<Graph>,
    /// The compile status line: text, ok.
    pub status: (String, bool),
    pub events: Vec<ViewEvent>,
    pan: (f32, f32),
    zoom: f32,
    selected: Vec<u32>,
    drag: Drag,
    edit: Option<Edit>,
    edit_field: Field,
    /// Waiting for an object to be picked in the hierarchy: (block, parameter).
    pub eyedrop: Option<(u32, usize)>,
    confirm_visual: bool,
    pending: Pending,
    mouse: (f32, f32),
    /// The code box has the keyboard.
    pub code_focused: bool,
    code_rect: Rect,
    search: Option<Search>,
    /// Where the search panel was drawn last (for the mouse).
    search_panel: Rect,
}

fn category_color(cat: &str) -> Color {
    match cat {
        "Events" => rgb(0x9b3b3b),
        "Flow" => rgb(0xb0702a),
        "Actions" => rgb(0x2f6db3),
        "Values" => rgb(0x2f8a5a),
        "Math" => rgb(0x7a4fb0),
        "Mouse" => rgb(0xa8467a),
        _ => rgb(0x2a8a8f),
    }
}

use super::render::rgb;

/// The colour of a type of pin and of the wires that carry it (after the sockets of Blender's nodes).
fn ty_color(ty: Ty) -> Color {
    match ty {
        Ty::Exec => [0.95, 0.95, 0.95, 1.0],
        Ty::Number => rgb(0xa1a1a1),
        Ty::Bool => rgb(0xe0a3e8),
        Ty::Text => rgb(0x70b2ff),
        Ty::Any => rgb(0x4ec9b0),
    }
}

/// The type of a pin of a block of the graph.
fn pin_ty(g: &Graph, id: u32, out: bool, pin: usize) -> Ty {
    match g.node(id) {
        Some(n) if out => graph::out_ty(&n.kind, &n.params, pin),
        Some(n) => graph::in_ty(&n.kind, pin),
        None => Ty::Any,
    }
}

impl ScriptView {
    pub fn new(object: &str, code: &str, graph: Option<Graph>) -> ScriptView {
        let mut f = Field::new(true, false);
        f.highlight = Highlight::Script;
        f.gutter = true;
        f.edit.set_text(code);
        let mut v = ScriptView {
            object: object.to_string(),
            mode: if graph.is_some() { Mode::Visual } else { Mode::Code },
            code: f,
            graph,
            status: (String::new(), true),
            events: Vec::new(),
            pan: (30.0, 30.0),
            zoom: 1.0,
            selected: Vec::new(),
            drag: Drag::None,
            edit: None,
            edit_field: Field::new(false, false),
            eyedrop: None,
            confirm_visual: false,
            pending: Pending::None,
            mouse: (0.0, 0.0),
            code_focused: false,
            code_rect: [0.0; 4],
            search: None,
            search_panel: [0.0; 4],
        };
        v.refresh_status();
        v
    }

    /// New content from the application (opening, undo): the camera and selection stay.
    pub fn set_content(&mut self, code: &str, graph: Option<Graph>) {
        if self.code.edit.text() != code {
            self.code.edit.set_text(code);
        }
        let had = self.graph.is_some();
        self.graph = graph;
        if self.graph.is_some() != had {
            self.mode = if self.graph.is_some() { Mode::Visual } else { Mode::Code };
        }
        self.selected.retain(|id| self.graph.as_ref().is_some_and(|g| g.node(*id).is_some()));
        self.edit = None;
        self.refresh_status();
    }

    fn refresh_status(&mut self) {
        let src = self.code.edit.text();
        self.status = match &self.graph {
            Some(g) => match g.compile() {
                Ok(_) => ("visual script OK, it runs when you press Play".into(), true),
                Err(e) => (e, false),
            },
            None if src.trim().is_empty() => ("no script yet: write code, or start a visual script".into(), true),
            None => match crate::script::check(&src) {
                Ok(()) => ("script OK, it runs when you press Play".into(), true),
                Err(e) => (e, false),
            },
        };
    }

    pub fn text_focus(&self) -> bool {
        self.edit.is_some() || self.search.is_some() || (self.mode == Mode::Code && self.code_focused)
    }

    // ---------------------------------------------------------------- geometry
    fn z(&self, s: f32) -> f32 {
        s * self.zoom
    }

    fn parts(&self, rect: Rect, s: f32) -> (Rect, Rect) {
        let th = (TOOLBAR_H * s).round();
        ([rect[0], rect[1], rect[2], th], [rect[0], rect[1] + th, rect[2], (rect[3] - th).max(1.0)])
    }

    fn to_screen(&self, body: Rect, p: (f32, f32), s: f32) -> (f32, f32) {
        let z = self.z(s);
        (body[0] + self.pan.0 * s + p.0 * z, body[1] + self.pan.1 * s + p.1 * z)
    }

    fn to_canvas(&self, body: Rect, p: (f32, f32), s: f32) -> (f32, f32) {
        let z = self.z(s);
        ((p.0 - body[0] - self.pan.0 * s) / z, (p.1 - body[1] - self.pan.1 * s) / z)
    }

    fn layout(&self, g: &Graph, id: u32, body: Rect, s: f32) -> Option<Layout> {
        let n = g.node(id)?;
        let d = graph::def(&n.kind)?;
        let z = self.z(s);
        let (x, y) = self.to_screen(body, (n.x, n.y), s);
        let rows = d.ins.len().max(d.outs.len());
        let h = (HEADER_H + ROW_H * rows as f32 + PARAM_H * d.params.len() as f32 + 6.0) * z;
        let rect = [x, y, NODE_W * z, h];
        let header = [x, y, NODE_W * z, HEADER_H * z];
        let row_y = |i: usize| y + (HEADER_H + ROW_H * (i as f32 + 0.5)) * z;
        let ins: Vec<(f32, f32)> = (0..d.ins.len()).map(|i| (x, row_y(i))).collect();
        let outs: Vec<(f32, f32)> = (0..d.outs.len()).map(|i| (x + NODE_W * z, row_y(i))).collect();
        let values = (0..d.ins.len())
            .map(|i| {
                let wired = g.source_of(id, i).is_some();
                (d.ins[i].kind == PinKind::Data && !wired).then(|| [x + NODE_W * 0.34 * z, row_y(i) - (ROW_H * 0.5 - 3.0) * z, NODE_W * 0.34 * z, (ROW_H - 6.0) * z])
            })
            .collect();
        let py = y + (HEADER_H + ROW_H * rows as f32) * z;
        let mut params = Vec::new();
        let mut droppers = Vec::new();
        for (i, p) in d.params.iter().enumerate() {
            let top = py + PARAM_H * i as f32 * z + 3.0 * z;
            let is_obj = matches!(p.kind, ParamKind::Object);
            let w = if is_obj { NODE_W * 0.64 - 30.0 } else { NODE_W * 0.64 };
            params.push([x + NODE_W * 0.32 * z, top, w * z, (PARAM_H - 6.0) * z]);
            droppers.push(is_obj.then(|| [x + (NODE_W * 0.32 + w + 4.0) * z, top, 24.0 * z, (PARAM_H - 6.0) * z]));
        }
        Some(Layout { rect, header, ins, outs, values, params, droppers })
    }

    // ---------------------------------------------------------------- drawing
    pub fn draw(&mut self, c: &mut Ctx, rect: Rect, focused: bool, eyedrop_banner: bool) {
        let s = c.s;
        let (tool, body) = self.parts(rect, s);
        c.r.fill(rect[0], rect[1], rect[2], rect[3], c_bg());
        // toolbar
        c.r.fill(tool[0], tool[1], tool[2], tool[3], c_bar());
        c.r.fill(tool[0], tool[1] + tool[3] - 1.0, tool[2], 1.0, c_border_soft());
        let by = tool[1] + 5.0 * s;
        let bh = tool[3] - 10.0 * s;
        let hot = |r: Rect| inside(self.mouse, r);
        let visual = [tool[0] + 10.0 * s, by, 84.0 * s, bh];
        let code = [visual[0] + visual[2] + 4.0 * s, by, 70.0 * s, bh];
        c.button(visual, if self.confirm_visual { "Replace code?" } else { "Visual" }, hot(visual), false, true, self.confirm_visual, self.mode == Mode::Visual);
        c.button(code, "Code", hot(code), false, true, false, self.mode == Mode::Code);
        let mut x = code[0] + code[2] + 14.0 * s;
        if self.mode == Mode::Visual && self.graph.is_some() {
            let conv = [x, by, 124.0 * s, bh];
            c.button(conv, "Convert to code", hot(conv), false, true, false, false);
            x += conv[2] + 14.0 * s;
        }
        let color = if self.status.1 { c_good() } else { c_error() };
        let max_w = (rect[0] + rect[2] - x - 10.0 * s).max(10.0);
        let shown = c.f.ui.fit(&self.status.0, max_w);
        c.text_in(&c.f.ui, &shown, x, tool[1], tool[3], color);

        match self.mode {
            Mode::Code => {
                let mono = &c.f.mono;
                self.code_rect = body;
                self.code.draw(c, body, mono, focused && self.code_focused, true, c_code_bg());
            }
            Mode::Visual => match self.graph.take() {
                Some(g) => {
                    self.draw_graph(c, &g, body, focused);
                    self.graph = Some(g);
                    self.draw_search(c, body, focused);
                }
                None => {
                    c.r.fill(body[0], body[1], body[2], body[3], c_code_bg());
                    c.text_center(&c.f.ui, "No visual script yet. Press Visual to start one.", body[0], body[1], body[2], body[3], c_dim());
                }
            },
        }
        if eyedrop_banner {
            let w = 520.0 * s;
            let r = [body[0] + (body[2] - w) * 0.5, body[1] + 10.0 * s, w, 34.0 * s];
            c.r.rect(r[0], r[1], r[2], r[3], 8.0 * s, c_accent(), 0.0, c_accent());
            c.text_center(&c.f.bold, "Eyedropper: click an object in the Hierarchy to pick it  (Esc cancels)", r[0], r[1], r[2], r[3], [1.0; 4]);
        }
    }

    fn draw_graph(&mut self, c: &mut Ctx, g: &Graph, body: Rect, focused: bool) {
        let s = c.s;
        let z = self.z(s);
        c.r.fill(body[0], body[1], body[2], body[3], c_code_bg());
        c.r.push_clip(body[0], body[1], body[2], body[3]);
        // dotted grid that moves with the canvas
        let step = (32.0 * z).max(8.0);
        let (ox, oy) = (body[0] + (self.pan.0 * s).rem_euclid(step), body[1] + (self.pan.1 * s).rem_euclid(step));
        let (mut gx, mut gy) = (ox, oy);
        while gy < body[1] + body[3] {
            gx = ox;
            while gx < body[0] + body[2] {
                c.r.fill(gx.round(), gy.round(), 2.0, 2.0, alpha([1.0; 4], 0.07));
                gx += step;
            }
            gy += step;
        }
        let _ = gx;
        // wires
        for w in &g.wires {
            let (Some(a), Some(b)) = (self.layout(g, w.from, body, s), self.layout(g, w.to, body, s)) else { continue };
            let (Some(p0), Some(p1)) = (a.outs.get(w.from_pin), b.ins.get(w.to_pin)) else { continue };
            let (from_ty, to_ty) = (pin_ty(g, w.from, true, w.from_pin), pin_ty(g, w.to, false, w.to_pin));
            // the wire takes the colour of what it carries, and eases into the colour of the input it ends at
            let (a_col, b_col) = (ty_color(from_ty), ty_color(if to_ty == Ty::Any { from_ty } else { to_ty }));
            wire(c, *p0, *p1, a_col, b_col, if from_ty == Ty::Exec { 3.0 * z } else { 2.0 * z });
        }
        // the wire being pulled
        if let Drag::Wire { from: (id, pin, is_out), .. } = &self.drag {
            if let Some(l) = self.layout(g, *id, body, s) {
                let p = if *is_out { l.outs.get(*pin) } else { l.ins.get(*pin) };
                if let Some(&p) = p {
                    let ty = pin_ty(g, *id, *is_out, *pin);
                    let col = ty_color(ty);
                    let width = if ty == Ty::Exec { 3.0 * z } else { 2.0 * z };
                    if *is_out {
                        wire(c, p, self.mouse, col, col, width);
                    } else {
                        wire(c, self.mouse, p, col, col, width);
                    }
                }
            }
        }
        // blocks
        for n in &g.nodes {
            let (Some(d), Some(l)) = (graph::def(&n.kind), self.layout(g, n.id, body, s)) else { continue };
            if l.rect[0] > body[0] + body[2] || l.rect[1] > body[1] + body[3] || l.rect[0] + l.rect[2] < body[0] || l.rect[1] + l.rect[3] < body[1] {
                continue;
            }
            let selected = self.selected.contains(&n.id);
            c.r.shadow(l.rect[0], l.rect[1] + 3.0 * z, l.rect[2], l.rect[3], 6.0 * z, 10.0 * z, [0.0, 0.0, 0.0, 0.4]);
            c.r.rect(l.rect[0], l.rect[1], l.rect[2], l.rect[3], 7.0 * z, c_panel(), if selected { 2.0 } else { 1.0 }, if selected { c_accent_hot() } else { c_border() });
            c.r.push_clip(l.rect[0], l.rect[1], l.rect[2], l.rect[3]);
            c.r.fill(l.header[0], l.header[1], l.header[2], l.header[3], category_color(d.category));
            c.text_in(&c.f.bold, d.title, l.header[0] + 10.0 * z, l.header[1], l.header[3], [1.0; 4]);
            for (i, pin) in d.ins.iter().enumerate() {
                let p = l.ins[i];
                c.text_in(&c.f.ui, pin.name, p.0 + 14.0 * z, p.1 - 10.0 * z, 20.0 * z, c_muted());
                if let Some(Some(vr)) = l.values.get(i) {
                    let editing = matches!(self.edit, Some(Edit::Value(id, k)) if id == n.id && k == i);
                    self.draw_box(c, *vr, &n.values[i], editing, false, focused);
                }
            }
            for (i, pin) in d.outs.iter().enumerate() {
                let p = l.outs[i];
                let w = c.f.ui.measure(pin.name);
                c.text_in(&c.f.ui, pin.name, p.0 - w - 14.0 * z, p.1 - 10.0 * z, 20.0 * z, c_muted());
            }
            for (i, pd) in d.params.iter().enumerate() {
                let r = l.params[i];
                c.text_in(&c.f.ui, pd.name, l.rect[0] + 10.0 * z, r[1], r[3], c_dim());
                let editing = matches!(self.edit, Some(Edit::Param(id, k)) if id == n.id && k == i);
                let value = n.params.get(i).map_or("", |v| v.as_str());
                let shown = if matches!(pd.kind, ParamKind::Object) && value.trim().is_empty() { "self" } else { value };
                self.draw_box(c, r, shown, editing, matches!(pd.kind, ParamKind::Choice(_)), focused);
                if let Some(Some(dr)) = l.droppers.get(i) {
                    let active = self.eyedrop == Some((n.id, i));
                    c.r.rect(dr[0], dr[1], dr[2], dr[3], 5.0 * z, if active { c_accent() } else if inside(self.mouse, *dr) { c_btn_hot() } else { c_btn() }, 1.0, c_border_soft());
                    pipette(c, *dr, if active { [1.0; 4] } else { c_text() });
                }
            }
            c.r.pop_clip();
            // pins on top of the border
            for (i, pin) in d.ins.iter().enumerate() {
                pin_dot(c, l.ins[i], pin.kind, ty_color(graph::in_ty(&n.kind, i)), g.source_of(n.id, i).is_some(), 6.0 * z, inside(self.mouse, around(l.ins[i], 9.0 * z)));
            }
            for (i, pin) in d.outs.iter().enumerate() {
                let used = g.wires.iter().any(|w| w.from == n.id && w.from_pin == i);
                pin_dot(c, l.outs[i], pin.kind, ty_color(graph::out_ty(&n.kind, &n.params, i)), used, 6.0 * z, inside(self.mouse, around(l.outs[i], 9.0 * z)));
            }
        }
        c.r.pop_clip();
        c.text_in(&c.f.ui, "Right-click, then type: add a block.  Pull a wire into empty space for a block menu.", body[0] + 12.0 * s, body[1] + body[3] - 26.0 * s, 22.0 * s, c_dim());
    }

    fn draw_box(&mut self, c: &mut Ctx, r: Rect, text: &str, editing: bool, choice: bool, focused: bool) {
        let s = c.s;
        c.r.rect(r[0], r[1], r[2], r[3], 4.0 * s, c_field(), 1.0, if editing { c_accent() } else { c_border_soft() });
        if editing {
            let font = &c.f.ui;
            self.edit_field.draw(c, r, font, focused, true, c_field());
        } else {
            let shown = c.f.ui.fit(text, r[2] - 10.0 * s - if choice { 14.0 * s } else { 0.0 });
            c.text_in(&c.f.ui, &shown, r[0] + 6.0 * s, r[1], r[3], c_text());
            if choice {
                c.arrow(r[0] + r[2] - 10.0 * s, r[1] + r[3] * 0.5, 4.0 * s, true, c_muted());
            }
        }
    }

    // ---------------------------------------------------------------- hit tests
    fn pin_at(&self, g: &Graph, p: (f32, f32), body: Rect, s: f32) -> Option<(u32, usize, bool)> {
        let r = 10.0 * self.z(s);
        for n in g.nodes.iter().rev() {
            let Some(l) = self.layout(g, n.id, body, s) else { continue };
            for (i, q) in l.outs.iter().enumerate() {
                if inside(p, around(*q, r)) {
                    return Some((n.id, i, true));
                }
            }
            for (i, q) in l.ins.iter().enumerate() {
                if inside(p, around(*q, r)) {
                    return Some((n.id, i, false));
                }
            }
        }
        None
    }

    fn node_at(&self, g: &Graph, p: (f32, f32), body: Rect, s: f32) -> Option<u32> {
        g.nodes.iter().rev().find(|n| self.layout(g, n.id, body, s).is_some_and(|l| inside(p, l.rect))).map(|n| n.id)
    }

    // ---------------------------------------------------------------- mouse
    /// A click inside the view. Returns true when a text box (or the code) now has the keyboard.
    pub fn mouse_down(&mut self, p: (f32, f32), right: bool, rect: Rect, s: f32, shift: bool, f: &Fonts) -> bool {
        self.mouse = p;
        let (tool, body) = self.parts(rect, s);
        if self.search.is_some() {
            if inside(p, self.search_panel) {
                if let Some(k) = self.search_row_at(p, s) {
                    self.search_choose(k);
                }
                return true;
            }
            self.search = None; // a click elsewhere closes the panel (and does what it would have done)
        }
        self.finish_edit();
        self.eyedrop = None; // a click in the view ends the eyedropper (the pipette button starts it again below)
        if !right && inside(p, tool) {
            self.toolbar_click(p, tool, s);
            return false;
        }
        match self.mode {
            Mode::Code => {
                self.code_focused = true;
                let track = [body[0] + body[2] - 12.0 * s, body[1], 12.0 * s, body[3]];
                let (_, ch) = self.code.content_size(&f.mono, s);
                if !right && self.code.scroll.press(p, track, ch, body[3]) {
                    self.drag = Drag::CodeScroll;
                } else if !right {
                    self.code.mouse_down(p, body, &f.mono, s, shift);
                    self.drag = Drag::CodeSelect;
                }
                true
            }
            Mode::Visual => {
                self.code_focused = false;
                if self.graph.is_none() {
                    return false;
                }
                self.visual_down(p, right, body, s, shift, f)
            }
        }
    }

    fn toolbar_click(&mut self, p: (f32, f32), tool: Rect, s: f32) {
        let by = tool[1] + 5.0 * s;
        let bh = tool[3] - 10.0 * s;
        let visual = [tool[0] + 10.0 * s, by, 84.0 * s, bh];
        let code = [visual[0] + visual[2] + 4.0 * s, by, 70.0 * s, bh];
        let conv = [code[0] + code[2] + 14.0 * s, by, 124.0 * s, bh];
        if inside(p, visual) {
            if self.mode == Mode::Visual && self.graph.is_some() {
                return;
            }
            if self.graph.is_none() {
                let has_code = !self.code.edit.text().trim().is_empty();
                if has_code && !self.confirm_visual {
                    self.confirm_visual = true; // the code would be replaced: ask once
                    return;
                }
                self.confirm_visual = false;
                self.graph = Some(Graph::starter());
                self.mode = Mode::Visual;
                self.graph_changed();
            } else {
                self.mode = Mode::Visual;
            }
        } else if inside(p, code) {
            self.confirm_visual = false;
            self.mode = Mode::Code;
        } else if inside(p, conv) && self.mode == Mode::Visual && self.graph.is_some() {
            self.graph = None;
            self.mode = Mode::Code;
            self.refresh_status();
            self.events.push(ViewEvent::ToCode);
        }
    }

    fn visual_down(&mut self, p: (f32, f32), right: bool, body: Rect, s: f32, shift: bool, f: &Fonts) -> bool {
        let Some(g) = self.graph.take() else { return false };
        let z = self.z(s);
        let mut keyboard = false;
        // pins first: they sit on the border of a block
        if let Some((id, pin, out)) = self.pin_at(&g, p, body, s) {
            if right {
                self.pending = Pending::Pin(id, pin, out);
                self.events.push(ViewEvent::Popup(vec![MenuItem::new(CMD_DISCONNECT, "Disconnect")], p));
            } else if !out && g.source_of(id, pin).is_some() {
                // pick the wire up by its far end
                let (from, from_pin) = g.source_of(id, pin).unwrap_or((id, pin));
                self.graph = Some(g);
                if let Some(g) = self.graph.as_mut() {
                    g.disconnect_input(id, pin);
                }
                self.drag = Drag::Wire { from: (from, from_pin, true), start: p };
                self.graph_changed();
                return false;
            } else {
                self.drag = Drag::Wire { from: (id, pin, out), start: p };
            }
            self.graph = Some(g);
            return false;
        }
        if let Some(id) = self.node_at(&g, p, body, s) {
            let l = self.layout(&g, id, body, s);
            if let (Some(l), Some(n), false) = (l, g.node(id), right) {
                let d = graph::def(&n.kind);
                // typed-in values and parameters
                for (i, r) in l.values.iter().enumerate() {
                    if r.is_some_and(|r| inside(p, r)) {
                        self.begin_edit(Edit::Value(id, i), &n.values[i].clone(), r.unwrap_or_default(), f, s);
                        self.graph = Some(g);
                        return true;
                    }
                }
                if let Some(d) = d {
                    for (i, pd) in d.params.iter().enumerate() {
                        if let Some(Some(dr)) = l.droppers.get(i) {
                            if inside(p, *dr) {
                                self.eyedrop = Some((id, i));
                                self.graph = Some(g);
                                return false;
                            }
                        }
                        if inside(p, l.params[i]) {
                            match pd.kind {
                                ParamKind::Choice(options) => {
                                    let cur = n.params.get(i).cloned().unwrap_or_default();
                                    let items: Vec<MenuItem> = options.iter().enumerate().map(|(k, o)| MenuItem::new(CMD_CHOICE + k as u16, o).checked(*o == cur)).collect();
                                    self.pending = Pending::Choice(id, i);
                                    self.events.push(ViewEvent::Popup(items, (l.params[i][0], l.params[i][1] + l.params[i][3])));
                                    self.graph = Some(g);
                                    return false;
                                }
                                _ => {
                                    let v = n.params.get(i).cloned().unwrap_or_default();
                                    self.begin_edit(Edit::Param(id, i), &v, l.params[i], f, s);
                                    self.graph = Some(g);
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
            if right {
                self.pending = Pending::Node(id);
                if !self.selected.contains(&id) {
                    self.selected = vec![id];
                }
                self.events.push(ViewEvent::Popup(vec![MenuItem::new(CMD_NODE_DUPLICATE, "Duplicate"), MenuItem::new(CMD_NODE_DELETE, "Delete")], p));
            } else {
                if shift {
                    if let Some(k) = self.selected.iter().position(|&x| x == id) {
                        self.selected.remove(k);
                    } else {
                        self.selected.push(id);
                    }
                } else if !self.selected.contains(&id) {
                    self.selected = vec![id];
                }
                let origin = self.selected.iter().filter_map(|&i| g.node(i).map(|n| (i, n.x, n.y))).collect();
                self.drag = Drag::Move { start: p, origin };
            }
            self.graph = Some(g);
            return keyboard;
        }
        // empty canvas
        if right {
            // the add panel: whatever is typed next goes into its search box
            let at = self.to_canvas(body, p, s);
            self.open_search(p, at, None);
            keyboard = true;
        } else {
            self.selected.clear();
            self.drag = Drag::Pan { start: p, pan: self.pan };
        }
        let _ = z;
        self.graph = Some(g);
        keyboard
    }

    fn begin_edit(&mut self, target: Edit, text: &str, rect: Rect, f: &Fonts, s: f32) {
        self.edit_field.edit.set_text(text);
        self.edit_field.edit.select_all();
        self.edit_field.mouse_down((rect[0] + rect[2] - 4.0, rect[1] + 2.0), rect, &f.ui, s, false);
        self.edit_field.edit.select_all();
        self.edit = Some(target);
    }

    fn finish_edit(&mut self) {
        self.edit = None;
    }

    pub fn mouse_move(&mut self, p: (f32, f32), rect: Rect, s: f32, f: &Fonts) {
        self.mouse = p;
        let (_, body) = self.parts(rect, s);
        if self.search.is_some() && inside(p, self.search_panel) {
            if let Some(k) = self.search_row_at(p, s) {
                if let Some(sc) = self.search.as_mut() {
                    sc.sel = k; // the row under the mouse is the one Enter picks
                }
            }
        }
        match &self.drag {
            Drag::CodeSelect => self.code.mouse_drag(p, body, &f.mono, s),
            Drag::CodeScroll => {
                let track = [body[0] + body[2] - 12.0 * s, body[1], 12.0 * s, body[3]];
                let (_, ch) = self.code.content_size(&f.mono, s);
                self.code.scroll.drag_to(p.1, track, ch, body[3]);
            }
            Drag::Pan { start, pan } => {
                self.pan = (pan.0 + (p.0 - start.0) / s, pan.1 + (p.1 - start.1) / s);
            }
            Drag::Move { start, origin } => {
                let z = self.z(s);
                let (dx, dy) = ((p.0 - start.0) / z, (p.1 - start.1) / z);
                let origin = origin.clone();
                if let Some(g) = self.graph.as_mut() {
                    for (id, x, y) in origin {
                        if let Some(n) = g.node_mut(id) {
                            n.x = (x + dx).round();
                            n.y = (y + dy).round();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn mouse_up(&mut self, p: (f32, f32), rect: Rect, s: f32) {
        self.mouse = p;
        let (_, body) = self.parts(rect, s);
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        match drag {
            Drag::CodeSelect => self.code.selecting = false,
            Drag::CodeScroll => self.code.scroll.release(),
            Drag::Move { start, .. } => {
                if (p.0 - start.0).abs() + (p.1 - start.1).abs() > 2.0 {
                    self.graph_changed(); // moved: worth saving (the code does not change)
                }
            }
            Drag::Wire { from: (id, pin, out), start } => {
                let Some(g) = self.graph.take() else { return };
                let target = self.pin_at(&g, p, body, s);
                let over_block = self.node_at(&g, p, body, s).is_some();
                self.graph = Some(g);
                if let Some((tid, tpin, tout)) = target {
                    if tout != out {
                        let ok = if out { self.graph.as_mut().is_some_and(|g| g.connect(id, pin, tid, tpin)) } else { self.graph.as_mut().is_some_and(|g| g.connect(tid, tpin, id, pin)) };
                        if ok {
                            self.graph_changed();
                        }
                    }
                } else if !over_block && (p.0 - start.0).hypot(p.1 - start.1) > 10.0 * s {
                    // let go on empty canvas: like Blender, offer the blocks the wire can end at
                    let at = self.to_canvas(body, p, s);
                    self.open_search(p, at, Some((id, pin, out)));
                }
            }
            _ => {}
        }
    }

    pub fn wheel(&mut self, notches: f32, p: (f32, f32), rect: Rect, s: f32, f: &Fonts) {
        let (_, body) = self.parts(rect, s);
        match self.mode {
            Mode::Code => self.code.wheel(notches, body, &f.mono, s),
            Mode::Visual if self.search.is_some() && inside(p, self.search_panel) => {
                let n = self.search_results().len();
                if let Some(sc) = self.search.as_mut() {
                    let max = n.saturating_sub(SEARCH_ROWS);
                    sc.scroll = (sc.scroll as f32 - notches * 3.0).round().clamp(0.0, max as f32) as usize;
                }
            }
            Mode::Visual => {
                let before = self.to_canvas(body, p, s);
                self.zoom = (self.zoom * 1.1f32.powf(notches)).clamp(0.6, 1.5);
                // keep the point under the mouse where it was
                let z = self.z(s);
                self.pan = ((p.0 - body[0] - before.0 * z) / s, (p.1 - body[1] - before.1 * z) / s);
            }
        }
    }

    // ---------------------------------------------------------------- keyboard
    pub fn key(&mut self, vk: u32, ctrl: bool, shift: bool, clip: &mut dyn Clipboard) -> bool {
        if self.eyedrop.is_some() && vk == 0x1B {
            self.eyedrop = None;
            return true;
        }
        if self.search.is_some() {
            return self.search_key(vk, ctrl, shift, clip);
        }
        if self.edit.is_some() {
            match vk {
                0x1B | 0x0D | 0x09 => self.edit = None,
                _ => {
                    let (changed, _) = self.edit_field.key(vk, ctrl, shift, clip);
                    if changed {
                        self.apply_edit();
                    }
                }
            }
            return true;
        }
        match self.mode {
            Mode::Code => {
                if !self.code_focused {
                    return false;
                }
                let (changed, _) = self.code.key(vk, ctrl, shift, clip);
                if changed {
                    self.code_changed();
                }
                true
            }
            Mode::Visual => {
                if matches!(self.drag, Drag::Wire { .. }) && vk == 0x1B {
                    self.drag = Drag::None;
                    return true;
                }
                match vk {
                    0x2E | 0x08 if !self.selected.is_empty() => {
                        self.delete_selected();
                        true
                    }
                    0x44 if ctrl => {
                        self.duplicate_selected();
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    pub fn char_input(&mut self, ch: char) -> bool {
        if let Some(sc) = self.search.as_mut() {
            if sc.field.char_input(ch) {
                sc.sel = 0; // a new search starts at the best match
                sc.scroll = 0;
            }
            return true;
        }
        if self.edit.is_some() {
            if self.edit_field.char_input(ch) {
                self.apply_edit();
            }
            return true;
        }
        if self.mode == Mode::Code && self.code_focused {
            if self.code.char_input(ch) {
                self.code_changed();
            }
            return true;
        }
        false
    }

    // ---------------------------------------------------------------- changes
    fn code_changed(&mut self) {
        let text = self.code.edit.text();
        self.refresh_status();
        self.events.push(ViewEvent::Code(text));
    }

    fn graph_changed(&mut self) {
        let Some(g) = self.graph.clone() else { return };
        let compiled = g.compile();
        match &compiled {
            Ok(code) => {
                self.status = ("visual script OK, it runs when you press Play".into(), true);
                self.code.edit.set_text(code);
            }
            Err(e) => self.status = (e.clone(), false),
        }
        self.events.push(ViewEvent::Graph(g, compiled));
    }

    fn apply_edit(&mut self) {
        let text = self.edit_field.edit.text();
        let changed = match (&self.edit, self.graph.as_mut()) {
            (Some(Edit::Value(id, i)), Some(g)) => g.node_mut(*id).is_some_and(|n| {
                n.values[*i] = text;
                true
            }),
            (Some(Edit::Param(id, i)), Some(g)) => g.node_mut(*id).is_some_and(|n| {
                n.params[*i] = text;
                true
            }),
            _ => false,
        };
        if changed {
            self.graph_changed();
        }
    }

    /// The eyedropper picked an object of the scene.
    pub fn pick_object(&mut self, name: &str) {
        let Some((id, i)) = self.eyedrop.take() else { return };
        if let Some(n) = self.graph.as_mut().and_then(|g| g.node_mut(id)) {
            if i < n.params.len() {
                n.params[i] = name.to_string();
            }
        }
        self.graph_changed();
    }

    fn delete_selected(&mut self) {
        let ids = std::mem::take(&mut self.selected);
        if let Some(g) = self.graph.as_mut() {
            for id in ids {
                g.remove_node(id);
            }
        }
        self.graph_changed();
    }

    fn duplicate_selected(&mut self) {
        let ids = self.selected.clone();
        let mut copies = Vec::new();
        if let Some(g) = self.graph.as_mut() {
            for id in ids {
                if let Some(c) = g.duplicate(id) {
                    copies.push(c);
                }
            }
        }
        if !copies.is_empty() {
            self.selected = copies;
            self.graph_changed();
        }
    }

    /// A choice made in one of the view's popup menus.
    pub fn menu_choice(&mut self, id: u16) {
        let pending = std::mem::replace(&mut self.pending, Pending::None);
        match (id, pending) {
            (CMD_CHOICE..=5299, Pending::Choice(node, param)) => {
                let k = (id - CMD_CHOICE) as usize;
                if let Some(g) = self.graph.as_mut() {
                    let option = g.node(node).and_then(|n| graph::def(&n.kind)).and_then(|d| d.params.get(param)).and_then(|p| if let ParamKind::Choice(o) = p.kind { o.get(k).copied() } else { None });
                    if let (Some(o), Some(n)) = (option, g.node_mut(node)) {
                        n.params[param] = o.to_string();
                        self.graph_changed();
                    }
                }
            }
            (CMD_NODE_DELETE, Pending::Node(_)) => self.delete_selected(),
            (CMD_NODE_DUPLICATE, Pending::Node(_)) => self.duplicate_selected(),
            (CMD_DISCONNECT, Pending::Pin(node, pin, out)) => {
                if let Some(g) = self.graph.as_mut() {
                    if out {
                        g.disconnect_output(node, pin);
                    } else {
                        g.disconnect_input(node, pin);
                    }
                }
                self.graph_changed();
            }
            _ => {}
        }
    }

    pub fn cursor_hint(&self) -> bool {
        self.eyedrop.is_some()
    }
}

impl ScriptView {
    // ---------------------------------------------------------------- the add panel
    fn open_search(&mut self, screen: (f32, f32), canvas: (f32, f32), from: Option<(u32, usize, bool)>) {
        self.search = Some(Search { screen, canvas, field: Field::new(false, false), sel: 0, scroll: 0, from });
    }

    /// The blocks the panel lists for what is typed: (index into the block table, pin to wire to, or usize::MAX).
    fn search_results(&self) -> Vec<(usize, usize)> {
        let Some(sc) = &self.search else { return Vec::new() };
        let base: Vec<(usize, usize)> = match (sc.from, self.graph.as_ref()) {
            (Some((id, pin, out)), Some(g)) => match g.node(id) {
                // a wire was pulled out of a pin: only blocks it can end at
                Some(_) => graph::matching_blocks(pin_ty(g, id, out, pin), out),
                None => Vec::new(),
            },
            _ => (0..graph::DEFS.len()).map(|i| (i, usize::MAX)).collect(),
        };
        let q = sc.field.edit.text();
        let mut scored: Vec<(u8, usize, (usize, usize))> = base.into_iter().enumerate().filter_map(|(k, r)| graph::query_score(&graph::DEFS[r.0], &q).map(|s| (s, k, r))).collect();
        scored.sort_by_key(|&(s, k, _)| (s, k));
        scored.into_iter().map(|(_, _, r)| r).collect()
    }

    fn search_geometry(&self, body: Rect, s: f32, n: usize) -> (Rect, Rect) {
        let sc = self.search.as_ref();
        let screen = sc.map_or((body[0], body[1]), |sc| sc.screen);
        let (w, rows) = (SEARCH_W * s, n.clamp(1, SEARCH_ROWS));
        let h = (SEARCH_HEAD + SEARCH_ROW * rows as f32 + 8.0) * s;
        let x = screen.0.min(body[0] + body[2] - w - 8.0 * s).max(body[0] + 8.0 * s);
        let y = screen.1.min(body[1] + body[3] - h - 8.0 * s).max(body[1] + 8.0 * s);
        ([x, y, w, h], [x + 8.0 * s, y + 8.0 * s, w - 16.0 * s, 26.0 * s])
    }

    /// The result row under a point of the panel.
    fn search_row_at(&self, p: (f32, f32), s: f32) -> Option<usize> {
        let sc = self.search.as_ref()?;
        let top = self.search_panel[1] + SEARCH_HEAD * s;
        if p.1 < top {
            return None;
        }
        let k = sc.scroll + ((p.1 - top) / (SEARCH_ROW * s)) as usize;
        (k < self.search_results().len() && k < sc.scroll + SEARCH_ROWS).then_some(k)
    }

    fn search_key(&mut self, vk: u32, ctrl: bool, shift: bool, clip: &mut dyn Clipboard) -> bool {
        let n = self.search_results().len();
        let Some(sc) = self.search.as_mut() else { return false };
        match vk {
            0x1B => self.search = None,
            0x0D => {
                let k = sc.sel;
                self.search_choose(k);
            }
            0x26 => sc.sel = sc.sel.saturating_sub(1),
            0x28 => sc.sel = (sc.sel + 1).min(n.saturating_sub(1)),
            0x21 => sc.sel = sc.sel.saturating_sub(SEARCH_ROWS),
            0x22 => sc.sel = (sc.sel + SEARCH_ROWS).min(n.saturating_sub(1)),
            _ => {
                let (changed, _) = sc.field.key(vk, ctrl, shift, clip);
                if changed {
                    sc.sel = 0;
                    sc.scroll = 0;
                }
            }
        }
        true
    }

    /// Adds the block at `k` of the results where the panel was opened, joined to the wire it was opened for.
    fn search_choose(&mut self, k: usize) {
        let results = self.search_results();
        let Some(&(di, pin)) = results.get(k) else { return };
        let Some(sc) = self.search.take() else { return };
        let (mut x, mut y) = sc.canvas;
        if pin != usize::MAX {
            // the pin that gets the wire lands on the spot where the wire was let go
            y -= HEADER_H + ROW_H * (pin as f32 + 0.5);
            if matches!(sc.from, Some((_, _, false))) {
                x -= NODE_W;
            }
        }
        if let Some(g) = self.graph.as_mut() {
            if let Some(new) = g.add_node(graph::DEFS[di].id, x.round(), y.round()) {
                if let Some((id, fp, out)) = sc.from {
                    if out {
                        g.connect(id, fp, new, pin);
                    } else {
                        g.connect(new, pin, id, fp);
                    }
                }
                self.selected = vec![new];
                self.graph_changed();
            }
        }
    }

    fn draw_search(&mut self, c: &mut Ctx, body: Rect, focused: bool) {
        if self.search.is_none() {
            return;
        }
        let s = c.s;
        let results = self.search_results();
        let (panel, fr) = self.search_geometry(body, s, results.len());
        self.search_panel = panel;
        let Some(sc) = self.search.as_mut() else { return };
        sc.sel = sc.sel.min(results.len().saturating_sub(1));
        if sc.sel < sc.scroll {
            sc.scroll = sc.sel;
        } else if sc.sel >= sc.scroll + SEARCH_ROWS {
            sc.scroll = sc.sel + 1 - SEARCH_ROWS;
        }
        sc.scroll = sc.scroll.min(results.len().saturating_sub(SEARCH_ROWS));
        let (sel, scroll) = (sc.sel, sc.scroll);
        c.r.shadow(panel[0], panel[1] + 3.0 * s, panel[2], panel[3], 8.0 * s, 14.0 * s, [0.0, 0.0, 0.0, 0.5]);
        c.r.rect(panel[0], panel[1], panel[2], panel[3], 8.0 * s, c_panel(), 1.0, c_border());
        c.r.rect(fr[0], fr[1], fr[2], fr[3], 4.0 * s, c_field(), 1.0, c_accent());
        let font = &c.f.ui;
        if let Some(sc) = self.search.as_mut() {
            sc.field.draw(c, fr, font, focused, true, c_field());
            if sc.field.edit.text().is_empty() {
                let hint = if sc.from.is_some() { "Search the blocks this wire fits..." } else { "Search blocks..." };
                c.text_in(&c.f.ui, hint, fr[0] + 8.0 * s, fr[1], fr[3], c_dim());
            }
        }
        if results.is_empty() {
            c.text_in(&c.f.ui, "No matching blocks", panel[0] + 14.0 * s, panel[1] + SEARCH_HEAD * s, SEARCH_ROW * s, c_dim());
            return;
        }
        for (k, &(di, _)) in results.iter().enumerate().skip(scroll).take(SEARCH_ROWS) {
            let d = &graph::DEFS[di];
            let row = [panel[0] + 4.0 * s, panel[1] + (SEARCH_HEAD + SEARCH_ROW * (k - scroll) as f32) * s, panel[2] - 8.0 * s, SEARCH_ROW * s - 2.0];
            if k == sel {
                c.r.rect(row[0], row[1], row[2], row[3], 4.0 * s, alpha(c_accent(), 0.4), 0.0, [0.0; 4]);
            }
            c.r.fill(row[0] + 3.0 * s, row[1] + 4.0 * s, 4.0 * s, row[3] - 8.0 * s, category_color(d.category));
            c.text_in(&c.f.ui, d.title, row[0] + 16.0 * s, row[1], row[3], c_text());
            let w = c.f.ui.measure(d.category);
            c.text_in(&c.f.ui, d.category, row[0] + row[2] - w - 10.0 * s, row[1], row[3], c_dim());
        }
        if results.len() > SEARCH_ROWS {
            // a thin scroll bar
            let track = [panel[0] + panel[2] - 5.0 * s, panel[1] + SEARCH_HEAD * s, 3.0 * s, SEARCH_ROW * SEARCH_ROWS as f32 * s];
            let (th, ty) = (track[3] * SEARCH_ROWS as f32 / results.len() as f32, track[3] * scroll as f32 / results.len() as f32);
            c.r.fill(track[0], track[1] + ty, track[2], th, alpha([1.0; 4], 0.25));
        }
    }
}

fn around(p: (f32, f32), r: f32) -> Rect {
    [p.0 - r, p.1 - r, r * 2.0, r * 2.0]
}

/// A smooth wire between two points, fading from one colour to the other.
fn wire(c: &mut Ctx, a: (f32, f32), b: (f32, f32), col_a: Color, col_b: Color, width: f32) {
    let dx = ((b.0 - a.0).abs() * 0.5).max(40.0 * c.s);
    let (c1, c2) = ((a.0 + dx, a.1), (b.0 - dx, b.1));
    let mut prev = a;
    for i in 1..=24 {
        let t = i as f32 / 24.0;
        let u = 1.0 - t;
        let p = (u * u * u * a.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * b.0, u * u * u * a.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * b.1);
        let k = (i as f32 - 0.5) / 24.0;
        let color = [col_a[0] + (col_b[0] - col_a[0]) * k, col_a[1] + (col_b[1] - col_a[1]) * k, col_a[2] + (col_b[2] - col_a[2]) * k, 1.0];
        c.r.line(prev.0, prev.1, p.0, p.1, width.max(1.0), color);
        prev = p;
    }
}

fn pin_dot(c: &mut Ctx, p: (f32, f32), kind: PinKind, col: Color, connected: bool, r: f32, hot: bool) {
    let r = if hot { r * 1.35 } else { r };
    match kind {
        PinKind::Exec => {
            // a square-ish arrow head
            c.r.rect(p.0 - r, p.1 - r, r * 2.0, r * 2.0, 2.0, if connected { col } else { c_bg() }, 1.5, col);
        }
        PinKind::Data => {
            c.r.circle(p.0, p.1, r, if connected { col } else { c_bg() }, 1.5, col);
        }
    }
}

/// A tiny eyedropper icon.
fn pipette(c: &mut Ctx, r: Rect, color: Color) {
    let (cx, cy) = (r[0] + r[2] * 0.5, r[1] + r[3] * 0.5);
    let k = (r[3] * 0.32).max(3.0);
    c.r.line(cx - k, cy + k, cx + k * 0.3, cy - k * 0.3, 2.0, color);
    c.r.circle(cx + k * 0.75, cy - k * 0.75, k * 0.55, color, 0.0, color);
    c.r.fill((cx - k * 1.15).round(), (cy + k * 0.95).round(), 2.0, 2.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoClip;
    impl Clipboard for NoClip {
        fn get(&mut self) -> String {
            String::new()
        }
        fn set(&mut self, _: &str) {}
    }

    #[test]
    fn starting_a_visual_script_makes_the_starter_graph_and_an_event() {
        let mut v = ScriptView::new("Crate", "", None);
        assert_eq!(v.mode, Mode::Code);
        let tool = [0.0, 0.0, 800.0, 36.0];
        v.toolbar_click((20.0, 18.0), tool, 1.0);
        assert_eq!(v.mode, Mode::Visual);
        assert!(v.graph.is_some());
        assert!(matches!(v.events.last(), Some(ViewEvent::Graph(_, Ok(_)))));
    }

    #[test]
    fn replacing_code_needs_a_second_click() {
        let mut v = ScriptView::new("Crate", "on update {\n}\n", None);
        let tool = [0.0, 0.0, 800.0, 36.0];
        v.toolbar_click((20.0, 18.0), tool, 1.0);
        assert!(v.graph.is_none() && v.confirm_visual);
        v.toolbar_click((20.0, 18.0), tool, 1.0);
        assert!(v.graph.is_some());
    }

    fn type_into_search(v: &mut ScriptView, text: &str) {
        for ch in text.chars() {
            assert!(v.char_input(ch));
        }
    }

    #[test]
    fn typing_in_the_add_panel_searches_and_enter_adds_the_block() {
        let mut v = ScriptView::new("Crate", "", Some(Graph::starter()));
        v.open_search((50.0, 50.0), (300.0, 100.0), None);
        assert!(v.text_focus(), "the panel takes the keyboard right away");
        type_into_search(&mut v, "mouse set");
        let first = v.search_results()[0].0;
        assert_eq!(graph::DEFS[first].id, "mouse_set");
        assert!(v.key(0x0D, false, false, &mut NoClip));
        assert!(v.search.is_none());
        let n = v.graph.as_ref().unwrap().nodes.last().unwrap();
        assert_eq!((n.kind.as_str(), n.x, n.y), ("mouse_set", 300.0, 100.0));
        assert!(matches!(v.events.last(), Some(ViewEvent::Graph(_, Ok(_)))));
    }

    #[test]
    fn the_add_panel_lists_everything_and_escape_closes_it() {
        let mut v = ScriptView::new("Crate", "", Some(Graph::starter()));
        v.open_search((0.0, 0.0), (0.0, 0.0), None);
        assert_eq!(v.search_results().len(), graph::DEFS.len());
        type_into_search(&mut v, "zzzz");
        assert!(v.search_results().is_empty());
        assert!(v.key(0x1B, false, false, &mut NoClip) && v.search.is_none());
    }

    #[test]
    fn letting_go_of_a_wire_offers_blocks_that_fit_and_joins_it() {
        let mut g = Graph::starter();
        let key = g.add_node("mouse_x", 0.0, 300.0).unwrap();
        let mut v = ScriptView::new("Crate", "", Some(g));
        // a number wire pulled out of Mouse X: only blocks with a number (or any) input are offered
        v.open_search((400.0, 300.0), (400.0, 300.0), Some((key, 0, true)));
        let found: Vec<&str> = v.search_results().iter().map(|&(i, _)| graph::DEFS[i].id).collect();
        assert!(found.contains(&"mouse_set") && found.contains(&"math") && !found.contains(&"on_start") && !found.contains(&"text"));
        type_into_search(&mut v, "mouse set");
        v.key(0x0D, false, false, &mut NoClip);
        let g = v.graph.as_ref().unwrap();
        let new = g.nodes.last().unwrap();
        assert_eq!(new.kind, "mouse_set");
        assert_eq!(g.source_of(new.id, 1), Some((key, 0)), "the wire ends at the first number input");
        assert_eq!(new.x, 400.0, "the block hangs from the spot where the wire was let go");
    }

    #[test]
    fn a_wire_pulled_from_an_input_offers_blocks_with_an_output() {
        let mut g = Graph::starter();
        let set = g.add_node("mouse_set", 400.0, 200.0).unwrap();
        let mut v = ScriptView::new("Crate", "", Some(g));
        v.open_search((200.0, 200.0), (200.0, 200.0), Some((set, 1, false)));
        type_into_search(&mut v, "mouse y");
        v.key(0x0D, false, false, &mut NoClip);
        let g = v.graph.as_ref().unwrap();
        let new = g.nodes.last().unwrap();
        assert_eq!(new.kind, "mouse_y");
        assert_eq!(g.source_of(set, 1), Some((new.id, 0)));
        assert!(new.x < 200.0, "an output block sits to the left of the spot");
    }

    #[test]
    fn menu_choices_change_parameters() {
        let mut v = ScriptView::new("Crate", "", Some(Graph::starter()));
        let id = v.graph.as_mut().unwrap().add_node("set_prop", 300.0, 100.0).unwrap();
        v.pending = Pending::Choice(id, 1);
        v.menu_choice(CMD_CHOICE + 2);
        assert_eq!(v.graph.as_ref().unwrap().node(id).unwrap().params[1], "position.z");
    }

    #[test]
    fn the_eyedropper_fills_an_object_parameter() {
        let mut g = Graph::starter();
        let id = g.add_node("set_prop", 0.0, 0.0).unwrap();
        let mut v = ScriptView::new("Crate", "", Some(g));
        v.eyedrop = Some((id, 0));
        v.pick_object("Door");
        assert_eq!(v.graph.as_ref().unwrap().node(id).unwrap().params[0], "Door");
        assert!(v.eyedrop.is_none());
    }

    #[test]
    fn typing_in_code_mode_reports_the_code() {
        let mut v = ScriptView::new("Crate", "", None);
        v.code_focused = true;
        assert!(v.char_input('x'));
        assert!(matches!(v.events.last(), Some(ViewEvent::Code(t)) if t == "x"));
        let _ = NoClip;
    }
}
