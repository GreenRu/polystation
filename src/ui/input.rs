//! Mouse and keyboard handling of the UI: what a click, a drag, a key or a typed character does.
use super::popup::Outcome;
use super::tree::Hit;
use super::widgets::{Clipboard, Kind};
use super::*;
use crate::win32::{CMD_TAB_GUIDE, CMD_TAB_PROJECT, CMD_TAB_SCRIPT};

/// The system clipboard.
struct SysClipboard;

impl Clipboard for SysClipboard {
    fn get(&mut self) -> String {
        win32::clipboard_get()
    }

    fn set(&mut self, text: &str) {
        win32::clipboard_set(text);
    }
}

impl Ui {
    /// A popup was answered: the script view's own menus are handled here, everything else goes to the application.
    fn push_command(&mut self, id: u16) {
        if script_view::is_menu_command(id) {
            if let Some(v) = self.active_view.checked_sub(1).and_then(|i| self.views.get_mut(i)) {
                v.menu_choice(id);
            }
            self.flush_view_events();
        } else {
            self.commands.push(id);
        }
    }

    /// The script view under the mouse (when a script tab is showing).
    fn over_view(&self, p: (f32, f32)) -> bool {
        self.script_active() && inside(p, self.layout.viewport())
    }

    fn screen(&self) -> (f32, f32) {
        (self.layout.w, self.layout.h)
    }

    fn insp_track(&self) -> Rect {
        let i = self.layout.inspector();
        [i[0] + i[2] - 12.0 * self.scale, i[1], 12.0 * self.scale, i[3]]
    }

    /// The widget under a point that can be interacted with.
    fn widget_at(&self, p: (f32, f32)) -> Option<usize> {
        (0..self.widgets.len()).rev().find(|&i| {
            let w = &self.widgets[i];
            if matches!(w.kind, Kind::Static | Kind::Section) {
                return false;
            }
            match self.widget_rect(i) {
                Some(r) => {
                    let clip = self.clip_of(w.group);
                    let r = if w.kind == Kind::Combo || (w.kind == Kind::Edit && !w.field.as_ref().is_some_and(|f| f.multi)) { [r[0], r[1], r[2], r[3].min((26.0 * self.scale).round())] } else { r };
                    inside(p, r) && inside(p, clip)
                }
                None => false,
            }
        })
    }

    fn bar_index_at(&self, p: (f32, f32)) -> Option<usize> {
        if p.1 >= self.layout.mb {
            return None;
        }
        self.bar_rects().iter().position(|r| p.0 >= r[0] && p.0 < r[0] + r[2])
    }

    fn close_popup(&mut self) {
        self.popup = None;
    }

    fn field_rect_font(&self, i: usize) -> Option<(Rect, bool)> {
        let r = self.widget_rect(i)?;
        Some((r, self.widgets[i].field.as_ref()?.multi))
    }

    fn show_caret(&mut self, i: usize) {
        let Some((rect, multi)) = self.field_rect_font(i) else { return };
        let Some(f) = self.fonts.as_ref() else { return };
        let font = if multi { &f.mono } else { &f.ui };
        let s = self.scale;
        if let Some(fl) = self.widgets[i].field.as_mut() {
            fl.show_caret(rect, font, s);
        }
    }

    fn focus_field_index(&mut self, i: usize, select_all: bool) {
        self.focus = Focus::Field(i);
        if let Some(f) = self.widgets[i].field.as_mut() {
            if select_all {
                f.edit.select_all();
            }
            f.blink = Instant::now();
        }
    }

    /// Moves the keyboard focus to the next / previous text box (Tab).
    fn tab_to(&mut self, from: usize, dir: i32) {
        let n = self.widgets.len();
        for k in 1..=n {
            let j = (from as i32 + dir * k as i32).rem_euclid(n as i32) as usize;
            let w = &self.widgets[j];
            if w.kind == Kind::Edit && w.enabled && !w.field.as_ref().is_some_and(|f| f.multi) && self.widget_visible(w) {
                self.focus_field_index(j, true);
                // bring it into view
                if let (Some(r), true) = (self.widget_rect(j), region_of(self.widgets[j].group) == Region::Inspector) {
                    let v = self.layout.inspector();
                    if r[1] < v[1] {
                        self.insp_scroll.offset -= v[1] - r[1] + 8.0;
                    } else if r[1] + r[3] > v[1] + v[3] {
                        self.insp_scroll.offset += r[1] + r[3] - v[1] - v[3] + 8.0;
                    }
                }
                return;
            }
        }
    }

    // ---------------------------------------------------------------- mouse
    pub fn mouse_leave(&mut self) {
        self.hover = None;
        self.bar_hot = None;
        self.tree.hover = None;
        self.list.hover = None;
    }

    pub fn mouse_move(&mut self, p: (f32, f32)) -> Route {
        if self.game {
            self.mouse = p;
            return Route::Viewport;
        }
        self.mouse = p;
        let s = self.scale;
        if self.script_active() {
            if self.active == Active::ScriptView || self.over_view(p) {
                let rect = self.layout.viewport();
                if let (Some(f), Some(v)) = (self.fonts.as_ref(), self.active_view.checked_sub(1).and_then(|i| self.views.get_mut(i))) {
                    v.mouse_move(p, rect, s, f);
                    self.cursor = if v.cursor_hint() { Cursor::Hand } else { Cursor::Arrow };
                }
                self.flush_view_events();
                return Route::Ui;
            }
        }
        // captured drags
        match self.active {
            Active::ScriptView => {}
            Active::Num { idx, start_x, start_val, dragging } => {
                let mut dragging = dragging;
                if !dragging && (p.0 - start_x).abs() >= 4.0 {
                    dragging = true;
                    self.active = Active::Num { idx, start_x, start_val, dragging };
                }
                if dragging {
                    let id = self.widgets[idx].id;
                    if let Some((step, min, max, whole, scaled)) = win32::drag_spec(id) {
                        let mut st = step * if scaled { self.drag_scale } else { 1.0 };
                        if win32::key_down(0x10) {
                            st *= 0.1;
                        }
                        if win32::key_down(0x11) {
                            st *= 10.0;
                        }
                        let mut v = start_val + (p.0 - start_x) * st;
                        if whole {
                            v = v.round();
                        }
                        v = v.clamp(min, max);
                        let text = win32::format_number(v, whole);
                        if let Some(f) = self.widgets[idx].field.as_mut() {
                            if f.edit.text() != text {
                                f.edit.set_text(&text);
                                self.edit_events.push(id);
                            }
                        }
                    }
                }
                self.cursor = Cursor::SizeWE;
                return Route::Ui;
            }
            Active::Select(i) => {
                if let (Some((rect, multi)), Some(f)) = (self.field_rect_font(i), self.fonts.as_ref()) {
                    let font = if multi { &f.mono } else { &f.ui };
                    if let Some(fl) = self.widgets[i].field.as_mut() {
                        fl.mouse_drag(p, rect, font, s);
                    }
                }
                self.cursor = Cursor::IBeam;
                return Route::Ui;
            }
            Active::ScrollInspector => {
                let (content, view) = (self.inspector_content(), self.layout.inspector()[3]);
                let track = self.insp_track();
                self.insp_scroll.drag_to(p.1, track, content, view);
                return Route::Ui;
            }
            Active::TreeDrag { node, start, on } => {
                let r = self.layout.tree();
                let mut on = on;
                if !on && (p.0 - start.0).hypot(p.1 - start.1) >= 5.0 * s {
                    on = true;
                    self.active = Active::TreeDrag { node, start, on };
                }
                if on {
                    self.tree.drag = Some((node, p));
                    self.tree.drop = self.tree.drop_target(p, r, s, node);
                    // near the top / bottom edge the list scrolls along
                    let edge = 22.0 * s;
                    let content = self.tree.content_height(s);
                    if p.1 < r[1] + edge {
                        self.tree.scroll.offset -= 6.0 * s;
                    } else if p.1 > r[1] + r[3] - edge {
                        self.tree.scroll.offset += 6.0 * s;
                    }
                    self.tree.scroll.clamp(content, r[3]);
                    self.cursor = Cursor::Hand;
                }
                return Route::Ui;
            }
            Active::ScrollTree => {
                let r = self.layout.tree();
                let track = [r[0] + r[2] - 12.0 * s, r[1], 12.0 * s, r[3]];
                self.tree.scroll.drag_to(p.1, track, self.tree.content_height(s), r[3]);
                return Route::Ui;
            }
            Active::ScrollList => {
                if let Some(rect) = self.widget_index(win32::LV_EXPLORER).and_then(|i| self.widget_rect(i)) {
                    let hh = (list::HEADER_H * s).round();
                    let track = [rect[0] + rect[2] - 12.0 * s, rect[1] + hh, 12.0 * s, rect[3] - hh];
                    self.list.scroll.drag_to(p.1, track, self.list.content_height(s), rect[3] - hh);
                }
                return Route::Ui;
            }
            Active::ScrollField(i) => {
                if let (Some((rect, multi)), Some(f)) = (self.field_rect_font(i), self.fonts.as_ref()) {
                    let font = if multi { &f.mono } else { &f.ui };
                    if let Some(fl) = self.widgets[i].field.as_mut() {
                        let (_, ch) = fl.content_size(font, s);
                        let track = [rect[0] + rect[2] - 12.0 * s, rect[1], 12.0 * s, rect[3]];
                        fl.scroll.drag_to(p.1, track, ch, rect[3]);
                    }
                }
                return Route::Ui;
            }
            Active::Button(_) => return Route::Ui,
            Active::None => {}
        }
        // popup menu
        if self.popup.is_some() {
            let screen = self.screen();
            if let (Some(pop), Some(f)) = (self.popup.as_mut(), self.fonts.as_ref()) {
                pop.mouse_move(p, screen, f, s);
            }
            // while a bar menu is open, moving onto another title opens that one
            let cur = self.popup.as_ref().and_then(|pp| pp.from_bar);
            if let (Some(cur), Some(k)) = (cur, self.bar_index_at(p)) {
                if k != cur {
                    self.open_bar_menu(k, false);
                }
            }
            self.cursor = Cursor::Arrow;
            return Route::Ui;
        }
        // open drop-down list
        if let Some((wi, _)) = self.combo_open {
            if let Some(rect) = self.widget_rect(wi) {
                let list = self.combo_list_rect(wi, rect);
                let ih = (26.0 * s).round();
                let hot = inside(p, list).then(|| ((p.1 - list[1] - 4.0 * s) / ih).floor().max(0.0) as usize).filter(|&k| k < self.widgets[wi].items.len());
                self.combo_open = Some((wi, hot));
            }
            self.cursor = Cursor::Arrow;
            return Route::Ui;
        }
        self.bar_hot = self.bar_index_at(p);
        self.hover = self.widget_at(p);
        self.cursor = match self.hover.map(|i| &self.widgets[i]) {
            Some(w) if w.kind == Kind::Edit && w.enabled => {
                if win32::drag_spec(w.id).is_some() && self.focus != Focus::Field(self.hover.unwrap_or(0)) { Cursor::SizeWE } else { Cursor::IBeam }
            }
            Some(w) if w.kind == Kind::Button && w.enabled => Cursor::Hand,
            Some(w) if w.kind == Kind::Combo && w.enabled => Cursor::Hand,
            _ => Cursor::Arrow,
        };
        let tr = self.layout.tree();
        self.tree.hover = if inside(p, tr) { match self.tree.hit(p, tr, s) { Hit::Row(n) | Hit::Arrow(n) | Hit::Check(n) | Hit::Badge(n) => Some(n), Hit::Nothing => None } } else { None };
        self.list.hover = match self.widget_index(win32::LV_EXPLORER).and_then(|i| self.widget_rect(i)) {
            Some(r) if self.bottom_tab == 0 && inside(p, r) => self.list.row_at(p, r, s),
            _ => None,
        };
        if inside(p, self.layout.viewport()) { Route::Viewport } else { Route::Ui }
    }

    pub fn mouse_down(&mut self, btn: u8, p: (f32, f32), double: bool, mods: Mods) -> Route {
        if self.game {
            self.mouse = p;
            return Route::Viewport;
        }
        self.mouse = p;
        let s = self.scale;
        let screen = self.screen();
        // an open popup takes the click first
        if self.popup.is_some() {
            let from = self.popup.as_ref().and_then(|pp| pp.from_bar);
            let outcome = match (self.popup.as_mut(), self.fonts.as_ref()) {
                (Some(pop), Some(f)) if btn == 0 => pop.mouse_down(p, screen, f, s),
                _ => Outcome::Closed,
            };
            match outcome {
                Outcome::Command(id) => {
                    self.push_command(id);
                    self.close_popup();
                }
                Outcome::Closed => {
                    self.close_popup();
                    // clicking the title of the open menu closes it; any other click goes on to its target
                    if from.is_some() && from == self.bar_index_at(p) {
                        return Route::Ui;
                    }
                    if self.bar_index_at(p).is_some() && btn == 0 {
                        // fall through: opens the other title below
                    } else if btn == 0 {
                        return Route::Ui; // the click only closed the menu
                    }
                }
                Outcome::Open => {}
            }
            if self.popup.is_some() {
                return Route::Ui;
            }
            if btn != 0 || self.bar_index_at(p).is_none() {
                return Route::Ui;
            }
        }
        // an open drop-down list
        if let Some((wi, _)) = self.combo_open {
            self.combo_open = None;
            if let Some(rect) = self.widget_rect(wi) {
                let list = self.combo_list_rect(wi, rect);
                if btn == 0 && inside(p, list) {
                    let ih = (26.0 * s).round();
                    let k = ((p.1 - list[1] - 4.0 * s) / ih).floor().max(0.0) as usize;
                    if k < self.widgets[wi].items.len() {
                        if self.widgets[wi].sel != k {
                            self.widgets[wi].sel = k;
                            self.combo_events.push(self.widgets[wi].id);
                        }
                        return Route::Ui;
                    }
                }
            }
            return Route::Ui;
        }
        if btn == 0 {
            return self.left_down(p, double, mods);
        }
        // right button
        if self.over_view(p) && btn == 1 {
            let rect = self.layout.viewport();
            if let (Some(f), Some(v)) = (self.fonts.as_ref(), self.active_view.checked_sub(1).and_then(|i| self.views.get_mut(i))) {
                v.mouse_down(p, true, rect, s, false, f);
            }
            self.focus = Focus::Script;
            self.flush_view_events();
            return Route::Ui;
        }
        let tr = self.layout.tree();
        if inside(p, tr) {
            self.focus = Focus::Tree;
            if let Hit::Row(n) | Hit::Arrow(n) | Hit::Check(n) = self.tree.hit(p, tr, s) {
                self.tree.selected = Some(n);
            }
            self.tree_rclick = Some(p);
            return Route::Ui;
        }
        if let Some(i) = self.widget_index(win32::LV_EXPLORER) {
            if let Some(rect) = self.widget_rect(i) {
                if self.bottom_tab == 0 && inside(p, rect) {
                    self.focus = Focus::List;
                    if let Some(r) = self.list.row_at(p, rect, s) {
                        self.list.selected = Some(r);
                        self.explorer_click = true;
                    }
                    self.explorer_rclick = Some(p);
                    return Route::Ui;
                }
            }
        }
        if inside(p, self.layout.viewport()) {
            self.focus = Focus::None;
            return Route::Viewport;
        }
        Route::Ui
    }

    fn left_down(&mut self, p: (f32, f32), double: bool, mods: Mods) -> Route {
        let s = self.scale;
        let l = self.layout;
        // the tabs above the 3D view
        if inside(p, l.view_tabs()) {
            self.focus = Focus::None;
            let rects = self.view_tab_rects();
            for (i, (r, close)) in rects.iter().enumerate() {
                if close.is_some_and(|c| inside(p, c)) {
                    self.close_view(i);
                    return Route::Ui;
                }
                if inside(p, *r) {
                    self.active_view = i;
                    if i > 0 {
                        self.focus = Focus::Script;
                    }
                    return Route::Ui;
                }
            }
            return Route::Ui;
        }
        // a script tab: everything in the view is the script's
        if self.over_view(p) {
            let rect = l.viewport();
            if let (Some(f), Some(v)) = (self.fonts.as_ref(), self.active_view.checked_sub(1).and_then(|i| self.views.get_mut(i))) {
                v.mouse_down(p, false, rect, s, mods.shift, f);
            }
            self.focus = Focus::Script;
            self.active = Active::ScriptView;
            self.flush_view_events();
            return Route::Ui;
        }
        // menu bar
        if let Some(k) = self.bar_index_at(p) {
            self.focus = Focus::None;
            self.open_bar_menu(k, true);
            return Route::Ui;
        }
        // widgets
        if let Some(i) = self.widget_at(p) {
            if !self.widgets[i].enabled {
                return Route::Ui;
            }
            match self.widgets[i].kind {
                Kind::Button => {
                    self.focus = Focus::None;
                    self.active = Active::Button(i);
                }
                Kind::Combo => {
                    self.focus = Focus::None;
                    self.combo_open = Some((i, None));
                }
                Kind::Edit => {
                    let rect = self.widget_rect(i).unwrap_or_default();
                    let id = self.widgets[i].id;
                    let multi = self.widgets[i].field.as_ref().is_some_and(|f| f.multi);
                    // the scroll bar of a text area
                    if multi {
                        if let Some(f) = self.fonts.as_ref() {
                            let font = &f.mono;
                            if let Some(fl) = self.widgets[i].field.as_mut() {
                                let (_, ch) = fl.content_size(font, s);
                                let track = [rect[0] + rect[2] - 12.0 * s, rect[1], 12.0 * s, rect[3]];
                                if fl.scroll.press(p, track, ch, rect[3]) {
                                    self.active = Active::ScrollField(i);
                                    return Route::Ui;
                                }
                            }
                        }
                    }
                    let focused = self.focus == Focus::Field(i);
                    if !focused && win32::drag_spec(id).is_some() {
                        // a number box: a click edits it, a drag changes the value
                        let val = self.get_text(id).trim().replace(',', ".").parse::<f32>().unwrap_or(0.0);
                        self.active = Active::Num { idx: i, start_x: p.0, start_val: val, dragging: false };
                    } else {
                        self.focus_field_index(i, false);
                        if let Some(f) = self.fonts.as_ref() {
                            let font = if multi { &f.mono } else { &f.ui };
                            if let Some(fl) = self.widgets[i].field.as_mut() {
                                fl.mouse_down(p, rect, font, s, mods.shift);
                                if double && !multi {
                                    fl.edit.select_all();
                                }
                            }
                        }
                        self.active = Active::Select(i);
                    }
                }
                _ => {}
            }
            return Route::Ui;
        }
        // scroll bars and lists
        let ins = l.inspector();
        if inside(p, ins) {
            let (content, view) = (self.inspector_content(), ins[3]);
            let track = self.insp_track();
            if self.insp_scroll.press(p, track, content, view) {
                self.active = Active::ScrollInspector;
            } else {
                self.focus = Focus::None;
            }
            return Route::Ui;
        }
        let tr = l.tree();
        if inside(p, tr) {
            self.focus = Focus::Tree;
            let track = [tr[0] + tr[2] - 12.0 * s, tr[1], 12.0 * s, tr[3]];
            if self.tree.scroll.press(p, track, self.tree.content_height(s), tr[3]) {
                self.active = Active::ScrollTree;
                return Route::Ui;
            }
            // the eyedropper of a script block: the next click on an object of the hierarchy picks it
            let hit = self.tree.hit(p, tr, s);
            if let Some(v) = self.active_view.checked_sub(1).and_then(|i| self.views.get_mut(i)) {
                if v.eyedrop.is_some() {
                    if let Hit::Row(n) | Hit::Arrow(n) | Hit::Check(n) | Hit::Badge(n) = hit {
                        let name = self.tree.node(n).map(|x| x.text.clone()).unwrap_or_default();
                        if !name.starts_with('[') && !name.starts_with('<') {
                            v.pick_object(&name);
                            self.flush_view_events();
                        } else {
                            v.eyedrop = None;
                        }
                    } else {
                        v.eyedrop = None;
                    }
                    return Route::Ui;
                }
            }
            match hit {
                Hit::Badge(n) => {
                    self.tree.selected = Some(n);
                    self.script_open = self.tree.node(n).map(|x| x.text.clone());
                }
                Hit::Arrow(n) => {
                    if let Some(node) = self.tree.node_mut(n) {
                        node.expanded = !node.expanded;
                    }
                }
                Hit::Check(n) => {
                    if let Some(node) = self.tree.node_mut(n) {
                        node.checked = !node.checked;
                    }
                }
                Hit::Row(n) => {
                    self.tree_click = true;
                    let marked = self.tree.node(n).is_some_and(|x| x.bold) && !mods.ctrl && !mods.shift;
                    if !marked || double {
                        self.tree.selected = Some(n);
                    }
                    if double {
                        self.tree_dbl_click = true;
                    } else {
                        // a row of a multi-selection keeps the selection (so the whole group can be dragged);
                        // a plain click on it, without dragging, selects just that row when the button comes up
                        self.active = Active::TreeDrag { node: n, start: p, on: false };
                    }
                }
                Hit::Nothing => {}
            }
            return Route::Ui;
        }
        if self.bottom_tab == 0 {
            if let Some(rect) = self.widget_index(win32::LV_EXPLORER).and_then(|i| self.widget_rect(i)) {
                if inside(p, rect) {
                    self.focus = Focus::List;
                    let hh = (list::HEADER_H * s).round();
                    let track = [rect[0] + rect[2] - 12.0 * s, rect[1] + hh, 12.0 * s, rect[3] - hh];
                    if self.list.scroll.press(p, track, self.list.content_height(s), rect[3] - hh) {
                        self.active = Active::ScrollList;
                        return Route::Ui;
                    }
                    if let Some(r) = self.list.row_at(p, rect, s) {
                        self.list.selected = Some(r);
                        self.explorer_click = true;
                        if double {
                            self.explorer_dbl_click = true;
                        }
                    }
                    return Route::Ui;
                }
            }
        }
        // bottom tabs
        for i in 0..3 {
            if inside(p, self.tab_rect(i)) {
                if self.bottom_tab as usize != i {
                    self.show_bottom_tab(i as u8);
                    self.commands.push([CMD_TAB_PROJECT, CMD_TAB_SCRIPT, CMD_TAB_GUIDE][i]);
                }
                return Route::Ui;
            }
        }
        if inside(p, l.viewport()) {
            self.focus = Focus::None;
            return Route::Viewport;
        }
        self.focus = Focus::None;
        Route::Ui
    }

    pub fn mouse_up(&mut self, btn: u8, p: (f32, f32)) -> Route {
        if self.game {
            self.mouse = p;
            return Route::Viewport;
        }
        self.mouse = p;
        if btn != 0 {
            return if inside(p, self.layout.viewport()) { Route::Viewport } else { Route::Ui };
        }
        let screen = self.screen();
        let s = self.scale;
        if self.popup.is_some() {
            if let (Some(pop), Some(f)) = (self.popup.as_mut(), self.fonts.as_ref()) {
                if let Outcome::Command(id) = pop.mouse_up(p, screen, f, s) {
                    self.push_command(id);
                    self.popup = None;
                }
            }
            return Route::Ui;
        }
        let active = std::mem::replace(&mut self.active, Active::None);
        match active {
            Active::Button(i) => {
                if self.widget_rect(i).is_some_and(|r| inside(p, r)) && self.widgets[i].enabled {
                    let w = &mut self.widgets[i];
                    if w.toggle {
                        w.on = !w.on;
                    }
                    self.commands.push(w.id);
                }
                Route::Ui
            }
            Active::Num { idx, dragging, .. } => {
                if !dragging {
                    self.focus_field_index(idx, true);
                } else if let Some(f) = self.widgets[idx].field.as_mut() {
                    f.edit.select_all();
                }
                Route::Ui
            }
            Active::Select(i) => {
                if let Some(f) = self.widgets[i].field.as_mut() {
                    f.selecting = false;
                }
                Route::Ui
            }
            Active::ScrollInspector => {
                self.insp_scroll.release();
                Route::Ui
            }
            Active::ScriptView => {
                let rect = self.layout.viewport();
                if let Some(v) = self.active_view.checked_sub(1).and_then(|i| self.views.get_mut(i)) {
                    v.mouse_up(p, rect, s);
                }
                self.flush_view_events();
                Route::Ui
            }
            Active::ScrollTree => {
                self.tree.scroll.release();
                Route::Ui
            }
            Active::TreeDrag { node, on, .. } => {
                if !on && self.tree.node(node).is_some_and(|x| x.bold) {
                    self.tree.selected = Some(node);
                }
                if on {
                    if let Some((target, at)) = self.tree.drop {
                        self.tree_drop = Some((node, target, at));
                    }
                }
                self.tree.drag = None;
                self.tree.drop = None;
                Route::Ui
            }
            Active::ScrollList => {
                self.list.scroll.release();
                Route::Ui
            }
            Active::ScrollField(i) => {
                if let Some(f) = self.widgets[i].field.as_mut() {
                    f.scroll.release();
                }
                Route::Ui
            }
            Active::None => {
                if inside(p, self.layout.viewport()) { Route::Viewport } else { Route::Ui }
            }
        }
    }

    /// The mouse wheel. Returns true when the UI used it.
    pub fn wheel(&mut self, notches: f32, p: (f32, f32)) -> bool {
        if self.game {
            return false;
        }
        let s = self.scale;
        if self.popup.is_some() || self.combo_open.is_some() {
            return true;
        }
        if self.over_view(p) {
            let rect = self.layout.viewport();
            if let (Some(f), Some(v)) = (self.fonts.as_ref(), self.active_view.checked_sub(1).and_then(|i| self.views.get_mut(i))) {
                v.wheel(notches, p, rect, s, f);
            }
            return true;
        }
        if let Some(i) = self.widget_at(p) {
            if let (Some(rect), Some(f)) = (self.widget_rect(i), self.fonts.as_ref()) {
                let font = &f.mono;
                if let Some(fl) = self.widgets[i].field.as_mut() {
                    if fl.multi {
                        fl.wheel(notches, rect, font, s);
                        return true;
                    }
                }
            }
        }
        let l = self.layout;
        let tr = l.tree();
        if inside(p, tr) {
            self.tree.scroll.wheel(notches, 3.0 * (tree::ROW_H * s).round(), self.tree.content_height(s), tr[3]);
            return true;
        }
        let ins = l.inspector();
        if inside(p, ins) {
            let content = self.inspector_content();
            self.insp_scroll.wheel(notches, 48.0 * s, content, ins[3]);
            return true;
        }
        if let Some(i) = self.widget_index(win32::LV_EXPLORER) {
            if let (true, Some(rect)) = (self.bottom_tab == 0, self.widget_rect(i)) {
                if inside(p, rect) {
                    let hh = (list::HEADER_H * s).round();
                    self.list.scroll.wheel(notches, 3.0 * (list::ROW_H * s).round(), self.list.content_height(s), rect[3] - hh);
                    return true;
                }
            }
        }
        // the guide / script boxes are widgets and were handled above; other chrome swallows the wheel
        !inside(p, l.viewport())
    }

    // ---------------------------------------------------------------- keyboard
    /// A key press (Windows virtual-key code). Returns true when the UI used it.
    pub fn key_down(&mut self, vk: u32, mods: Mods) -> bool {
        let s = self.scale;
        let screen = self.screen();
        if self.popup.is_some() {
            let mut step = 0;
            let outcome = match (self.popup.as_mut(), self.fonts.as_ref()) {
                (Some(pop), Some(f)) => pop.key(vk, screen, f, s, &mut step),
                _ => Outcome::Closed,
            };
            match outcome {
                Outcome::Command(id) => {
                    self.push_command(id);
                    self.popup = None;
                }
                Outcome::Closed => self.popup = None,
                Outcome::Open => {
                    if let (true, Some(cur)) = (step != 0, self.popup.as_ref().and_then(|p| p.from_bar)) {
                        let n = self.bar.len() as i32;
                        self.open_bar_menu((cur as i32 + step).rem_euclid(n) as usize, false);
                    }
                }
            }
            return true;
        }
        if let Some((wi, hot)) = self.combo_open {
            let n = self.widgets[wi].items.len();
            match vk {
                0x1B => self.combo_open = None,
                0x28 | 0x26 if n > 0 => {
                    let cur = hot.unwrap_or(self.widgets[wi].sel);
                    let next = if vk == 0x28 { (cur + 1).min(n - 1) } else { cur.saturating_sub(1) };
                    self.combo_open = Some((wi, Some(next)));
                }
                0x0D | 0x20 => {
                    if let Some(k) = hot.filter(|&k| k < n) {
                        if self.widgets[wi].sel != k {
                            self.widgets[wi].sel = k;
                            self.combo_events.push(self.widgets[wi].id);
                        }
                    }
                    self.combo_open = None;
                }
                _ => {}
            }
            return true;
        }
        // the eyedropper can always be cancelled
        if vk == 0x1B {
            if let Some(v) = self.active_view.checked_sub(1).and_then(|i| self.views.get_mut(i)) {
                if v.eyedrop.is_some() {
                    v.eyedrop = None;
                    return true;
                }
            }
        }
        if self.focus == Focus::Script && self.script_active() {
            let mut clip = SysClipboard;
            let used = self.active_view.checked_sub(1).and_then(|i| self.views.get_mut(i)).is_some_and(|v| v.key(vk, mods.ctrl, mods.shift, &mut clip));
            self.flush_view_events();
            if used {
                return true;
            }
        }
        match self.focus {
            Focus::Field(i) => {
                if vk == 0x1B {
                    self.focus = Focus::None;
                    return true;
                }
                let multi = self.widgets[i].field.as_ref().is_some_and(|f| f.multi);
                if !multi && vk == 0x09 {
                    self.tab_to(i, if mods.shift { -1 } else { 1 });
                    return true;
                }
                if !multi && vk == 0x0D {
                    self.focus = Focus::None;
                    return true;
                }
                let id = self.widgets[i].id;
                let (mut changed, mut used) = (false, false);
                if let Some(f) = self.widgets[i].field.as_mut() {
                    (changed, used) = f.key(vk, mods.ctrl, mods.shift, &mut SysClipboard);
                    f.blink = Instant::now();
                }
                if changed {
                    self.edit_events.push(id);
                }
                self.show_caret(i);
                used
            }
            Focus::Tree => match vk {
                0x26 | 0x28 => {
                    self.tree.step(if vk == 0x28 { 1 } else { -1 });
                    if let Some(sel) = self.tree.selected {
                        let rh = (tree::ROW_H * s).round();
                        if let Some(row) = self.tree.rows().iter().position(|r| r.0 == sel) {
                            let (content, view) = (self.tree.content_height(s), self.layout.tree()[3]);
                            self.tree.scroll.reveal(row as f32 * rh, (row + 1) as f32 * rh, content, view);
                        }
                    }
                    true
                }
                0x25 | 0x27 => {
                    if let Some(n) = self.tree.selected {
                        let has = self.tree.has_children(n);
                        if let Some(node) = self.tree.node_mut(n) {
                            if has {
                                node.expanded = vk == 0x27;
                            }
                        }
                    }
                    true
                }
                0x20 => {
                    if let Some(node) = self.tree.selected.and_then(|n| self.tree.node_mut(n)) {
                        node.checked = !node.checked;
                    }
                    true
                }
                0x0D => {
                    self.tree_dbl_click = true;
                    true
                }
                _ => false,
            },
            Focus::List => match vk {
                0x26 | 0x28 => {
                    let view = self.widget_index(win32::LV_EXPLORER).and_then(|i| self.widget_rect(i)).map_or(100.0, |r| r[3]);
                    self.list.step(if vk == 0x28 { 1 } else { -1 }, view, s);
                    true
                }
                0x0D => {
                    self.explorer_dbl_click = true;
                    true
                }
                _ => false,
            },
            Focus::None | Focus::Script => false,
        }
    }

    /// A typed character.
    pub fn char_input(&mut self, ch: char) -> bool {
        if self.focus == Focus::Script && self.script_active() {
            let used = self.active_view.checked_sub(1).and_then(|i| self.views.get_mut(i)).is_some_and(|v| v.char_input(ch));
            self.flush_view_events();
            return used;
        }
        let Focus::Field(i) = self.focus else { return false };
        let id = self.widgets[i].id;
        let changed = match self.widgets[i].field.as_mut() {
            Some(f) => {
                f.blink = Instant::now();
                f.char_input(ch)
            }
            None => false,
        };
        if changed {
            self.edit_events.push(id);
        }
        self.show_caret(i);
        true
    }

    pub fn blur(&mut self) {
        self.focus = Focus::None;
    }
}
