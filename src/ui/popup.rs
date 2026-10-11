//! Popup menus (the menu bar's drop-downs and the right-click menus): a stack of lists that fly out to
//! the right, driven by mouse and keyboard events and drawn by the UI itself.
use super::render::alpha;
use super::style::*;

#[derive(Clone)]
pub struct MenuItem {
    pub id: u16,
    pub text: String,
    pub shortcut: String,
    pub checked: bool,
    pub disabled: bool,
    pub separator: bool,
    pub sub: Vec<MenuItem>,
}

impl MenuItem {
    /// `text` may carry a shortcut hint after a tab ("Delete\tDel").
    pub fn new(id: u16, text: &str) -> MenuItem {
        let (t, s) = text.split_once('\t').unwrap_or((text, ""));
        MenuItem { id, text: t.to_string(), shortcut: s.to_string(), checked: false, disabled: false, separator: false, sub: Vec::new() }
    }

    pub fn sep() -> MenuItem {
        MenuItem { separator: true, ..MenuItem::new(0, "") }
    }

    pub fn submenu(text: &str, sub: Vec<MenuItem>) -> MenuItem {
        MenuItem { sub, ..MenuItem::new(0, text) }
    }

    pub fn checked(mut self, on: bool) -> MenuItem {
        self.checked = on;
        self
    }

    pub fn enabled(mut self, on: bool) -> MenuItem {
        self.disabled = !on;
        self
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing decided yet.
    Open,
    /// A command was picked.
    Command(u16),
    /// The menu was dismissed.
    Closed,
}

struct Entry {
    id: u16,
    text: String,
    shortcut: String,
    separator: bool,
    enabled: bool,
    checked: bool,
    sub: Vec<MenuItem>,
    /// Vertical extent inside the level.
    top: f32,
    bottom: f32,
}

impl Entry {
    fn selectable(&self) -> bool {
        self.enabled && !self.separator
    }
}

struct Level {
    entries: Vec<Entry>,
    rect: [f32; 4],
    hot: Option<usize>,
}

pub struct Popup {
    levels: Vec<Level>,
    /// The menu-bar title this menu belongs to, if any.
    pub from_bar: Option<usize>,
    /// Opened by a button press: releasing over an item (after moving there) picks it.
    drag_select: bool,
    moved: bool,
}

fn inside(p: (f32, f32), r: [f32; 4]) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

fn make_level(items: &[MenuItem], x: f32, y: f32, flip_x: Option<f32>, screen: (f32, f32), f: &Fonts, s: f32) -> Level {
    let pad = (6.0 * s).round();
    let mut entries = Vec::new();
    let (mut width, mut top) = ((200.0 * s).round(), pad);
    for it in items {
        let h = if it.separator { (9.0 * s).round() } else { (26.0 * s).round() };
        if !it.separator {
            let mut w = 34.0 * s + f.ui.measure(&it.text) + 30.0 * s;
            if !it.shortcut.is_empty() {
                w += 28.0 * s + f.ui.measure(&it.shortcut);
            }
            width = width.max(w.round());
        }
        entries.push(Entry { id: it.id, text: it.text.clone(), shortcut: it.shortcut.clone(), separator: it.separator, enabled: !it.disabled, checked: it.checked, sub: it.sub.clone(), top, bottom: top + h });
        top += h;
    }
    let height = top + pad;
    let mut x = x;
    if x + width > screen.0 - 4.0 {
        x = flip_x.map_or(screen.0 - width - 4.0, |fx| fx - width);
    }
    x = x.max(4.0);
    let y = if y + height > screen.1 - 4.0 { (screen.1 - height - 4.0).max(4.0) } else { y };
    Level { entries, rect: [x, y, width, height], hot: None }
}

impl Popup {
    /// Opens a menu with its top-left corner at (x, y).
    pub fn open(items: &[MenuItem], x: f32, y: f32, screen: (f32, f32), f: &Fonts, s: f32, from_bar: Option<usize>, by_press: bool) -> Popup {
        Popup { levels: vec![make_level(items, x, y, None, screen, f, s)], from_bar, drag_select: by_press, moved: false }
    }

    pub fn contains(&self, p: (f32, f32)) -> bool {
        self.levels.iter().any(|l| inside(p, l.rect))
    }

    fn item_at(l: &Level, p: (f32, f32)) -> Option<usize> {
        let local = p.1 - l.rect[1];
        l.entries.iter().position(|e| local >= e.top && local < e.bottom)
    }

    fn open_sub(&mut self, k: usize, i: usize, screen: (f32, f32), f: &Fonts, s: f32) {
        let (rect, top, sub) = {
            let l = &self.levels[k];
            (l.rect, l.entries[i].top, l.entries[i].sub.clone())
        };
        if sub.is_empty() {
            return;
        }
        self.levels.truncate(k + 1);
        self.levels.push(make_level(&sub, rect[0] + rect[2] - 4.0 * s, rect[1] + top - 6.0 * s, Some(rect[0] + 4.0 * s), screen, f, s));
    }

    pub fn mouse_move(&mut self, p: (f32, f32), screen: (f32, f32), f: &Fonts, s: f32) {
        let Some(k) = self.levels.iter().rposition(|l| inside(p, l.rect)) else { return };
        let i = Self::item_at(&self.levels[k], p).filter(|&i| self.levels[k].entries[i].selectable());
        if self.levels[k].hot != i {
            self.moved = true;
            self.levels[k].hot = i;
            self.levels.truncate(k + 1);
            if let Some(i) = i {
                self.open_sub(k, i, screen, f, s);
            }
        }
    }

    fn pick(&mut self, k: usize, i: usize, screen: (f32, f32), f: &Fonts, s: f32) -> Outcome {
        if self.levels[k].entries[i].sub.is_empty() {
            Outcome::Command(self.levels[k].entries[i].id)
        } else {
            self.open_sub(k, i, screen, f, s);
            Outcome::Open
        }
    }

    pub fn mouse_down(&mut self, p: (f32, f32), screen: (f32, f32), f: &Fonts, s: f32) -> Outcome {
        match self.levels.iter().rposition(|l| inside(p, l.rect)) {
            Some(k) => match Self::item_at(&self.levels[k], p).filter(|&i| self.levels[k].entries[i].selectable()) {
                Some(i) => self.pick(k, i, screen, f, s),
                None => Outcome::Open,
            },
            None => Outcome::Closed,
        }
    }

    pub fn mouse_up(&mut self, p: (f32, f32), screen: (f32, f32), f: &Fonts, s: f32) -> Outcome {
        if !(self.drag_select && self.moved) {
            return Outcome::Open;
        }
        match self.levels.iter().rposition(|l| inside(p, l.rect)) {
            Some(k) => match Self::item_at(&self.levels[k], p).filter(|&i| self.levels[k].entries[i].selectable()) {
                Some(i) if self.levels[k].entries[i].sub.is_empty() => self.pick(k, i, screen, f, s),
                _ => Outcome::Open,
            },
            None => Outcome::Open,
        }
    }

    fn move_hot(l: &mut Level, dir: i32) {
        let n = l.entries.len() as i32;
        let mut i = l.hot.map_or(if dir > 0 { -1 } else { n }, |h| h as i32);
        for _ in 0..n {
            i = (i + dir).rem_euclid(n);
            if l.entries[i as usize].selectable() {
                l.hot = Some(i as usize);
                return;
            }
        }
    }

    /// Navigation keys (Windows virtual-key codes). Left / Right on the top level are reported as
    /// `Switch` through `bar_step` so the menu bar can open its neighbour.
    pub fn key(&mut self, vk: u32, screen: (f32, f32), f: &Fonts, s: f32, bar_step: &mut i32) -> Outcome {
        let top = self.levels.len() - 1;
        match vk {
            0x1B => {
                if top > 0 {
                    self.levels.truncate(top);
                } else {
                    return Outcome::Closed;
                }
            }
            0x28 => Self::move_hot(&mut self.levels[top], 1),
            0x26 => Self::move_hot(&mut self.levels[top], -1),
            0x27 => match self.levels[top].hot {
                Some(i) if !self.levels[top].entries[i].sub.is_empty() => {
                    self.open_sub(top, i, screen, f, s);
                    let n = self.levels.len() - 1;
                    Self::move_hot(&mut self.levels[n], 1);
                }
                _ => *bar_step = 1,
            },
            0x25 => {
                if top > 0 {
                    self.levels.truncate(top);
                } else {
                    *bar_step = -1;
                }
            }
            0x0D | 0x20 => {
                if let Some(i) = self.levels[top].hot {
                    if self.levels[top].entries[i].sub.is_empty() {
                        return Outcome::Command(self.levels[top].entries[i].id);
                    }
                    self.open_sub(top, i, screen, f, s);
                    let n = self.levels.len() - 1;
                    Self::move_hot(&mut self.levels[n], 1);
                }
            }
            _ => {}
        }
        Outcome::Open
    }

    pub fn draw(&self, c: &mut Ctx) {
        let s = c.s;
        for l in &self.levels {
            let [x, y, w, h] = l.rect;
            c.r.shadow(x + 1.0, y + 6.0 * s, w, h, 10.0 * s, 18.0 * s, [0.0, 0.0, 0.0, 0.55]);
            c.r.rect(x, y, w, h, 10.0 * s, c_menu_bg(), 1.0, c_border());
            for (i, e) in l.entries.iter().enumerate() {
                let (ey, eh) = (y + e.top, e.bottom - e.top);
                if e.separator {
                    c.r.fill(x + 10.0 * s, ey + (eh * 0.5).floor(), w - 20.0 * s, 1.0, c_border_soft());
                    continue;
                }
                if l.hot == Some(i) && e.enabled {
                    c.r.rect(x + 5.0 * s, ey + 1.0, w - 10.0 * s, eh - 2.0, 6.0 * s, c_menu_hot(), 0.0, c_menu_hot());
                }
                let color = if e.enabled { c_text() } else { c_dim() };
                c.text_in(&c.f.ui, &e.text, x + 34.0 * s, ey, eh, color);
                if !e.shortcut.is_empty() {
                    let sw = c.f.ui.measure(&e.shortcut);
                    c.text_in(&c.f.ui, &e.shortcut, x + w - 30.0 * s - sw, ey, eh, if e.enabled { c_muted() } else { c_dim() });
                }
                if e.checked {
                    c.check_mark(x + 10.0 * s, ey + (eh - 14.0 * s) * 0.5, 14.0 * s, c_accent());
                }
                if !e.sub.is_empty() {
                    c.arrow(x + w - 16.0 * s, ey + eh * 0.5, 5.0 * s, false, if e.enabled { c_muted() } else { c_dim() });
                }
            }
        }
    }

    /// Dim the scene behind a modal menu? Menus do not dim; this only exists so callers can ask.
    pub fn hot_is_leaf(&self) -> bool {
        self.levels.last().and_then(|l| l.hot.map(|i| l.entries[i].sub.is_empty())).unwrap_or(false)
    }
}

/// Soft backdrop color used under dialogs.
pub fn backdrop() -> super::render::Color {
    alpha([0.0, 0.0, 0.0, 1.0], 0.55)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::font::{rasterize, Font};

    pub fn fonts() -> Fonts {
        Fonts { ui: Font::from_bitmap(&rasterize("Segoe UI", 14, false), 0), bold: Font::from_bitmap(&rasterize("Segoe UI", 13, true), 0), mono: Font::from_bitmap(&rasterize("Consolas", 14, false), 0), pixel: Font::from_bitmap(&rasterize("Consolas", 12, false), 0) }
    }

    fn items() -> Vec<MenuItem> {
        vec![
            MenuItem::new(1, "Open\tCtrl+O"),
            MenuItem::sep(),
            MenuItem::new(2, "Disabled").enabled(false),
            MenuItem::submenu("More", vec![MenuItem::new(10, "Inner"), MenuItem::new(11, "Other")]),
            MenuItem::new(3, "Last"),
        ]
    }

    #[test]
    fn clicking_picks_commands_and_ignores_disabled_items_and_separators() {
        let f = fonts();
        let screen = (800.0, 600.0);
        let mut p = Popup::open(&items(), 100.0, 100.0, screen, &f, 1.0, None, false);
        let rect = p.levels[0].rect;
        let at = |p: &Popup, i: usize| (rect[0] + 40.0, rect[1] + (p.levels[0].entries[i].top + p.levels[0].entries[i].bottom) * 0.5);
        assert_eq!(p.mouse_down(at(&p, 0), screen, &f, 1.0), Outcome::Command(1));
        assert_eq!(p.mouse_down(at(&p, 1), screen, &f, 1.0), Outcome::Open, "separator");
        assert_eq!(p.mouse_down(at(&p, 2), screen, &f, 1.0), Outcome::Open, "disabled");
        assert_eq!(p.mouse_down((5.0, 5.0), screen, &f, 1.0), Outcome::Closed, "outside");
    }

    #[test]
    fn flyouts_open_to_the_side_and_can_be_picked() {
        let f = fonts();
        let screen = (800.0, 600.0);
        let mut p = Popup::open(&items(), 100.0, 100.0, screen, &f, 1.0, None, false);
        let r0 = p.levels[0].rect;
        let y3 = r0[1] + (p.levels[0].entries[3].top + p.levels[0].entries[3].bottom) * 0.5;
        p.mouse_move((r0[0] + 50.0, y3), screen, &f, 1.0);
        assert_eq!(p.levels.len(), 2);
        let r1 = p.levels[1].rect;
        assert!(r1[0] >= r0[0] + r0[2] - 6.0, "flyout starts at the right edge");
        let y_inner = r1[1] + (p.levels[1].entries[0].top + p.levels[1].entries[0].bottom) * 0.5;
        assert_eq!(p.mouse_down((r1[0] + 30.0, y_inner), screen, &f, 1.0), Outcome::Command(10));
        // moving back to a plain item closes the flyout
        p.mouse_move((r0[0] + 50.0, r0[1] + p.levels[0].entries[0].top + 5.0), screen, &f, 1.0);
        assert_eq!(p.levels.len(), 1);
    }

    #[test]
    fn keyboard_navigation() {
        let f = fonts();
        let screen = (800.0, 600.0);
        let mut p = Popup::open(&items(), 100.0, 100.0, screen, &f, 1.0, None, false);
        let mut step = 0;
        p.key(0x28, screen, &f, 1.0, &mut step); // down: first item
        p.key(0x28, screen, &f, 1.0, &mut step); // skips the separator and the disabled item: "More"
        assert_eq!(p.levels[0].hot, Some(3));
        p.key(0x27, screen, &f, 1.0, &mut step); // right opens the flyout
        assert_eq!(p.levels.len(), 2);
        assert_eq!(p.key(0x0D, screen, &f, 1.0, &mut step), Outcome::Command(10));
        p.key(0x1B, screen, &f, 1.0, &mut step);
        assert_eq!(p.levels.len(), 1);
        assert_eq!(p.key(0x1B, screen, &f, 1.0, &mut step), Outcome::Closed);
        p.key(0x25, screen, &f, 1.0, &mut step);
        assert_eq!(step, -1, "left on the top level asks the bar for the previous menu");
    }

    #[test]
    fn menus_stay_on_the_screen() {
        let f = fonts();
        let p = Popup::open(&items(), 790.0, 590.0, (800.0, 600.0), &f, 1.0, None, false);
        let r = p.levels[0].rect;
        assert!(r[0] >= 0.0 && r[0] + r[2] <= 800.0 && r[1] >= 0.0 && r[1] + r[3] <= 600.0, "{r:?}");
    }
}
