#![allow(dead_code)]
//! The engine's own user interface: drawn with OpenGL, driven by our own widgets and input handling.
//! Nothing here uses the controls Windows provides; only the window, a font and the clipboard come
//! from the system.
//!
//! The [`Ui`] holds the retained widgets of the main window (the panels, toolbar and tabs), the
//! hierarchy tree, the file list, the menu bar and the popups. `input.rs` turns mouse and keyboard
//! events into changes and application events; `draw` paints a frame.
pub mod dialogs;
pub mod gui_dialog;
pub mod hub;
pub mod settings_dialog;
pub mod edit;
pub mod font;
mod input;
pub mod list;
pub mod logo;
pub mod popup;
pub mod render;
pub mod script_view;
pub mod scroll;
pub mod style;
pub mod tree;
pub mod widgets;

use crate::win32::{self, ST_SCRIPT_INFO};
use list::List;
pub use popup::MenuItem;
use popup::Popup;
use render::{alpha, Renderer};
use scroll::VScroll;
use std::time::Instant;
use style::*;
use tree::Tree;
use widgets::{Field, Kind, Tone, Widget};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cursor {
    Arrow,
    IBeam,
    SizeWE,
    Hand,
}

#[derive(Clone, Copy, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// What an event turned out to be for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    /// The UI used it.
    Ui,
    /// It belongs to the 3D view (the application handles it).
    Viewport,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    None,
    /// A text box (index into the widgets).
    Field(usize),
    Tree,
    List,
    /// The script view (code, or a text box of a block).
    Script,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Active {
    None,
    /// A button held down.
    Button(usize),
    /// A number box being clicked or dragged.
    Num { idx: usize, start_x: f32, start_val: f32, dragging: bool },
    /// Selecting text with the mouse.
    Select(usize),
    /// A hierarchy row held down: dragging it (once `on`) rearranges the tree.
    TreeDrag { node: usize, start: (f32, f32), on: bool },
    ScrollInspector,
    ScrollTree,
    ScrollList,
    ScrollField(usize),
    /// A press inside the script view (drags go to it until the button comes up).
    ScriptView,
}

/// Window layout in pixels.
#[derive(Clone, Copy, Default)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
    pub mb: f32,
    pub tb: f32,
    pub hh: f32,
    pub tab: f32,
    pub tree_w: f32,
    pub panel_x: f32,
    pub body: f32,
    pub ex_h: f32,
    pub gap: f32,
    pub status_h: f32,
    /// A compiled game: the 3D view fills the window and nothing else is drawn.
    pub game: bool,
    /// Height of the strip of view tabs above the 3D view.
    pub vtab: f32,
}

pub type Rect = [f32; 4];

impl Layout {
    pub fn viewport(&self) -> Rect {
        if self.game {
            return [0.0, 0.0, self.w.max(1.0), self.h.max(1.0)];
        }
        [self.tree_w + self.gap, self.tb + self.vtab, (self.panel_x - self.tree_w - 2.0 * self.gap).max(1.0), (self.body - self.ex_h - self.vtab).max(1.0)]
    }

    /// The tabs above the 3D view: "Scene View" and one per open script.
    pub fn view_tabs(&self) -> Rect {
        [self.tree_w + self.gap, self.tb, (self.panel_x - self.tree_w - 2.0 * self.gap).max(1.0), self.vtab]
    }

    pub fn menubar(&self) -> Rect {
        [0.0, 0.0, self.w, self.mb]
    }

    pub fn toolbar(&self) -> Rect {
        [0.0, self.mb, self.w, self.tb - self.mb]
    }

    pub fn tree_header(&self) -> Rect {
        [0.0, self.tb, self.tree_w, self.hh]
    }

    pub fn tree(&self) -> Rect {
        [0.0, self.tb + self.hh, self.tree_w, (self.body - self.ex_h - self.hh).max(1.0)]
    }

    pub fn inspector_header(&self) -> Rect {
        [self.panel_x, self.tb, (self.w - self.panel_x).max(1.0), self.hh]
    }

    pub fn inspector(&self) -> Rect {
        [self.panel_x, self.tb + self.hh, (self.w - self.panel_x).max(1.0), (self.body - self.hh).max(1.0)]
    }

    pub fn tab_strip(&self) -> Rect {
        [0.0, self.tb + self.body - self.ex_h, (self.panel_x - self.gap).max(1.0), self.tab]
    }

    pub fn bottom(&self) -> Rect {
        let t = self.tab_strip();
        [0.0, t[1] + self.tab, t[2], (self.ex_h - self.tab).max(1.0)]
    }

    pub fn status(&self) -> Rect {
        [0.0, self.tb + self.body, self.w, self.h - self.tb - self.body]
    }
}

pub fn inside(p: (f32, f32), r: Rect) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

/// Where a widget group lives.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Toolbar,
    Inspector,
    Bottom,
}

pub fn region_of(group: u8) -> Region {
    match group {
        9 => Region::Toolbar,
        7 | 8 | 10 => Region::Bottom,
        _ => Region::Inspector,
    }
}

pub struct Ui {
    pub scale: f32,
    pub fonts: Option<Fonts>,
    pub widgets: Vec<Widget>,
    pub tree: Tree,
    pub list: List,
    bar: Vec<(String, Vec<MenuItem>)>,
    menu_state: Vec<(u16, bool)>,
    bar_hot: Option<usize>,
    popup: Option<Popup>,
    /// The open drop-down list: (widget index, hot item).
    combo_open: Option<(usize, Option<usize>)>,
    focus: Focus,
    hover: Option<usize>,
    active: Active,
    insp_scroll: VScroll,
    bottom_tab: u8,
    group_visible: [bool; 16],
    pub status: String,
    layout: Layout,
    cursor: Cursor,
    mouse: (f32, f32),
    pub drag_scale: f32,
    // what the application collects
    pub commands: Vec<u16>,
    pub edit_events: Vec<u16>,
    pub combo_events: Vec<u16>,
    pub tree_dbl_click: bool,
    pub explorer_dbl_click: bool,
    /// A hierarchy row / file explorer row was clicked (one-shot; also true when it was already selected).
    pub tree_click: bool,
    pub explorer_click: bool,
    pub tree_rclick: Option<(f32, f32)>,
    /// A hierarchy row was dropped: (dragged node, target node, where relative to the target). Nodes are tree indices.
    pub tree_drop: Option<(usize, usize, tree::DropAt)>,
    pub explorer_rclick: Option<(f32, f32)>,
    pub playing: bool,
    pub game: bool,
    pub start: Instant,
    /// Open script tabs (tab number n + 1) and which tab shows (0 = the scene).
    pub views: Vec<script_view::ScriptView>,
    pub active_view: usize,
    /// A script button of the hierarchy was clicked: the object's name.
    pub script_open: Option<String>,
    /// What the script views did that the application has to store.
    pub script_events: Vec<ScriptEvent>,
}

/// Something the application must store (or react to) after the user worked in a script tab.
pub enum ScriptEvent {
    Code { object: String, text: String },
    /// A graph changed; `code` is what it compiles to (an error leaves the old script in place).
    Graph { object: String, graph: crate::graph::Graph, code: Result<String, String> },
    /// The visual script was turned into plain code.
    ToCode { object: String },
}

impl Ui {
    pub fn new(scale: f32) -> Ui {
        let mut groups = [false; 16];
        for g in [0, 1, 7, 8, 9, 10, 15] {
            groups[g] = true;
        }
        Ui {
            scale,
            fonts: None,
            widgets: Vec::new(),
            tree: Tree::default(),
            list: List::new(&[("Name", 420.0), ("Type", 110.0), ("Size", 90.0)], true),
            bar: Vec::new(),
            menu_state: Vec::new(),
            bar_hot: None,
            popup: None,
            combo_open: None,
            focus: Focus::None,
            hover: None,
            active: Active::None,
            insp_scroll: VScroll::default(),
            bottom_tab: 0,
            group_visible: groups,
            status: String::new(),
            layout: Layout::default(),
            cursor: Cursor::Arrow,
            mouse: (0.0, 0.0),
            drag_scale: 1.0,
            commands: Vec::new(),
            edit_events: Vec::new(),
            combo_events: Vec::new(),
            tree_dbl_click: false,
            tree_click: false,
            explorer_click: false,
            explorer_dbl_click: false,
            tree_rclick: None,
            tree_drop: None,
            explorer_rclick: None,
            playing: false,
            game: false,
            views: Vec::new(),
            active_view: 0,
            script_open: None,
            script_events: Vec::new(),
            start: Instant::now(),
        }
    }

    pub fn px(&self, v: f32) -> f32 {
        (v * self.scale).round()
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    // ---------------------------------------------------------------- layout
    pub fn relayout(&mut self, w: f32, h: f32) {
        let s = self.scale;
        let mb = (28.0 * s).round();
        let tb = mb + (36.0 * s).round();
        let status_h = (22.0 * s).round();
        let body = (h - status_h - tb).max(1.0);
        let tree_w = (280.0 * s).round();
        let panel_w = (336.0 * s).round();
        self.layout = Layout {
            w,
            h,
            mb,
            tb,
            hh: (26.0 * s).round(),
            tab: (28.0 * s).round(),
            tree_w,
            panel_x: (w - panel_w).max(tree_w),
            body,
            ex_h: (240.0 * s).round().min((body / 2.0).floor()),
            gap: s.round().max(1.0),
            status_h,
            game: self.game,
            vtab: if self.game { 0.0 } else { (30.0 * s).round() },
        };
    }

    pub fn tab_visible(&self, group: u8) -> bool {
        match group {
            8 => self.bottom_tab == 0,
            7 => self.bottom_tab == 1,
            10 => self.bottom_tab == 2,
            _ => true,
        }
    }

    pub fn widget_visible(&self, w: &Widget) -> bool {
        self.group_visible.get(w.group as usize).copied().unwrap_or(true) && self.tab_visible(w.group)
    }

    /// Height of everything the inspector shows (for its scroll bar).
    fn inspector_content(&self) -> f32 {
        let s = self.scale;
        self.widgets.iter().filter(|w| region_of(w.group) == Region::Inspector && self.widget_visible(w)).map(|w| (w.y + w.h) as f32 * s).fold(0.0, f32::max) + 16.0 * s
    }

    /// A widget's rectangle in window pixels (None when it is not showing).
    pub fn widget_rect(&self, idx: usize) -> Option<Rect> {
        let w = self.widgets.get(idx)?;
        if !self.widget_visible(w) {
            return None;
        }
        let (s, l) = (self.scale, &self.layout);
        let p = |v: i32| (v as f32 * s).round();
        Some(match region_of(w.group) {
            Region::Toolbar => [p(w.x), l.mb + p(w.y), p(w.w), p(w.h)],
            Region::Inspector => {
                let v = l.inspector();
                [v[0] + p(w.x), v[1] + p(w.y) - self.insp_scroll.offset, p(w.w), p(w.h)]
            }
            Region::Bottom => {
                let b = l.bottom();
                let x = p(w.x);
                let rw = if w.w == 0 { (b[2] - x - p(4)).max(1.0) } else { p(w.w) };
                let rh = if w.h == 0 { (b[3] - p(w.y) - p(4)).max(1.0) } else { p(w.h) };
                [x, b[1] + p(w.y), rw, rh]
            }
        })
    }

    /// The region a widget is clipped to.
    fn clip_of(&self, group: u8) -> Rect {
        match region_of(group) {
            Region::Toolbar => self.layout.toolbar(),
            Region::Inspector => self.layout.inspector(),
            Region::Bottom => self.layout.bottom(),
        }
    }

    // ---------------------------------------------------------------- the application's view
    pub fn widget_index(&self, id: u16) -> Option<usize> {
        if id == 0 {
            return None;
        }
        self.widgets.iter().position(|w| w.id == id)
    }

    pub fn set_text(&mut self, id: u16, text: &str) {
        let Some(i) = self.widget_index(id) else { return };
        let w = &mut self.widgets[i];
        match &mut w.field {
            Some(f) => {
                if f.edit.text() != text.replace("\r\n", "\n") {
                    f.edit.set_text(text);
                    f.scroll_x = 0.0;
                }
            }
            None => w.text = text.to_string(),
        }
    }

    pub fn get_text(&self, id: u16) -> String {
        match self.widget_index(id).map(|i| &self.widgets[i]) {
            Some(w) => w.field.as_ref().map_or_else(|| w.text.clone(), |f| f.edit.text()),
            None => String::new(),
        }
    }

    pub fn set_enabled(&mut self, id: u16, on: bool) {
        if let Some(i) = self.widget_index(id) {
            self.widgets[i].enabled = on;
            if !on && self.focus == Focus::Field(i) {
                self.focus = Focus::None;
            }
        }
    }

    pub fn set_group_visible(&mut self, group: u8, visible: bool) {
        if let Some(g) = self.group_visible.get_mut(group as usize) {
            *g = visible;
        }
        if let Focus::Field(i) = self.focus {
            if !self.widgets.get(i).is_some_and(|w| self.widget_visible(w)) {
                self.focus = Focus::None;
            }
        }
    }

    pub fn combo_set(&mut self, id: u16, items: &[&str], selected: usize) {
        if let Some(i) = self.widget_index(id) {
            let w = &mut self.widgets[i];
            w.items = items.iter().map(|s| s.to_string()).collect();
            w.sel = selected.min(w.items.len().saturating_sub(1));
        }
    }

    pub fn combo_selected(&self, id: u16) -> usize {
        self.widget_index(id).map_or(0, |i| self.widgets[i].sel)
    }

    pub fn set_checked(&mut self, id: u16, on: bool) {
        if let Some(i) = self.widget_index(id) {
            self.widgets[i].on = on;
        }
    }

    pub fn is_checked(&self, id: u16) -> bool {
        self.widget_index(id).is_some_and(|i| self.widgets[i].on)
    }

    pub fn set_script_status(&mut self, text: &str, ok: bool) {
        if let Some(i) = self.widget_index(ST_SCRIPT_INFO) {
            self.widgets[i].tone = if ok { Tone::Good } else { Tone::Error };
            self.widgets[i].text = text.to_string();
        }
    }

    pub fn set_check(&mut self, id: u16, on: bool) {
        self.menu_state.retain(|(i, _)| *i != id);
        self.menu_state.push((id, on));
    }

    pub fn bottom_tab(&self) -> u8 {
        self.bottom_tab
    }

    pub fn show_bottom_tab(&mut self, t: u8) {
        self.bottom_tab = t.min(2);
        if let Focus::Field(i) = self.focus {
            if !self.widgets.get(i).is_some_and(|w| self.widget_visible(w)) {
                self.focus = Focus::None;
            }
        }
    }

    pub fn focus_field(&mut self, id: u16) {
        if let Some(i) = self.widget_index(id) {
            if self.widgets[i].field.is_some() {
                self.focus = Focus::Field(i);
            }
        }
    }

    /// True while a text box has the keyboard.
    pub fn text_focus(&self) -> bool {
        match self.focus {
            Focus::Field(_) => true,
            Focus::Script => self.active_view > 0 && self.views.get(self.active_view - 1).is_some_and(|v| v.text_focus()),
            _ => false,
        }
    }

    /// A script tab is showing instead of the 3D view.
    pub fn script_active(&self) -> bool {
        self.active_view > 0 && !self.game
    }

    /// Opens (or shows) the tab of an object's script.
    pub fn open_script_tab(&mut self, object: &str, code: &str, graph: Option<crate::graph::Graph>) {
        match self.views.iter().position(|v| v.object == object) {
            Some(i) => {
                self.views[i].set_content(code, graph);
                self.active_view = i + 1;
            }
            None => {
                self.views.push(script_view::ScriptView::new(object, code, graph));
                self.active_view = self.views.len();
            }
        }
        self.focus = Focus::Script;
    }

    /// New content for an open tab (after undo, a rename ...). Does nothing when the script is not open.
    pub fn update_script_tab(&mut self, object: &str, code: &str, graph: Option<crate::graph::Graph>) {
        if let Some(v) = self.views.iter_mut().find(|v| v.object == object) {
            v.set_content(code, graph);
        }
    }

    pub fn close_script_tab(&mut self, object: &str) {
        if let Some(i) = self.views.iter().position(|v| v.object == object) {
            self.close_view(i + 1);
        }
    }

    pub fn open_script_tabs(&self) -> Vec<String> {
        self.views.iter().map(|v| v.object.clone()).collect()
    }

    fn close_view(&mut self, tab: usize) {
        if tab == 0 || tab > self.views.len() {
            return;
        }
        self.views.remove(tab - 1);
        if self.active_view >= tab {
            self.active_view = self.active_view.saturating_sub(1).min(self.views.len());
        }
        if self.focus == Focus::Script {
            self.focus = Focus::None;
        }
    }

    pub fn tree_set_mark(&mut self, item: isize, on: bool) {
        if let Some(n) = self.tree.node_mut((item - 1).max(0) as usize) {
            n.mark = on;
        }
    }

    pub fn tree_set_badge(&mut self, item: isize, on: bool) {
        if let Some(n) = self.tree.node_mut((item - 1).max(0) as usize) {
            n.badge = on;
        }
    }

    /// Rectangles of the view tabs: (tab, close button) for tab 0 = the scene, then one per script.
    pub fn view_tab_rects(&self) -> Vec<(Rect, Option<Rect>)> {
        match self.fonts.as_ref() {
            Some(f) => self.view_tab_rects_with(f),
            None => Vec::new(),
        }
    }

    fn view_tab_rects_with(&self, f: &Fonts) -> Vec<(Rect, Option<Rect>)> {
        let s = self.scale;
        let strip = self.layout.view_tabs();
        let mut out = Vec::new();
        let mut x = strip[0] + 6.0 * s;
        let h = strip[3] - 4.0 * s;
        let w0 = (f.bold.measure("Scene View") + 40.0 * s).round();
        out.push(([x, strip[1] + 4.0 * s, w0, h], None));
        x += w0 + 2.0 * s;
        for v in &self.views {
            let w = (f.bold.measure(&v.object) + 66.0 * s).round();
            let r = [x, strip[1] + 4.0 * s, w, h];
            out.push((r, Some([r[0] + r[2] - 24.0 * s, r[1] + (r[3] - 16.0 * s) * 0.5, 16.0 * s, 16.0 * s])));
            x += w + 2.0 * s;
        }
        out
    }

    /// Sends what the script views did to where it belongs.
    fn flush_view_events(&mut self) {
        let Some(i) = self.active_view.checked_sub(1).filter(|&i| i < self.views.len()) else { return };
        let events = std::mem::take(&mut self.views[i].events);
        let object = self.views[i].object.clone();
        for e in events {
            match e {
                script_view::ViewEvent::Code(text) => self.script_events.push(ScriptEvent::Code { object: object.clone(), text }),
                script_view::ViewEvent::Graph(graph, code) => self.script_events.push(ScriptEvent::Graph { object: object.clone(), graph, code }),
                script_view::ViewEvent::ToCode => self.script_events.push(ScriptEvent::ToCode { object: object.clone() }),
                script_view::ViewEvent::Popup(items, at) => self.open_popup(&items, at.0, at.1),
            }
        }
    }

    pub fn set_bar(&mut self, bar: Vec<(String, Vec<MenuItem>)>) {
        self.bar = bar;
    }

    pub fn bar_rects(&self) -> Vec<Rect> {
        let f = self.fonts.as_ref();
        let s = self.scale;
        let mut x = (6.0 * s).round();
        self.bar
            .iter()
            .map(|(t, _)| {
                let w = f.map_or(40.0, |f| f.ui.measure(t)) + (22.0 * s).round();
                let r = [x, 0.0, w, self.layout.mb];
                x += w + (2.0 * s).round();
                r
            })
            .collect()
    }

    fn menu_items(&self, idx: usize) -> Vec<MenuItem> {
        fn walk(items: &mut [MenuItem], st: &[(u16, bool)]) {
            for it in items {
                if let Some(&(_, on)) = st.iter().find(|(id, _)| *id == it.id && it.id != 0) {
                    it.checked = on;
                }
                walk(&mut it.sub, st);
            }
        }
        let mut items = self.bar[idx].1.clone();
        walk(&mut items, &self.menu_state);
        items
    }

    /// Opens a context menu (or any popup list) at a window position.
    pub fn open_popup(&mut self, items: &[MenuItem], x: f32, y: f32) {
        let Some(f) = self.fonts.as_ref() else { return };
        self.popup = Some(Popup::open(items, x, y, (self.layout.w, self.layout.h), f, self.scale, None, false));
        self.combo_open = None;
    }

    pub fn open_bar(&mut self, idx: usize) {
        self.open_bar_menu(idx, false);
    }

    fn open_bar_menu(&mut self, idx: usize, by_press: bool) {
        let rects = self.bar_rects();
        let (Some(r), Some(f)) = (rects.get(idx), self.fonts.as_ref()) else { return };
        let items = self.menu_items(idx);
        self.popup = Some(Popup::open(&items, r[0], r[1] + r[3] + 1.0, (self.layout.w, self.layout.h), f, self.scale, Some(idx), by_press));
        self.combo_open = None;
    }

    pub fn popup_open(&self) -> bool {
        self.popup.is_some()
    }

    /// Where a widget is in the window, as (left, top, right, bottom).
    pub fn ctl_rect(&self, id: u16) -> Option<(f32, f32, f32, f32)> {
        let r = self.widget_rect(self.widget_index(id)?)?;
        Some((r[0], r[1], r[0] + r[2], r[1] + r[3]))
    }

    // ---------------------------------------------------------------- tree / list API (handles are node + 1)
    pub fn tree_clear(&mut self) {
        self.tree.clear();
        if self.focus == Focus::Tree {
            self.focus = Focus::None;
        }
    }

    pub fn tree_add(&mut self, parent: Option<isize>, text: &str, checked: bool) -> isize {
        let p = parent.filter(|&h| h > 0).map(|h| h as usize - 1);
        self.tree.add(p, text, checked) as isize + 1
    }

    pub fn tree_expand(&mut self, item: isize) {
        if let Some(n) = self.tree.node_mut((item - 1).max(0) as usize) {
            n.expanded = true;
        }
    }

    pub fn tree_checked(&self, item: isize) -> bool {
        self.tree.node((item - 1).max(0) as usize).is_some_and(|n| n.checked)
    }

    pub fn tree_set_checked(&mut self, item: isize, checked: bool) {
        if let Some(n) = self.tree.node_mut((item - 1).max(0) as usize) {
            n.checked = checked;
        }
    }

    pub fn tree_selected(&self) -> isize {
        self.tree.selected.map_or(0, |n| n as isize + 1)
    }

    pub fn tree_set_bold(&mut self, item: isize, bold: bool) {
        if let Some(n) = self.tree.node_mut((item - 1).max(0) as usize) {
            n.bold = bold;
        }
    }

    pub fn tree_select(&mut self, item: isize) {
        if item <= 0 {
            self.tree.selected = None;
        }
        if item > 0 && self.tree.node(item as usize - 1).is_some() {
            self.tree.selected = Some(item as usize - 1);
            self.tree.reveal(item as usize - 1);
            let l = self.layout.tree();
            let rh = (tree::ROW_H * self.scale).round();
            if let Some(row) = self.tree.rows().iter().position(|r| r.0 == item as usize - 1) {
                let content = self.tree.content_height(self.scale);
                self.tree.scroll.reveal(row as f32 * rh, (row + 1) as f32 * rh, content, l[3]);
            }
        }
    }

    pub fn explorer_clear(&mut self) {
        self.list.clear();
    }

    pub fn explorer_add(&mut self, name: &str, kind: &str, size: &str) {
        self.list.add(vec![name.to_string(), kind.to_string(), size.to_string()]);
    }

    pub fn explorer_select(&mut self, row: usize) {
        self.list.selected = Some(row);
    }

    pub fn explorer_selected(&self) -> Option<usize> {
        self.list.selected
    }

    // ---------------------------------------------------------------- drawing
    pub fn draw(&mut self, r: &mut Renderer, overlay: &mut dyn FnMut(&mut Ctx, Rect)) {
        let Some(fonts) = self.fonts.take() else { return };
        let l = self.layout;
        let s = self.scale;
        if self.game {
            r.begin(l.w as i32, l.h as i32);
            {
                let mut c = Ctx { r, f: &fonts, s };
                overlay(&mut c, l.viewport());
            }
            self.fonts = Some(fonts);
            r.end();
            return;
        }
        r.begin(l.w as i32, l.h as i32);
        {
            let mut c = Ctx { r, f: &fonts, s };
            self.draw_chrome(&mut c);
            self.draw_widgets(&mut c);
            let v = l.viewport();
            c.r.push_clip(v[0], v[1], v[2], v[3]);
            overlay(&mut c, v);
            c.r.pop_clip();
            // the strip of view tabs, and a script instead of the 3D view when its tab is open
            self.draw_view_tabs(&mut c);
            if let Some(i) = self.active_view.checked_sub(1).filter(|&i| i < self.views.len()) {
                let focused = self.focus == Focus::Script;
                let banner = self.views[i].eyedrop.is_some();
                self.views[i].draw(&mut c, v, focused, banner);
            }
        }
        r.end();
        // dialogs draw over this picture of the frame, so it is taken before menus and drop-downs go on top
        r.snapshot(l.w as i32, l.h as i32);
        r.begin(l.w as i32, l.h as i32);
        {
            let mut c = Ctx { r, f: &fonts, s };
            self.draw_overlays(&mut c);
        }
        self.fonts = Some(fonts);
        r.end();
    }

    fn draw_view_tabs(&self, c: &mut Ctx) {
        let s = c.s;
        let strip = self.layout.view_tabs();
        c.r.fill(strip[0], strip[1], strip[2], strip[3], c_bar());
        c.r.fill(strip[0], strip[1] + strip[3] - 1.0, strip[2], 1.0, c_border_soft());
        let rects = self.view_tab_rects_with(c.f);
        for (i, (r, close)) in rects.iter().enumerate() {
            let active = self.active_view == i;
            let hot = inside(self.mouse, *r);
            c.r.fill(r[0], r[1], r[2], r[3], if active { c_bg() } else if hot { c_btn() } else { c_bar() });
            if active {
                c.r.fill(r[0], r[1], r[2], 2.0 * s, c_accent());
            }
            let label = if i == 0 { "Scene View" } else { self.views[i - 1].object.as_str() };
            let color = if active { c_text() } else { c_muted() };
            if i == 0 {
                c.text_center(&c.f.bold, label, r[0], r[1], r[2], r[3], color);
            } else {
                c.text_in(&c.f.bold, "{ }", r[0] + 10.0 * s, r[1], r[3], c_accent_hot());
                c.text_in(&c.f.bold, label, r[0] + 40.0 * s, r[1], r[3], color);
            }
            if let Some(cr) = close {
                let over = inside(self.mouse, *cr);
                if over {
                    c.r.rect(cr[0], cr[1], cr[2], cr[3], 4.0 * s, c_btn_hot(), 0.0, c_btn_hot());
                }
                let (cx, cy, k) = (cr[0] + cr[2] * 0.5, cr[1] + cr[3] * 0.5, 4.0 * s);
                c.r.line(cx - k, cy - k, cx + k, cy + k, 1.6, if over { c_text() } else { c_muted() });
                c.r.line(cx - k, cy + k, cx + k, cy - k, 1.6, if over { c_text() } else { c_muted() });
            }
        }
    }

    fn draw_chrome(&mut self, c: &mut Ctx) {
        let l = self.layout;
        let s = c.s;
        let v = l.viewport();
        // background around the 3D view
        c.r.fill(0.0, 0.0, l.w, v[1], c_bg());
        c.r.fill(0.0, v[1], v[0], l.h - v[1], c_bg());
        c.r.fill(v[0] + v[2], v[1], l.w - v[0] - v[2], l.h - v[1], c_bg());
        c.r.fill(v[0], v[1] + v[3], v[2], l.h - v[1] - v[3], c_bg());
        // menu bar
        c.r.fill(0.0, 0.0, l.w, l.mb, c_bar());
        let rects = self.bar_rects();
        for (i, (title, _)) in self.bar.iter().enumerate() {
            let rr = rects[i];
            let open = self.popup.as_ref().is_some_and(|p| p.from_bar == Some(i));
            if open || self.bar_hot == Some(i) {
                c.r.rect(rr[0], rr[1] + 3.0 * s, rr[2], rr[3] - 6.0 * s, 7.0 * s, if open { c_btn_hot() } else { c_btn() }, 0.0, c_btn());
            }
            c.text_center(&c.f.ui, title, rr[0], rr[1], rr[2], rr[3], c_text());
        }
        // toolbar strip
        c.r.fill(0.0, l.mb, l.w, 1.0, c_border_soft());
        c.r.fill(0.0, l.tb - 1.0, l.w, 1.0, c_border_soft());
        // panel title strips
        for (r, t) in [(l.tree_header(), "HIERARCHY"), (l.inspector_header(), "INSPECTOR")] {
            c.r.fill(r[0], r[1], r[2], r[3], c_bar());
            c.r.fill(r[0], r[1] + r[3] - 1.0, r[2], 1.0, c_border_soft());
            c.text_in(&c.f.bold, t, r[0] + 10.0 * s, r[1], r[3], c_muted());
        }
        // dividers
        c.r.fill(l.tree_w, l.tb, l.gap, l.body, c_border_soft());
        c.r.fill(l.panel_x - l.gap, l.tb, l.gap, l.body, c_border_soft());
        // bottom tabs
        let t = l.tab_strip();
        c.r.fill(t[0], t[1], t[2], t[3], c_bar());
        c.r.fill(t[0], t[1] + t[3] - 1.0, t[2], 1.0, c_border_soft());
        for (i, title) in ["PROJECT", "SCRIPT", "GUIDE"].iter().enumerate() {
            if i == 1 {
                continue; // scripts have their own tabs above the scene now
            }
            let tr = self.tab_rect(i);
            if self.bottom_tab as usize == i {
                c.r.fill(tr[0], tr[1], tr[2], tr[3], c_bg());
                c.r.fill(tr[0], tr[1], tr[2], 2.0 * s, c_accent());
            }
            c.text_center(&c.f.bold, title, tr[0], tr[1], tr[2], tr[3], if self.bottom_tab as usize == i { c_text() } else { c_muted() });
        }
        // bottom area and inspector background
        let b = l.bottom();
        c.r.fill(b[0], b[1], b[2], b[3], c_bg());
        let ins = l.inspector();
        c.r.fill(ins[0], ins[1], ins[2], ins[3], c_bg());
        // status bar
        let st = l.status();
        c.r.fill(st[0], st[1], st[2], st[3], c_bar());
        c.r.fill(st[0], st[1], st[2], 1.0, c_border_soft());
        c.text_in(&c.f.ui, &self.status, 12.0 * s, st[1], st[3], c_muted());
        // the hierarchy
        let focused = self.focus == Focus::Tree;
        self.tree.draw(c, l.tree(), focused);
    }

    pub fn tab_rect(&self, i: usize) -> Rect {
        let t = self.layout.tab_strip();
        let s = self.scale;
        let slot = if i == 2 { 1 } else { i };
        [(8.0 * s + slot as f32 * 100.0 * s).round(), t[1] + (2.0 * s).round(), (96.0 * s).round(), t[3] - (3.0 * s).round()]
    }

    fn draw_widgets(&mut self, c: &mut Ctx) {
        let l = self.layout;
        let s = c.s;
        // inspector scroll range
        let content = self.inspector_content();
        let ins = l.inspector();
        self.insp_scroll.clamp(content, ins[3]);
        let mut order: Vec<usize> = (0..self.widgets.len()).collect();
        order.sort_by_key(|&i| matches!(self.widgets[i].kind, Kind::Combo)); // combos last (their list is an overlay anyway)
        for i in order {
            let Some(rect) = self.widget_rect(i) else { continue };
            let clip = self.clip_of(self.widgets[i].group);
            if rect[1] + rect[3] < clip[1] || rect[1] > clip[1] + clip[3] {
                continue;
            }
            c.r.push_clip(clip[0], clip[1], clip[2], clip[3]);
            self.draw_widget(c, i, rect);
            c.r.pop_clip();
        }
        // the file list
        if self.bottom_tab == 0 {
            if let Some(i) = self.widgets.iter().position(|w| w.id == win32::LV_EXPLORER) {
                if let Some(rect) = self.widget_rect(i) {
                    let b = l.bottom();
                    c.r.push_clip(b[0], b[1], b[2], b[3]);
                    let focused = self.focus == Focus::List;
                    self.list.draw(c, rect, focused);
                    c.r.pop_clip();
                }
            }
        }
        // inspector scroll bar
        let track = [ins[0] + ins[2] - 12.0 * s, ins[1], 12.0 * s, ins[3]];
        if let Some(t) = self.insp_scroll.thumb(track, content, ins[3]) {
            c.scrollbar(t, self.insp_scroll.dragging() || (self.mouse.0 >= track[0] && inside(self.mouse, ins)));
        }
    }

    fn draw_widget(&mut self, c: &mut Ctx, i: usize, rect: Rect) {
        let hot = self.hover == Some(i);
        let down = matches!(self.active, Active::Button(b) if b == i);
        let focused = self.focus == Focus::Field(i);
        let mouse = self.mouse;
        let open_combo = self.combo_open.is_some_and(|(o, _)| o == i);
        let fonts = c.f; // a plain reference copy: lets the fonts be used while `c` draws
        let w = &mut self.widgets[i];
        match w.kind {
            Kind::Static | Kind::Section => widgets::draw_static(c, rect, w),
            Kind::Button => {
                let pressed = down && inside(mouse, rect);
                c.button(rect, &w.text, hot, pressed, w.enabled, w.accent, w.toggle && w.on);
            }
            Kind::Combo => widgets::draw_combo(c, rect, w, hot, open_combo),
            Kind::Edit => {
                let enabled = w.enabled;
                let Some(f) = w.field.as_mut() else { return };
                if f.multi {
                    f.draw(c, rect, &fonts.mono, focused, enabled, c_code_bg());
                } else {
                    f.draw(c, rect, &fonts.ui, focused, enabled, c_field());
                }
            }
        }
    }

    fn draw_overlays(&mut self, c: &mut Ctx) {
        let s = c.s;
        // drop-down list
        if let Some((wi, hot)) = self.combo_open {
            if let Some(rect) = self.widget_rect(wi) {
                let list = self.combo_list_rect(wi, rect);
                let w = &self.widgets[wi];
                c.r.shadow(list[0] + 1.0, list[1] + 5.0 * s, list[2], list[3], 8.0 * s, 16.0 * s, [0.0, 0.0, 0.0, 0.5]);
                c.r.rect(list[0], list[1], list[2], list[3], 8.0 * s, c_menu_bg(), 1.0, c_border());
                let ih = (26.0 * s).round();
                for (k, it) in w.items.iter().enumerate() {
                    let iy = list[1] + 4.0 * s + k as f32 * ih;
                    if hot == Some(k) {
                        c.r.rect(list[0] + 4.0 * s, iy, list[2] - 8.0 * s, ih, 5.0 * s, c_menu_hot(), 0.0, c_menu_hot());
                    }
                    if k == w.sel {
                        c.check_mark(list[0] + 8.0 * s, iy + (ih - 12.0 * s) * 0.5, 12.0 * s, c_accent());
                    }
                    c.text_in(&c.f.ui, it, list[0] + 28.0 * s, iy, ih, c_text());
                }
            }
        }
        if let Some(p) = &self.popup {
            p.draw(c);
        }
        let _ = alpha;
    }

    /// The rectangle of an open drop-down list.
    pub fn combo_list_rect(&self, wi: usize, rect: Rect) -> Rect {
        let s = self.scale;
        let ih = (26.0 * s).round();
        let n = self.widgets[wi].items.len() as f32;
        let h = n * ih + 8.0 * s;
        let y = rect[1] + (24.0 * s).round() + 2.0;
        let y = if y + h > self.layout.h - 4.0 { (rect[1] - h - 2.0).max(4.0) } else { y };
        [rect[0], y, rect[2].max(self.fonts.as_ref().map_or(0.0, |f| self.widgets[wi].items.iter().map(|t| f.ui.measure(t)).fold(0.0, f32::max) + 48.0 * s)), h]
    }

    pub fn field_mut(&mut self, id: u16) -> Option<&mut Field> {
        let i = self.widget_index(id)?;
        self.widgets[i].field.as_deref_mut()
    }
}
