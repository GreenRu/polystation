//! Visual scripting: a script drawn as a graph of blocks (nodes) joined by wires. White wires carry the
//! order things happen in, coloured wires carry values. A graph is compiled to GScript, so it runs on the
//! same interpreter as hand-written scripts (and its generated code can be read and taken over).
//!
//! The kinds of block are listed in [`DEFS`]; the editor draws whatever the table says, so adding a block
//! means adding one entry here and one arm in `statement` or `value`.
use crate::json::J;
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PinKind {
    /// Order of execution.
    Exec,
    /// A value.
    Data,
}

pub struct PinDef {
    pub name: &'static str,
    pub kind: PinKind,
}

#[derive(Clone, Copy)]
pub enum ParamKind {
    Text,
    /// An object of the scene, picked with the eyedropper (empty / "self" = the object that owns the script).
    Object,
    Choice(&'static [&'static str]),
}

pub struct ParamDef {
    pub name: &'static str,
    pub kind: ParamKind,
    pub default: &'static str,
}

pub struct NodeDef {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub ins: &'static [PinDef],
    pub outs: &'static [PinDef],
    pub params: &'static [ParamDef],
    /// The value of an unconnected data input, per input (entries of exec inputs are unused).
    pub defaults: &'static [&'static str],
}

const fn exec(name: &'static str) -> PinDef {
    PinDef { name, kind: PinKind::Exec }
}
const fn data(name: &'static str) -> PinDef {
    PinDef { name, kind: PinKind::Data }
}
const fn text(name: &'static str, default: &'static str) -> ParamDef {
    ParamDef { name, kind: ParamKind::Text, default }
}
const fn object(name: &'static str) -> ParamDef {
    ParamDef { name, kind: ParamKind::Object, default: "" }
}
const fn choice(name: &'static str, options: &'static [&'static str]) -> ParamDef {
    ParamDef { name, kind: ParamKind::Choice(options), default: options[0] }
}

pub const PROPS: &[&str] = &["position.x", "position.y", "position.z", "rotation.x", "rotation.y", "rotation.z", "scale.x", "scale.y", "scale.z", "color.r", "color.g", "color.b", "alpha", "visible", "solid", "intensity"];
const MATH_OPS: &[&str] = &["+", "-", "*", "/", "%", "min", "max", "pow"];
const COMPARE_OPS: &[&str] = &["<", ">", "<=", ">=", "==", "!="];
const LOGIC_OPS: &[&str] = &["and", "or"];
const FUNCS: &[&str] = &["sin", "cos", "abs", "sqrt", "floor", "round", "sign"];
const BOOLS: &[&str] = &["true", "false"];

const THEN: &[PinDef] = &[exec("then")];
const IN_THEN: &[PinDef] = &[exec("in")];
const OUT_THEN: &[PinDef] = &[exec("out")];
const VALUE: &[PinDef] = &[data("value")];

pub const DEFS: &[NodeDef] = &[
    // ---- events: where a script starts
    NodeDef { id: "on_start", title: "On Start", category: "Events", ins: &[], outs: THEN, params: &[], defaults: &[] },
    NodeDef { id: "on_update", title: "On Update", category: "Events", ins: &[], outs: THEN, params: &[], defaults: &[] },
    NodeDef { id: "on_key", title: "On Key Pressed", category: "Events", ins: &[], outs: THEN, params: &[text("key", "space")], defaults: &[] },
    NodeDef { id: "on_touch", title: "On Touch", category: "Events", ins: &[], outs: THEN, params: &[], defaults: &[] },
    NodeDef { id: "on_leave", title: "On Leave", category: "Events", ins: &[], outs: THEN, params: &[], defaults: &[] },
    NodeDef { id: "on_every", title: "Every N Seconds", category: "Events", ins: &[], outs: THEN, params: &[text("seconds", "1")], defaults: &[] },
    // ---- flow
    NodeDef { id: "if", title: "If", category: "Flow", ins: &[exec("in"), data("condition")], outs: &[exec("true"), exec("false"), exec("after")], params: &[], defaults: &["", "true"] },
    // ---- actions
    NodeDef { id: "set_prop", title: "Set Property", category: "Actions", ins: &[exec("in"), data("value")], outs: OUT_THEN, params: &[object("object"), choice("property", PROPS)], defaults: &["", "0"] },
    NodeDef { id: "add_prop", title: "Change Property By", category: "Actions", ins: &[exec("in"), data("amount")], outs: OUT_THEN, params: &[object("object"), choice("property", PROPS)], defaults: &["", "1"] },
    NodeDef { id: "move", title: "Move", category: "Actions", ins: &[exec("in"), data("x"), data("y"), data("z")], outs: OUT_THEN, params: &[], defaults: &["", "0", "0", "0"] },
    NodeDef { id: "spin", title: "Spin", category: "Actions", ins: &[exec("in"), data("x"), data("y"), data("z")], outs: OUT_THEN, params: &[], defaults: &["", "0", "90", "0"] },
    NodeDef { id: "print", title: "Print", category: "Actions", ins: &[exec("in"), data("text")], outs: OUT_THEN, params: &[], defaults: &["", "\"hello\""] },
    NodeDef { id: "launch", title: "Launch Player", category: "Actions", ins: &[exec("in"), data("height")], outs: OUT_THEN, params: &[], defaults: &["", "12"] },
    NodeDef { id: "sound", title: "Play Sound", category: "Actions", ins: IN_THEN, outs: OUT_THEN, params: &[text("file", "sound.wav")], defaults: &[""] },
    NodeDef { id: "set_var", title: "Set Variable", category: "Actions", ins: &[exec("in"), data("value")], outs: OUT_THEN, params: &[text("name", "score")], defaults: &["", "0"] },
    NodeDef { id: "gui_text", title: "Set GUI Text", category: "Actions", ins: &[exec("in"), data("text")], outs: OUT_THEN, params: &[text("element", "Score")], defaults: &["", "\"text\""] },
    NodeDef { id: "gui_value", title: "Set GUI Bar", category: "Actions", ins: &[exec("in"), data("value")], outs: OUT_THEN, params: &[text("element", "Health")], defaults: &["", "0.5"] },
    NodeDef { id: "gui_show", title: "Show / Hide GUI", category: "Actions", ins: &[exec("in"), data("visible")], outs: OUT_THEN, params: &[text("element", "Menu")], defaults: &["", "true"] },
    NodeDef { id: "hud", title: "Show HUD Text", category: "Actions", ins: &[exec("in"), data("text")], outs: OUT_THEN, params: &[], defaults: &["", "\"hello\""] },
    // ---- values
    NodeDef { id: "number", title: "Number", category: "Values", ins: &[], outs: VALUE, params: &[text("value", "0")], defaults: &[] },
    NodeDef { id: "text", title: "Text", category: "Values", ins: &[], outs: VALUE, params: &[text("text", "hello")], defaults: &[] },
    NodeDef { id: "bool", title: "True / False", category: "Values", ins: &[], outs: VALUE, params: &[choice("value", BOOLS)], defaults: &[] },
    NodeDef { id: "get_prop", title: "Get Property", category: "Values", ins: &[], outs: VALUE, params: &[object("object"), choice("property", PROPS)], defaults: &[] },
    NodeDef { id: "get_var", title: "Get Variable", category: "Values", ins: &[], outs: VALUE, params: &[text("name", "score")], defaults: &[] },
    NodeDef { id: "time", title: "Time", category: "Values", ins: &[], outs: VALUE, params: &[], defaults: &[] },
    NodeDef { id: "dt", title: "Delta Time", category: "Values", ins: &[], outs: VALUE, params: &[], defaults: &[] },
    NodeDef { id: "key_held", title: "Key Is Held", category: "Values", ins: &[], outs: VALUE, params: &[text("key", "w")], defaults: &[] },
    NodeDef { id: "key_pressed", title: "Key Just Pressed", category: "Values", ins: &[], outs: VALUE, params: &[text("key", "e")], defaults: &[] },
    NodeDef { id: "distance", title: "Distance Between", category: "Values", ins: &[], outs: VALUE, params: &[object("from"), object("to")], defaults: &[] },
    NodeDef { id: "random", title: "Random", category: "Values", ins: &[data("min"), data("max")], outs: VALUE, params: &[], defaults: &["0", "1"] },
    NodeDef { id: "clicked", title: "GUI Button Clicked", category: "Values", ins: &[], outs: VALUE, params: &[text("button", "Menu")], defaults: &[] },
    // ---- math and logic
    NodeDef { id: "math", title: "Math", category: "Math", ins: &[data("a"), data("b")], outs: VALUE, params: &[choice("op", MATH_OPS)], defaults: &["0", "0"] },
    NodeDef { id: "func", title: "Function", category: "Math", ins: &[data("a")], outs: VALUE, params: &[choice("fn", FUNCS)], defaults: &["0"] },
    NodeDef { id: "compare", title: "Compare", category: "Logic", ins: &[data("a"), data("b")], outs: VALUE, params: &[choice("op", COMPARE_OPS)], defaults: &["0", "0"] },
    NodeDef { id: "logic", title: "And / Or", category: "Logic", ins: &[data("a"), data("b")], outs: VALUE, params: &[choice("op", LOGIC_OPS)], defaults: &["true", "true"] },
    NodeDef { id: "not", title: "Not", category: "Logic", ins: &[data("a")], outs: VALUE, params: &[], defaults: &["true"] },
];

pub fn def(kind: &str) -> Option<&'static NodeDef> {
    DEFS.iter().find(|d| d.id == kind)
}

/// The categories in menu order.
pub fn categories() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for d in DEFS {
        if !v.contains(&d.category) {
            v.push(d.category);
        }
    }
    v
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: u32,
    pub kind: String,
    pub x: f32,
    pub y: f32,
    /// Typed-in value of every input (used while the input has no wire).
    pub values: Vec<String>,
    pub params: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wire {
    pub from: u32,
    pub from_pin: usize,
    pub to: u32,
    pub to_pin: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub wires: Vec<Wire>,
    next_id: u32,
}

impl Graph {
    /// A graph that starts with an "On Start" and an "On Update" block.
    pub fn starter() -> Graph {
        let mut g = Graph::default();
        g.add_node("on_start", 40.0, 40.0);
        g.add_node("on_update", 40.0, 220.0);
        g
    }

    pub fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: u32) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn add_node(&mut self, kind: &str, x: f32, y: f32) -> Option<u32> {
        let d = def(kind)?;
        self.next_id = self.next_id.max(self.nodes.iter().map(|n| n.id + 1).max().unwrap_or(0));
        let id = self.next_id;
        self.next_id += 1;
        let values = d.ins.iter().enumerate().map(|(i, _)| d.defaults.get(i).copied().unwrap_or("").to_string()).collect();
        self.nodes.push(Node { id, kind: kind.to_string(), x, y, values, params: d.params.iter().map(|p| p.default.to_string()).collect() });
        Some(id)
    }

    pub fn remove_node(&mut self, id: u32) {
        self.nodes.retain(|n| n.id != id);
        self.wires.retain(|w| w.from != id && w.to != id);
    }

    /// A copy of a node placed a little to the side (its wires are not copied).
    pub fn duplicate(&mut self, id: u32) -> Option<u32> {
        let n = self.node(id)?.clone();
        let new = self.add_node(&n.kind, n.x + 30.0, n.y + 30.0)?;
        let c = self.node_mut(new)?;
        c.values = n.values;
        c.params = n.params;
        Some(new)
    }

    fn pin_kind(&self, id: u32, out: bool, pin: usize) -> Option<PinKind> {
        let d = def(&self.node(id)?.kind)?;
        let pins = if out { d.outs } else { d.ins };
        pins.get(pin).map(|p| p.kind)
    }

    /// Joins an output pin to an input pin. Exec to exec, value to value; an input takes one wire (the new
    /// one replaces the old), an exec output leads to one place. Returns false for a connection that makes no sense.
    pub fn connect(&mut self, from: u32, from_pin: usize, to: u32, to_pin: usize) -> bool {
        let (Some(a), Some(b)) = (self.pin_kind(from, true, from_pin), self.pin_kind(to, false, to_pin)) else { return false };
        if a != b || from == to {
            return false;
        }
        self.wires.retain(|w| !(w.to == to && w.to_pin == to_pin));
        if a == PinKind::Exec {
            self.wires.retain(|w| !(w.from == from && w.from_pin == from_pin));
        }
        if self.wires.contains(&Wire { from, from_pin, to, to_pin }) {
            return true;
        }
        self.wires.push(Wire { from, from_pin, to, to_pin });
        true
    }

    pub fn disconnect_input(&mut self, to: u32, to_pin: usize) {
        self.wires.retain(|w| !(w.to == to && w.to_pin == to_pin));
    }

    pub fn disconnect_output(&mut self, from: u32, from_pin: usize) {
        self.wires.retain(|w| !(w.from == from && w.from_pin == from_pin));
    }

    pub fn source_of(&self, to: u32, to_pin: usize) -> Option<(u32, usize)> {
        self.wires.iter().find(|w| w.to == to && w.to_pin == to_pin).map(|w| (w.from, w.from_pin))
    }

    fn next_exec(&self, from: u32, pin: usize) -> Option<u32> {
        self.wires.iter().find(|w| w.from == from && w.from_pin == pin).map(|w| w.to)
    }

    // ------------------------------------------------------------------------------------------
    // compiling to GScript
    // ------------------------------------------------------------------------------------------
    pub fn compile(&self) -> Result<String, String> {
        let mut out = String::from("// Generated by the visual script editor. Edit the graph, or press Convert to code to take over.\n");
        let mut vars: BTreeSet<String> = BTreeSet::new();
        for n in &self.nodes {
            if matches!(n.kind.as_str(), "set_var" | "get_var") {
                let name = n.params.first().map(|s| s.trim().to_string()).unwrap_or_default();
                if !is_ident(&name) {
                    return Err(format!("\"{name}\" is not a good variable name (letters, digits and _, not starting with a digit)"));
                }
                vars.insert(name);
            }
        }
        for v in &vars {
            out += &format!("var {v} = 0\n");
        }
        let mut events: Vec<&Node> = self.nodes.iter().filter(|n| n.kind.starts_with("on_")).collect();
        events.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
        for e in events {
            let header = match e.kind.as_str() {
                "on_start" => "on start".to_string(),
                "on_update" => "on update".to_string(),
                "on_touch" => "on touch".to_string(),
                "on_leave" => "on leave".to_string(),
                "on_key" => format!("on key({})", quote(e.params.first().map_or("", |s| s.trim()))),
                _ => format!("on every({})", number_or(e.params.first().map_or("", |s| s), "1")),
            };
            out += &format!("\n{header} {{\n");
            let mut steps = 0;
            self.chain(self.next_exec(e.id, 0), 1, &mut steps, &mut out)?;
            out += "}\n";
        }
        Ok(out)
    }

    fn chain(&self, mut cur: Option<u32>, depth: usize, steps: &mut usize, out: &mut String) -> Result<(), String> {
        let pad = "    ".repeat(depth);
        while let Some(id) = cur {
            *steps += 1;
            if *steps > 500 || depth > 24 {
                return Err("the order wires loop back on themselves".into());
            }
            let n = self.node(id).ok_or("a wire points at a missing block")?;
            if n.kind == "if" {
                let cond = self.input(id, 1, 0)?;
                *out += &format!("{pad}if {cond} {{\n");
                self.chain(self.next_exec(id, 0), depth + 1, steps, out)?;
                if let Some(f) = self.next_exec(id, 1) {
                    *out += &format!("{pad}}} else {{\n");
                    self.chain(Some(f), depth + 1, steps, out)?;
                }
                *out += &format!("{pad}}}\n");
                cur = self.next_exec(id, 2);
                continue;
            }
            if let Some(line) = self.statement(n)? {
                *out += &format!("{pad}{line}\n");
            }
            cur = self.next_exec(id, 0);
        }
        Ok(())
    }

    fn statement(&self, n: &Node) -> Result<Option<String>, String> {
        let p = |i: usize| n.params.get(i).map(|s| s.trim()).unwrap_or("");
        let target = |name: &str, prop: &str| if is_self(name) { prop.to_string() } else { format!("{}.{prop}", object_expr(name)) };
        Ok(Some(match n.kind.as_str() {
            "set_prop" => format!("{} = {}", target(p(0), p(1)), self.input(n.id, 1, 0)?),
            "add_prop" => format!("{} += {}", target(p(0), p(1)), self.input(n.id, 1, 0)?),
            "move" => format!("move({}, {}, {})", self.input(n.id, 1, 0)?, self.input(n.id, 2, 0)?, self.input(n.id, 3, 0)?),
            "spin" => format!("spin({}, {}, {})", self.input(n.id, 1, 0)?, self.input(n.id, 2, 0)?, self.input(n.id, 3, 0)?),
            "print" => format!("print({})", self.input(n.id, 1, 0)?),
            "hud" => format!("hud({})", self.input(n.id, 1, 0)?),
            "launch" => format!("launch({})", self.input(n.id, 1, 0)?),
            "sound" => format!("sound({})", quote(p(0))),
            "set_var" => format!("{} = {}", p(0), self.input(n.id, 1, 0)?),
            "gui_text" => format!("gui_text({}, {})", quote(p(0)), self.input(n.id, 1, 0)?),
            "gui_value" => format!("gui_value({}, {})", quote(p(0)), self.input(n.id, 1, 0)?),
            "gui_show" => format!("gui_show({}, {})", quote(p(0)), self.input(n.id, 1, 0)?),
            _ => return Ok(None),
        }))
    }

    /// The expression for input `pin` of node `id`: whatever is wired to it, else what was typed in.
    fn input(&self, id: u32, pin: usize, depth: usize) -> Result<String, String> {
        if depth > 32 {
            return Err("the value wires loop back on themselves".into());
        }
        if let Some((from, _)) = self.source_of(id, pin) {
            return self.value(from, depth + 1);
        }
        let n = self.node(id).ok_or("a wire points at a missing block")?;
        let v = n.values.get(pin).map(|s| s.trim()).unwrap_or("");
        Ok(if v.is_empty() { "0".to_string() } else { v.to_string() })
    }

    fn value(&self, id: u32, depth: usize) -> Result<String, String> {
        let n = self.node(id).ok_or("a wire points at a missing block")?;
        let p = |i: usize| n.params.get(i).map(|s| s.trim()).unwrap_or("");
        let inp = |pin: usize| self.input(id, pin, depth);
        Ok(match n.kind.as_str() {
            "number" => number_or(p(0), "0"),
            "text" => quote(n.params.first().map_or("", |s| s)),
            "bool" => if p(0) == "false" { "false".into() } else { "true".into() },
            "get_prop" => {
                if is_self(p(0)) { p(1).to_string() } else { format!("{}.{}", object_expr(p(0)), p(1)) }
            }
            "get_var" => p(0).to_string(),
            "time" => "time".into(),
            "dt" => "dt".into(),
            "key_held" => format!("key({})", quote(p(0))),
            "key_pressed" => format!("pressed({})", quote(p(0))),
            "distance" => format!("distance({}, {})", object_expr(p(0)), object_expr(p(1))),
            "clicked" => format!("clicked({})", quote(p(0))),
            "random" => format!("random({}, {})", inp(0)?, inp(1)?),
            "math" => match p(0) {
                op @ ("min" | "max" | "pow") => format!("{op}({}, {})", inp(0)?, inp(1)?),
                op => format!("({} {op} {})", inp(0)?, inp(1)?),
            },
            "func" => format!("{}({})", p(0), inp(0)?),
            "compare" => format!("({} {} {})", inp(0)?, p(0), inp(1)?),
            "logic" => format!("({} {} {})", inp(0)?, p(0), inp(1)?),
            "not" => format!("(not {})", inp(0)?),
            other => return Err(format!("the block \"{other}\" has no value")),
        })
    }

    // ------------------------------------------------------------------------------------------
    // saving
    // ------------------------------------------------------------------------------------------
    pub fn to_json(&self) -> J {
        let nodes: Vec<J> = self
            .nodes
            .iter()
            .map(|n| {
                J::obj()
                    .with("id", n.id as usize)
                    .with("kind", n.kind.as_str())
                    .with("x", n.x)
                    .with("y", n.y)
                    .with("values", n.values.iter().map(|v| J::from(v.as_str())).collect::<Vec<_>>())
                    .with("params", n.params.iter().map(|v| J::from(v.as_str())).collect::<Vec<_>>())
            })
            .collect();
        let wires: Vec<J> = self.wires.iter().map(|w| J::Arr(vec![J::from(w.from as usize), J::from(w.from_pin), J::from(w.to as usize), J::from(w.to_pin)])).collect();
        J::obj().with("nodes", nodes).with("wires", wires)
    }

    /// Reads a saved graph; unknown blocks are dropped and wires that no longer fit are ignored.
    pub fn from_json(j: &J) -> Graph {
        let mut g = Graph::default();
        let strings = |v: Option<&J>| -> Vec<String> { v.and_then(|a| a.as_arr()).map(|a| a.iter().map(|s| if let J::Str(s) = s { s.clone() } else { String::new() }).collect()).unwrap_or_default() };
        for n in j.get("nodes").and_then(|a| a.as_arr()).unwrap_or(&[]) {
            let (Some(kind), Some(id)) = (n.str("kind"), n.num("id")) else { continue };
            let Some(d) = def(kind) else { continue };
            let mut values = strings(n.get("values"));
            values.resize(d.ins.len(), String::new());
            let mut params = strings(n.get("params"));
            for i in params.len()..d.params.len() {
                params.push(d.params[i].default.to_string());
            }
            params.truncate(d.params.len());
            g.nodes.push(Node { id: id as u32, kind: kind.to_string(), x: n.num("x").unwrap_or(0.0) as f32, y: n.num("y").unwrap_or(0.0) as f32, values, params });
        }
        for w in j.get("wires").and_then(|a| a.as_arr()).unwrap_or(&[]) {
            if let Some([a, b, c, d]) = w.as_arr().map(|a| a.iter().map(|v| if let J::Num(n) = v { *n as usize } else { usize::MAX }).collect::<Vec<_>>()).as_deref() {
                g.connect(*a as u32, *b, *c as u32, *d);
            }
        }
        g.next_id = g.nodes.iter().map(|n| n.id + 1).max().unwrap_or(0);
        g
    }
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_self(name: &str) -> bool {
    let n = name.trim();
    n.is_empty() || n.eq_ignore_ascii_case("self")
}

/// How a script names an object: `self`, `player` or `object("Name")`.
fn object_expr(name: &str) -> String {
    let n = name.trim();
    if is_self(n) {
        "self".into()
    } else if n.eq_ignore_ascii_case("player") {
        "player".into()
    } else {
        format!("object({})", quote(n))
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " "))
}

/// A typed number, or the fallback when it is empty.
fn number_or(s: &str, fallback: &str) -> String {
    let t = s.trim();
    if t.is_empty() { fallback.to_string() } else { t.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(kind: &str, outs: bool, name: &str) -> usize {
        let d = def(kind).unwrap();
        (if outs { d.outs } else { d.ins }).iter().position(|p| p.name == name).unwrap()
    }

    #[test]
    fn every_block_is_well_formed() {
        for d in DEFS {
            assert_eq!(d.defaults.len(), d.ins.len(), "{}: one default per input", d.id);
            assert!(DEFS.iter().filter(|o| o.id == d.id).count() == 1, "{} is listed twice", d.id);
        }
        assert!(categories().contains(&"Events") && categories().contains(&"Math"));
    }

    #[test]
    fn a_graph_compiles_to_a_valid_script() {
        let mut g = Graph::default();
        let upd = g.add_node("on_update", 0.0, 0.0).unwrap();
        let set = g.add_node("set_prop", 200.0, 0.0).unwrap();
        g.node_mut(set).unwrap().params = vec!["".into(), "position.y".into()];
        let t = g.add_node("time", 0.0, 100.0).unwrap();
        let f = g.add_node("func", 100.0, 100.0).unwrap();
        g.node_mut(f).unwrap().params = vec!["sin".into()];
        assert!(g.connect(upd, 0, set, 0));
        assert!(g.connect(t, 0, f, 0));
        assert!(g.connect(f, 0, set, pin("set_prop", false, "value")));
        let code = g.compile().unwrap();
        assert!(code.contains("on update {\n    position.y = sin(time)\n}"), "{code}");
        crate::script::check(&code).unwrap();
    }

    #[test]
    fn branches_variables_and_other_objects() {
        let mut g = Graph::default();
        let key = g.add_node("on_key", 0.0, 0.0).unwrap();
        g.node_mut(key).unwrap().params = vec!["e".into()];
        let iff = g.add_node("if", 100.0, 0.0).unwrap();
        let near = g.add_node("compare", 0.0, 100.0).unwrap();
        let dist = g.add_node("distance", 0.0, 200.0).unwrap();
        g.node_mut(dist).unwrap().params = vec!["Door".into(), "player".into()];
        g.node_mut(near).unwrap().params = vec!["<".into()];
        g.node_mut(near).unwrap().values = vec!["".into(), "3".into()];
        let open = g.add_node("set_prop", 300.0, 0.0).unwrap();
        g.node_mut(open).unwrap().params = vec!["Door".into(), "position.y".into()];
        g.node_mut(open).unwrap().values = vec!["".into(), "3".into()];
        let count = g.add_node("set_var", 300.0, 100.0).unwrap();
        g.node_mut(count).unwrap().params = vec!["opened".into()];
        g.node_mut(count).unwrap().values = vec!["".into(), "1".into()];
        g.connect(key, 0, iff, 0);
        g.connect(dist, 0, near, 0);
        g.connect(near, 0, iff, 1);
        g.connect(iff, 0, open, 0);
        g.connect(open, 0, count, 0);
        let code = g.compile().unwrap();
        assert!(code.contains("var opened = 0"), "{code}");
        assert!(code.contains("on key(\"e\") {\n    if (distance(object(\"Door\"), player) < 3) {\n        object(\"Door\").position.y = 3\n        opened = 1\n    }\n}"), "{code}");
        crate::script::check(&code).unwrap();
    }

    #[test]
    fn connections_follow_the_rules() {
        let mut g = Graph::default();
        let start = g.add_node("on_start", 0.0, 0.0).unwrap();
        let a = g.add_node("print", 0.0, 0.0).unwrap();
        let b = g.add_node("print", 0.0, 0.0).unwrap();
        let n = g.add_node("number", 0.0, 0.0).unwrap();
        assert!(!g.connect(start, 0, a, 1), "exec to a value input is refused");
        assert!(!g.connect(n, 0, a, 0), "a value to an exec input is refused");
        assert!(!g.connect(a, 0, a, 0), "no wire onto itself");
        assert!(g.connect(start, 0, a, 0));
        assert!(g.connect(start, 0, b, 0), "an exec output leads to one place: the new wire replaces the old");
        assert_eq!(g.wires.len(), 1);
        assert!(g.connect(n, 0, a, 1) && g.connect(n, 0, b, 1), "a value can feed several inputs");
        g.remove_node(n);
        assert_eq!(g.wires.len(), 1, "wires go with their block");
    }

    #[test]
    fn loops_and_bad_names_are_reported() {
        // wires that feed each other's values can never be worked out
        let mut g = Graph::default();
        let s = g.add_node("on_start", 0.0, 0.0).unwrap();
        let p = g.add_node("print", 0.0, 0.0).unwrap();
        let m1 = g.add_node("math", 0.0, 0.0).unwrap();
        let m2 = g.add_node("math", 0.0, 0.0).unwrap();
        g.connect(s, 0, p, 0);
        g.connect(m1, 0, p, 1);
        g.connect(m2, 0, m1, 0);
        g.connect(m1, 0, m2, 0);
        assert!(g.compile().unwrap_err().contains("loop"));
        let mut g = Graph::default();
        let v = g.add_node("set_var", 0.0, 0.0).unwrap();
        g.node_mut(v).unwrap().params = vec!["1bad".into()];
        assert!(g.compile().unwrap_err().contains("variable name"));
    }

    #[test]
    fn saving_and_loading_keeps_everything() {
        let mut g = Graph::starter();
        let p = g.add_node("print", 300.0, 20.0).unwrap();
        g.node_mut(p).unwrap().values = vec!["".into(), "\"tab\\there\"".into()];
        g.connect(0, 0, p, 0);
        let back = Graph::from_json(&crate::json::parse(&g.to_json().to_string()).unwrap());
        assert_eq!(back, g);
        assert_eq!(back.compile(), g.compile());
        // stale data: an unknown block and a wire to nowhere are dropped
        let stale = crate::json::parse(r#"{"nodes":[{"id":1,"kind":"nope"},{"id":2,"kind":"on_start","x":1,"y":2}],"wires":[[2,0,1,0]]}"#).unwrap();
        let g2 = Graph::from_json(&stale);
        assert_eq!((g2.nodes.len(), g2.wires.len()), (1, 0));
    }
}
