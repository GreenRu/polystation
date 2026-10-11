//! GScript: the small scripting language for object-based coding. Every object can carry one script;
//! while the game plays, the script's `on start`, `on update` and `on key("x")` blocks run and can read
//! and change the object (and others) through properties like `position.y` or `color.r`.
//!
//! ```text
//! var speed = 90                      // variables of this script (kept between frames)
//!
//! on start {
//!     print("hello from " + self.name)
//! }
//!
//! on update {                         // every frame; `dt` = seconds since the last frame
//!     rotation.y += speed * dt
//!     position.y = 1 + sin(time * 2) * 0.5
//!     if key("space") { color.r = random() }
//! }
//!
//! on key("e") { visible = !visible }  // once when the key goes down
//!
//! fn bump(h) { position.y += h }      // your own functions
//! ```
//!
//! Values are numbers, booleans (`true`/`false`), strings and `nil`. Operators: `+ - * / %`,
//! `== != < <= > >=`, `&& || !` (or `and or not`), assignments `= += -= *= /=`. Statements:
//! `var`, `if / else`, `while`, `for i in 0..10`, `return`, `break`, `continue`.
//!
//! Events: `on start`, `on update`, `on key("x")`, `on touch` / `on leave` (for objects whose
//! collision is Trigger).
//!
//! Objects: `self` (the object with the script), `player`, `object("Name")`. Properties:
//! `position`, `rotation` (degrees), `scale` with `.x .y .z`; `color` with `.r .g .b` (0..1);
//! `alpha`; `visible`; `solid`; `intensity` (lights). On `self` they can be written bare (`position.y`), on others through the
//! object (`object("Door").position.y = 3`). Functions: `sin cos tan atan2 abs sqrt floor ceil round
//! sign pow min max clamp lerp random print key pressed distance object exists`. `time` is the
//! seconds since Play was pressed.
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Prop {
    PosX,
    PosY,
    PosZ,
    RotX,
    RotY,
    RotZ,
    ScaleX,
    ScaleY,
    ScaleZ,
    ColR,
    ColG,
    ColB,
    Alpha,
    Visible,
    /// 1 = the player collides with the object, 0 = it can be walked through.
    Solid,
    /// Brightness of a light object.
    Intensity,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Group {
    Pos,
    Rot,
    Scale,
    Color,
}

impl Group {
    fn axis(self, name: &str) -> Option<Prop> {
        use Prop::*;
        Some(match (self, name) {
            (Group::Pos, "x") => PosX,
            (Group::Pos, "y") => PosY,
            (Group::Pos, "z") => PosZ,
            (Group::Rot, "x") => RotX,
            (Group::Rot, "y") => RotY,
            (Group::Rot, "z") => RotZ,
            (Group::Scale, "x") => ScaleX,
            (Group::Scale, "y") => ScaleY,
            (Group::Scale, "z") => ScaleZ,
            (Group::Color, "r") => ColR,
            (Group::Color, "g") => ColG,
            (Group::Color, "b") => ColB,
            _ => return None,
        })
    }
}

/// What a script can do to the world; implemented by the editor while playing.
pub trait Host {
    fn get(&mut self, obj: &str, p: Prop) -> Result<f64, String>;
    fn set(&mut self, obj: &str, p: Prop, v: f64) -> Result<(), String>;
    fn exists(&self, obj: &str) -> bool;
    fn key_held(&self, name: &str) -> bool;
    fn key_pressed(&self, name: &str) -> bool;
    fn time(&self) -> f64;
    fn print(&mut self, text: &str);
    /// Values shared by every script (set / get / add).
    fn var_get(&self, name: &str) -> Option<Value>;
    fn var_set(&mut self, name: &str, v: Value);
    /// Throws the player up so the jump reaches `height` units.
    fn launch(&mut self, height: f64);
    /// Text that stays visible while playing (the status bar).
    fn hud(&mut self, text: &str);
    /// Plays a .wav file (relative paths are looked up next to the project).
    fn sound(&mut self, path: &str);
    /// The text of a GUI element.
    fn gui_text(&self, _name: &str) -> Option<String> {
        None
    }
    fn gui_set_text(&mut self, _name: &str, _text: &str) -> Result<(), String> {
        Err("this host has no GUI".into())
    }
    /// A number of a GUI element: value, visible, x, y, w, h, alpha, r, g, b.
    fn gui_get(&self, _name: &str, _field: &str) -> Result<f64, String> {
        Err("this host has no GUI".into())
    }
    fn gui_set(&mut self, _name: &str, _field: &str, _v: f64) -> Result<(), String> {
        Err("this host has no GUI".into())
    }
    /// A GUI button was clicked this frame.
    fn gui_clicked(&self, _name: &str) -> bool {
        false
    }
    /// Frees the mouse cursor (true: the mouse clicks GUI buttons) or locks it for looking around again.
    fn set_cursor(&mut self, _free: Option<bool>) {}
    /// Changes how the mouse works (see the GUI editor's modes).
    fn set_gui_mode(&mut self, _mode: &str) -> Result<(), String> {
        Err("this host has no GUI".into())
    }
    /// The object this one is a child of (None at the top level).
    fn parent_of(&self, _obj: &str) -> Option<String> {
        None
    }
    /// The objects directly below this one, in the explorer's order.
    fn children_of(&self, _obj: &str) -> Vec<String> {
        Vec::new()
    }
    /// Makes `child` a child of `parent` (None = top level).
    fn set_parent(&mut self, _child: &str, _parent: Option<&str>) -> Result<(), String> {
        Err("this host has no hierarchy".into())
    }
}

// ------------------------------------------------------------------------------------------
// Lexer
// ------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Sym(&'static str),
    Eof,
}

struct Token {
    tok: Tok,
    line: usize,
}

const SYMS: [&str; 26] = ["..", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "{", "}", "(", ")", ",", ".", "+", "-", "*", "/", "%", "=", "<", ">", "!"];

fn lex(src: &str) -> Result<Vec<Token>, String> {
    let c: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut out) = (0, 1, Vec::new());
    while i < c.len() {
        let ch = c[i];
        if ch == '\n' {
            line += 1;
            i += 1;
        } else if ch.is_whitespace() || ch == ';' {
            i += 1;
        } else if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '"' {
            let mut s = String::new();
            i += 1;
            loop {
                match c.get(i) {
                    None | Some('\n') => return Err(format!("line {line}: unfinished text (missing \")")),
                    Some('"') => break,
                    Some('\\') => {
                        i += 1;
                        s.push(match c.get(i) {
                            Some('n') => '\n',
                            Some('t') => '\t',
                            Some(&o) => o,
                            None => '\\',
                        });
                    }
                    Some(&o) => s.push(o),
                }
                i += 1;
            }
            i += 1;
            out.push(Token { tok: Tok::Str(s), line });
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < c.len() && (c[i].is_ascii_digit() || (c[i] == '.' && c.get(i + 1) != Some(&'.') && c[start..i].iter().filter(|&&x| x == '.').count() == 0)) {
                i += 1;
            }
            let text: String = c[start..i].iter().collect();
            out.push(Token { tok: Tok::Num(text.parse().map_err(|_| format!("line {line}: bad number {text}"))?), line });
        } else if ch.is_alphabetic() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Token { tok: Tok::Ident(c[start..i].iter().collect()), line });
        } else {
            let rest: String = c[i..(i + 2).min(c.len())].iter().collect();
            let sym = SYMS.iter().find(|s| rest.starts_with(**s)).ok_or_else(|| format!("line {line}: unexpected character '{ch}'"))?;
            out.push(Token { tok: Tok::Sym(sym), line });
            i += sym.len();
        }
    }
    out.push(Token { tok: Tok::Eof, line });
    Ok(out)
}

// ------------------------------------------------------------------------------------------
// Syntax tree + parser
// ------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    Str(String),
    Bool(bool),
    Nil,
    Var(String),
    Member(Box<Expr>, String),
    Call(String, Vec<Expr>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug)]
struct Stmt {
    line: usize,
    kind: Kind,
}

#[derive(Clone, Debug)]
enum Kind {
    Var(String, Expr),
    Assign(Expr, &'static str, Expr),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    For(String, Expr, Expr, Vec<Stmt>),
    Return(Option<Expr>),
    Break,
    Continue,
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
enum Event {
    Start,
    Update,
    Key(String),
    Touch,
    Leave,
    /// Every this many seconds.
    Every(f64),
}

#[derive(Clone, Debug)]
struct Function {
    params: Vec<String>,
    body: Vec<Stmt>,
}

#[derive(Clone, Debug, Default)]
struct Program {
    vars: Vec<(usize, String, Expr)>,
    fns: HashMap<String, Function>,
    handlers: Vec<(Event, Vec<Stmt>)>,
}

struct Parser {
    t: Vec<Token>,
    p: usize,
}

type PResult<T> = Result<T, String>;

impl Parser {
    fn peek(&self) -> &Tok {
        &self.t[self.p].tok
    }

    fn line(&self) -> usize {
        self.t[self.p].line
    }

    fn next(&mut self) -> Tok {
        let t = self.t[self.p].tok.clone();
        if self.p + 1 < self.t.len() {
            self.p += 1;
        }
        t
    }

    fn describe(t: &Tok) -> String {
        match t {
            Tok::Num(n) => format!("{n}"),
            Tok::Str(s) => format!("\"{s}\""),
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Sym(s) => format!("`{s}`"),
            Tok::Eof => "the end of the script".into(),
        }
    }

    fn err<T>(&self, what: &str) -> PResult<T> {
        Err(format!("line {}: expected {what}, found {}", self.line(), Self::describe(self.peek())))
    }

    fn sym(&mut self, s: &'static str) -> bool {
        if *self.peek() == Tok::Sym(s) {
            self.p += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &'static str) -> PResult<()> {
        if self.sym(s) { Ok(()) } else { self.err(&format!("`{s}`")) }
    }

    fn kw(&mut self, k: &str) -> bool {
        if matches!(self.peek(), Tok::Ident(i) if i == k) {
            self.p += 1;
            true
        } else {
            false
        }
    }

    fn ident(&mut self, what: &str) -> PResult<String> {
        match self.peek().clone() {
            Tok::Ident(i) => {
                self.p += 1;
                Ok(i)
            }
            _ => self.err(what),
        }
    }

    fn program(&mut self) -> PResult<Program> {
        let mut prog = Program::default();
        while *self.peek() != Tok::Eof {
            let line = self.line();
            if self.kw("var") {
                let name = self.ident("a variable name")?;
                self.expect("=")?;
                prog.vars.push((line, name, self.expr()?));
            } else if self.kw("fn") {
                let name = self.ident("a function name")?;
                self.expect("(")?;
                let mut params = Vec::new();
                while !self.sym(")") {
                    params.push(self.ident("a parameter name")?);
                    if !self.sym(",") {
                        self.expect(")")?;
                        break;
                    }
                }
                let body = self.block()?;
                prog.fns.insert(name, Function { params, body });
            } else if self.kw("on") {
                let ev = self.ident("start, update, key, every, touch or leave")?;
                let event = match ev.as_str() {
                    "start" => Event::Start,
                    "update" => Event::Update,
                    "touch" => Event::Touch,
                    "leave" => Event::Leave,
                    "every" => {
                        self.expect("(")?;
                        let Tok::Num(n) = self.next() else {
                            self.p -= 1;
                            return self.err("a number of seconds, like every(2)");
                        };
                        self.expect(")")?;
                        Event::Every(n.max(0.05))
                    }
                    "key" => {
                        self.expect("(")?;
                        let Tok::Str(k) = self.next() else {
                            self.p -= 1;
                            return self.err("a key name in quotes, like \"space\"");
                        };
                        self.expect(")")?;
                        Event::Key(k.to_lowercase())
                    }
                    other => return Err(format!("line {line}: unknown event `{other}` (use start, update, key(\"x\"), every(seconds), touch or leave)")),
                };
                prog.handlers.push((event, self.block()?));
            } else {
                return Err(format!("line {line}: put code inside `on start {{ }}` or `on update {{ }}` (or declare a `var` / `fn`), found {}", Self::describe(self.peek())));
            }
        }
        Ok(prog)
    }

    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect("{")?;
        let mut v = Vec::new();
        while !self.sym("}") {
            if *self.peek() == Tok::Eof {
                return self.err("`}`");
            }
            v.push(self.stmt()?);
        }
        Ok(v)
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let line = self.line();
        let kind = if self.kw("var") {
            let name = self.ident("a variable name")?;
            self.expect("=")?;
            Kind::Var(name, self.expr()?)
        } else if self.kw("if") {
            let c = self.expr()?;
            let then = self.block()?;
            let mut els = Vec::new();
            if self.kw("else") {
                if matches!(self.peek(), Tok::Ident(i) if i == "if") {
                    els = vec![self.stmt()?];
                } else {
                    els = self.block()?;
                }
            }
            Kind::If(c, then, els)
        } else if self.kw("while") {
            let c = self.expr()?;
            Kind::While(c, self.block()?)
        } else if self.kw("for") {
            let name = self.ident("a loop variable")?;
            if !self.kw("in") {
                return self.err("`in`");
            }
            let a = self.expr()?;
            self.expect("..")?;
            let b = self.expr()?;
            Kind::For(name, a, b, self.block()?)
        } else if self.kw("return") {
            if matches!(self.peek(), Tok::Sym("}") | Tok::Eof) {
                Kind::Return(None)
            } else {
                Kind::Return(Some(self.expr()?))
            }
        } else if self.kw("break") {
            Kind::Break
        } else if self.kw("continue") {
            Kind::Continue
        } else {
            let e = self.expr()?;
            let op = ["=", "+=", "-=", "*=", "/="].into_iter().find(|s| *self.peek() == Tok::Sym(s));
            match op {
                Some(op) => {
                    self.p += 1;
                    if !matches!(e, Expr::Var(_) | Expr::Member(..)) {
                        return Err(format!("line {line}: you can only assign to a variable or a property"));
                    }
                    Kind::Assign(e, op, self.expr()?)
                }
                None => Kind::Expr(e),
            }
        };
        Ok(Stmt { line, kind })
    }

    fn expr(&mut self) -> PResult<Expr> {
        self.binary(0)
    }

    fn binary(&mut self, level: usize) -> PResult<Expr> {
        const LEVELS: [&[&str]; 5] = [&["||", "or"], &["&&", "and"], &["==", "!="], &["<", "<=", ">", ">="], &["+", "-"]];
        const HIGH: &[&str] = &["*", "/", "%"];
        let ops: &[&str] = if level < LEVELS.len() { LEVELS[level] } else { HIGH };
        let mut left = if level < LEVELS.len() { self.binary(level + 1)? } else { self.unary()? };
        loop {
            let found = match self.peek() {
                Tok::Sym(s) if ops.contains(s) => Some(*s),
                Tok::Ident(i) if ops.contains(&i.as_str()) => Some(if i == "or" { "||" } else { "&&" }),
                _ => None,
            };
            let Some(op) = found else { return Ok(left) };
            self.p += 1;
            let right = if level < LEVELS.len() { self.binary(level + 1)? } else { self.unary()? };
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
    }

    fn unary(&mut self) -> PResult<Expr> {
        if self.sym("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.sym("!") || self.kw("not") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        while self.sym(".") {
            let name = self.ident("a property name after `.`")?;
            e = Expr::Member(Box::new(e), name);
        }
        Ok(e)
    }

    fn primary(&mut self) -> PResult<Expr> {
        match self.peek().clone() {
            Tok::Num(n) => {
                self.p += 1;
                Ok(Expr::Num(n))
            }
            Tok::Str(s) => {
                self.p += 1;
                Ok(Expr::Str(s))
            }
            Tok::Sym("(") => {
                self.p += 1;
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Ident(i) => {
                self.p += 1;
                match i.as_str() {
                    "true" => return Ok(Expr::Bool(true)),
                    "false" => return Ok(Expr::Bool(false)),
                    "nil" => return Ok(Expr::Nil),
                    _ => {}
                }
                if self.sym("(") {
                    let mut args = Vec::new();
                    while !self.sym(")") {
                        args.push(self.expr()?);
                        if !self.sym(",") {
                            self.expect(")")?;
                            break;
                        }
                    }
                    Ok(Expr::Call(i, args))
                } else {
                    Ok(Expr::Var(i))
                }
            }
            _ => self.err("a value"),
        }
    }
}

// ------------------------------------------------------------------------------------------
// Interpreter
// ------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(f64),
    Bool(bool),
    Str(String),
    Nil,
    Obj(String),
    Group(String, GroupKind),
}

/// Public alias so the type can appear in `Value` without exposing the private enum's variants.
pub type GroupKind = u8;

fn group_of(k: GroupKind) -> Group {
    match k {
        0 => Group::Pos,
        1 => Group::Rot,
        2 => Group::Scale,
        _ => Group::Color,
    }
}

impl Value {
    fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Nil => false,
            _ => true,
        }
    }

    fn show(&self) -> String {
        match self {
            Value::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", *n as i64) } else { format!("{:.4}", n).trim_end_matches('0').trim_end_matches('.').to_string() }
            }
            Value::Bool(b) => b.to_string(),
            Value::Str(s) => s.clone(),
            Value::Nil => "nil".into(),
            Value::Obj(o) => format!("<object {o}>"),
            Value::Group(o, _) => format!("<property of {o}>"),
        }
    }
}

enum Flow {
    Normal,
    Break,
    Continue,
    Return(Value),
}

const BUDGET: u32 = 200_000;
const MAX_DEPTH: usize = 48;

#[derive(Debug)]
pub struct Script {
    prog: Rc<Program>,
    globals: HashMap<String, Value>,
    rng: u64,
    started: bool,
    /// Seconds collected towards each `on every(..)` block (indexed like the handlers).
    timers: Vec<f64>,
}

struct Run<'a> {
    prog: &'a Program,
    globals: &'a mut HashMap<String, Value>,
    rng: &'a mut u64,
    host: &'a mut dyn Host,
    me: &'a str,
    dt: f64,
    scopes: Vec<HashMap<String, Value>>,
    budget: u32,
    depth: usize,
    line: usize,
}

impl Script {
    pub fn compile(src: &str) -> Result<Script, String> {
        let t = lex(src)?;
        let prog = Parser { t, p: 0 }.program()?;
        let timers = vec![0.0; prog.handlers.len()];
        Ok(Script { prog: Rc::new(prog), globals: HashMap::new(), rng: 0x2545_F491_4F6C_DD1D, started: false, timers })
    }

    /// Runs the `var` initialisers and the `on start` blocks.
    pub fn start(&mut self, me: &str, host: &mut dyn Host) -> Result<(), String> {
        self.started = true;
        let prog = Rc::clone(&self.prog);
        let mut run = Run { prog: &prog, globals: &mut self.globals, rng: &mut self.rng, host, me, dt: 0.0, scopes: vec![HashMap::new()], budget: BUDGET, depth: 0, line: 1 };
        for (line, name, e) in &prog.vars {
            run.line = *line;
            let v = run.eval(e).map_err(|m| format!("line {}: {m}", run.line))?;
            run.globals.insert(name.clone(), v);
        }
        for (ev, body) in &prog.handlers {
            if *ev == Event::Start {
                run.run_body(body)?;
            }
        }
        Ok(())
    }

    /// Runs the `on touch` (or, with `leave`, `on leave`) blocks.
    pub fn touch(&mut self, me: &str, leave: bool, host: &mut dyn Host) -> Result<(), String> {
        let prog = Rc::clone(&self.prog);
        let mut run = Run { prog: &prog, globals: &mut self.globals, rng: &mut self.rng, host, me, dt: 0.0, scopes: vec![HashMap::new()], budget: BUDGET, depth: 0, line: 1 };
        let want = if leave { Event::Leave } else { Event::Touch };
        for (ev, body) in &prog.handlers {
            if *ev == want {
                run.run_body(body)?;
            }
        }
        Ok(())
    }

    pub fn update(&mut self, me: &str, dt: f64, host: &mut dyn Host) -> Result<(), String> {
        let prog = Rc::clone(&self.prog);
        let mut run = Run { prog: &prog, globals: &mut self.globals, rng: &mut self.rng, host, me, dt, scopes: vec![HashMap::new()], budget: BUDGET, depth: 0, line: 1 };
        for (i, (ev, body)) in prog.handlers.iter().enumerate() {
            match ev {
                Event::Update => run.run_body(body)?,
                Event::Key(k) if run.host.key_pressed(k) => run.run_body(body)?,
                Event::Every(secs) => {
                    self.timers[i] += dt;
                    if self.timers[i] >= *secs {
                        self.timers[i] = (self.timers[i] - secs).min(*secs); // never more than one catch-up run
                        run.run_body(body)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl Run<'_> {
    fn run_body(&mut self, body: &[Stmt]) -> Result<(), String> {
        self.scopes = vec![HashMap::new()];
        self.exec_block(body)?;
        Ok(())
    }

    fn exec_block(&mut self, body: &[Stmt]) -> Result<Flow, String> {
        self.scopes.push(HashMap::new());
        let mut result = Ok(Flow::Normal);
        for s in body {
            match self.exec(s) {
                Ok(Flow::Normal) => {}
                other => {
                    result = other;
                    break;
                }
            }
        }
        self.scopes.pop();
        result
    }

    fn tick(&mut self) -> Result<(), String> {
        if self.budget == 0 {
            return Err(format!("line {}: the script ran too long (an endless loop?)", self.line));
        }
        self.budget -= 1;
        Ok(())
    }

    fn fail<T>(&self, msg: impl Into<String>) -> Result<T, String> {
        Err(format!("line {}: {}", self.line, msg.into()))
    }

    fn exec(&mut self, s: &Stmt) -> Result<Flow, String> {
        self.tick()?;
        self.line = s.line;
        match &s.kind {
            Kind::Var(name, e) => {
                let v = self.eval(e)?;
                self.scopes.last_mut().unwrap().insert(name.clone(), v);
            }
            Kind::Assign(target, op, e) => {
                let rhs = self.eval(e)?;
                let new = if *op == "=" {
                    rhs
                } else {
                    let cur = self.eval(target)?;
                    self.arith(&op[..1], cur, rhs)?
                };
                self.assign(target, new)?;
            }
            Kind::If(c, then, els) => {
                let branch = if self.eval(c)?.truthy() { then } else { els };
                return self.exec_block(branch);
            }
            Kind::While(c, body) => {
                while self.eval(c)?.truthy() {
                    self.tick()?;
                    match self.exec_block(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
            }
            Kind::For(name, a, b, body) => {
                let (Value::Num(lo), Value::Num(hi)) = (self.eval(a)?, self.eval(b)?) else { return self.fail("a `for` range needs numbers, like 0..10") };
                let mut i = lo.floor();
                while i < hi {
                    self.tick()?;
                    self.scopes.push(HashMap::from([(name.clone(), Value::Num(i))]));
                    let flow = self.exec_block(body);
                    self.scopes.pop();
                    match flow? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                    i += 1.0;
                }
            }
            Kind::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(e)?,
                    None => Value::Nil,
                };
                return Ok(Flow::Return(v));
            }
            Kind::Break => return Ok(Flow::Break),
            Kind::Continue => return Ok(Flow::Continue),
            Kind::Expr(e) => {
                self.eval(e)?;
            }
        }
        Ok(Flow::Normal)
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        self.scopes.iter().rev().find_map(|s| s.get(name)).or_else(|| self.globals.get(name)).cloned()
    }

    fn me_prop(&mut self, name: &str) -> Option<Result<Value, String>> {
        let me = self.me.to_string();
        Some(match name {
            "self" => Ok(Value::Obj(me)),
            "player" => Ok(Value::Obj("player".into())),
            "position" => Ok(Value::Group(me, 0)),
            "rotation" => Ok(Value::Group(me, 1)),
            "scale" => Ok(Value::Group(me, 2)),
            "color" => Ok(Value::Group(me, 3)),
            "alpha" => self.host.get(&me, Prop::Alpha).map(Value::Num),
            "visible" => self.host.get(&me, Prop::Visible).map(|v| Value::Bool(v != 0.0)),
            "solid" => self.host.get(&me, Prop::Solid).map(|v| Value::Bool(v != 0.0)),
            "intensity" => self.host.get(&me, Prop::Intensity).map(Value::Num),
            "dt" => Ok(Value::Num(self.dt)),
            "time" => Ok(Value::Num(self.host.time())),
            "pi" => Ok(Value::Num(std::f64::consts::PI)),
            _ => return None,
        })
    }

    fn eval(&mut self, e: &Expr) -> Result<Value, String> {
        self.tick()?;
        match e {
            Expr::Num(n) => Ok(Value::Num(*n)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Nil => Ok(Value::Nil),
            Expr::Var(name) => {
                if let Some(v) = self.lookup(name) {
                    return Ok(v);
                }
                match self.me_prop(name) {
                    Some(r) => r.or_else(|m| self.fail(m)),
                    None => self.fail(format!("unknown name `{name}` (declare it with `var {name} = ...`)")),
                }
            }
            Expr::Member(base, name) => {
                let b = self.eval(base)?;
                match (b, name.as_str()) {
                    (Value::Obj(o), "name") => Ok(Value::Str(o)),
                    (Value::Obj(o), "position") => Ok(Value::Group(o, 0)),
                    (Value::Obj(o), "rotation") => Ok(Value::Group(o, 1)),
                    (Value::Obj(o), "scale") => Ok(Value::Group(o, 2)),
                    (Value::Obj(o), "color") => Ok(Value::Group(o, 3)),
                    (Value::Obj(o), "alpha") => self.host.get(&o, Prop::Alpha).map(Value::Num).or_else(|m| self.fail(m)),
                    (Value::Obj(o), "visible") => self.host.get(&o, Prop::Visible).map(|v| Value::Bool(v != 0.0)).or_else(|m| self.fail(m)),
                    (Value::Obj(o), "solid") => self.host.get(&o, Prop::Solid).map(|v| Value::Bool(v != 0.0)).or_else(|m| self.fail(m)),
                    (Value::Obj(o), "intensity") => self.host.get(&o, Prop::Intensity).map(Value::Num).or_else(|m| self.fail(m)),
                    (Value::Obj(o), axis @ ("x" | "y" | "z")) => {
                        let p = Group::Pos.axis(axis).unwrap();
                        self.host.get(&o, p).map(Value::Num).or_else(|m| self.fail(m))
                    }
                    (Value::Group(o, k), axis) => match group_of(k).axis(axis) {
                        Some(p) => self.host.get(&o, p).map(Value::Num).or_else(|m| self.fail(m)),
                        None => self.fail(format!("`.{axis}` is not part of this property (use x, y, z; colors use r, g, b)")),
                    },
                    (_, n) => self.fail(format!("there is no property `{n}` here")),
                }
            }
            Expr::Not(e) => Ok(Value::Bool(!self.eval(e)?.truthy())),
            Expr::Neg(e) => match self.eval(e)? {
                Value::Num(n) => Ok(Value::Num(-n)),
                _ => self.fail("only numbers can be negated"),
            },
            Expr::Bin(op, a, b) => match *op {
                "&&" => {
                    let l = self.eval(a)?;
                    Ok(Value::Bool(l.truthy() && self.eval(b)?.truthy()))
                }
                "||" => {
                    let l = self.eval(a)?;
                    Ok(Value::Bool(l.truthy() || self.eval(b)?.truthy()))
                }
                _ => {
                    let (l, r) = (self.eval(a)?, self.eval(b)?);
                    self.arith(op, l, r)
                }
            },
            Expr::Call(name, args) => {
                let vals = args.iter().map(|a| self.eval(a)).collect::<Result<Vec<_>, _>>()?;
                self.call(name, vals)
            }
        }
    }

    fn arith(&mut self, op: &str, l: Value, r: Value) -> Result<Value, String> {
        use Value::*;
        Ok(match (op, &l, &r) {
            ("+", Str(_), _) | ("+", _, Str(_)) => Str(format!("{}{}", l.show(), r.show())),
            ("==", _, _) => Bool(l == r),
            ("!=", _, _) => Bool(l != r),
            (_, Num(a), Num(b)) => match op {
                "+" => Num(a + b),
                "-" => Num(a - b),
                "*" => Num(a * b),
                "/" => {
                    if *b == 0.0 {
                        return self.fail("division by zero");
                    }
                    Num(a / b)
                }
                "%" => {
                    if *b == 0.0 {
                        return self.fail("division by zero");
                    }
                    Num(a % b)
                }
                "<" => Bool(a < b),
                "<=" => Bool(a <= b),
                ">" => Bool(a > b),
                ">=" => Bool(a >= b),
                _ => return self.fail(format!("unknown operator {op}")),
            },
            _ => return self.fail(format!("`{op}` needs two numbers (got {} and {})", l.show(), r.show())),
        })
    }

    fn assign(&mut self, target: &Expr, v: Value) -> Result<(), String> {
        match target {
            Expr::Var(name) => {
                if let Some(slot) = self.scopes.iter_mut().rev().find_map(|s| s.get_mut(name)) {
                    *slot = v;
                    return Ok(());
                }
                if let Some(slot) = self.globals.get_mut(name) {
                    *slot = v;
                    return Ok(());
                }
                let me = self.me.to_string();
                match name.as_str() {
                    "alpha" => self.set_prop(&me, Prop::Alpha, &v),
                    "visible" => self.set_prop(&me, Prop::Visible, &v),
                    "solid" => self.set_prop(&me, Prop::Solid, &v),
                    "intensity" => self.set_prop(&me, Prop::Intensity, &v),
                    _ => self.fail(format!("unknown variable `{name}` (declare it with `var {name} = ...`)")),
                }
            }
            Expr::Member(base, name) => {
                let b = self.eval(base)?;
                match (b, name.as_str()) {
                    (Value::Obj(o), "alpha") => self.set_prop(&o, Prop::Alpha, &v),
                    (Value::Obj(o), "visible") => self.set_prop(&o, Prop::Visible, &v),
                    (Value::Obj(o), "solid") => self.set_prop(&o, Prop::Solid, &v),
                    (Value::Obj(o), "intensity") => self.set_prop(&o, Prop::Intensity, &v),
                    (Value::Obj(o), "position" | "rotation" | "scale" | "color") => self.fail(format!("set one axis at a time, like {o}.{name}.x = 1 (or use .x on the object)")),
                    (Value::Obj(o), axis @ ("x" | "y" | "z")) => self.set_prop(&o, Group::Pos.axis(axis).unwrap(), &v),
                    (Value::Group(o, k), axis) => match group_of(k).axis(axis) {
                        Some(p) => self.set_prop(&o, p, &v),
                        None => self.fail(format!("`.{axis}` is not part of this property")),
                    },
                    _ => self.fail(format!("cannot assign to `.{name}` here")),
                }
            }
            _ => self.fail("cannot assign to that"),
        }
    }

    fn set_prop(&mut self, obj: &str, p: Prop, v: &Value) -> Result<(), String> {
        let n = match v {
            Value::Num(n) => *n,
            Value::Bool(b) => *b as u8 as f64,
            _ => return self.fail("this property needs a number (or true / false)"),
        };
        self.host.set(obj, p, n).or_else(|m| self.fail(m))
    }

    fn rand(&mut self) -> f64 {
        let mut x = *self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *self.rng = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    fn num(&self, v: &Value, what: &str) -> Result<f64, String> {
        match v {
            Value::Num(n) => Ok(*n),
            _ => self.fail(format!("{what} needs a number")),
        }
    }

    fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String> {
        if let Some(f) = self.prog.fns.get(name) {
            if args.len() != f.params.len() {
                return self.fail(format!("`{name}` takes {} value(s), got {}", f.params.len(), args.len()));
            }
            if self.depth >= MAX_DEPTH {
                return self.fail("functions call each other too deeply");
            }
            let (body, params) = (f.body.clone(), f.params.clone());
            let saved = std::mem::replace(&mut self.scopes, vec![params.into_iter().zip(args).collect()]);
            self.depth += 1;
            let flow = self.exec_block(&body);
            self.depth -= 1;
            self.scopes = saved;
            return Ok(match flow? {
                Flow::Return(v) => v,
                _ => Value::Nil,
            });
        }
        let n = |s: &Self, i: usize| -> Result<f64, String> { s.num(args.get(i).unwrap_or(&Value::Nil), &format!("`{name}`")) };
        let need = |count: usize, s: &Self| -> Result<(), String> {
            if args.len() == count { Ok(()) } else { s.fail(format!("`{name}` takes {count} value(s), got {}", args.len())) }
        };
        Ok(match name {
            "round" if args.len() == 2 => {
                let (x, d) = (n(self, 0)?, n(self, 1)?.clamp(0.0, 8.0));
                let k = 10f64.powi(d as i32);
                Value::Num((x * k).round() / k)
            }
            "sin" | "cos" | "tan" | "abs" | "sqrt" | "floor" | "ceil" | "round" | "sign" => {
                need(1, self)?;
                let x = n(self, 0)?;
                Value::Num(match name {
                    "sin" => x.sin(),
                    "cos" => x.cos(),
                    "tan" => x.tan(),
                    "abs" => x.abs(),
                    "sqrt" => x.max(0.0).sqrt(),
                    "floor" => x.floor(),
                    "ceil" => x.ceil(),
                    "round" => x.round(),
                    _ => if x == 0.0 { 0.0 } else { x.signum() },
                })
            }
            "atan2" | "pow" | "min" | "max" => {
                need(2, self)?;
                let (a, b) = (n(self, 0)?, n(self, 1)?);
                Value::Num(match name {
                    "atan2" => a.atan2(b),
                    "pow" => a.powf(b),
                    "min" => a.min(b),
                    _ => a.max(b),
                })
            }
            "clamp" | "lerp" => {
                need(3, self)?;
                let (a, b, c) = (n(self, 0)?, n(self, 1)?, n(self, 2)?);
                Value::Num(if name == "clamp" { a.clamp(b.min(c), b.max(c)) } else { a + (b - a) * c })
            }
            "set" => {
                need(2, self)?;
                let Value::Str(key) = &args[0] else { return self.fail("`set` needs a name in quotes, like set(\"score\", 5)") };
                if !matches!(args[1], Value::Num(_) | Value::Bool(_) | Value::Str(_)) {
                    return self.fail("`set` can keep numbers, true / false and text");
                }
                self.host.var_set(key, args[1].clone());
                Value::Nil
            }
            "get" => {
                need(1, self)?;
                let Value::Str(key) = &args[0] else { return self.fail("`get` needs a name in quotes, like get(\"score\")") };
                self.host.var_get(key).unwrap_or(Value::Nil)
            }
            "add" => {
                need(2, self)?;
                let Value::Str(key) = &args[0] else { return self.fail("`add` needs a name in quotes, like add(\"score\", 1)") };
                let by = n(self, 1)?;
                let cur = match self.host.var_get(key) {
                    Some(Value::Num(c)) => c,
                    None | Some(Value::Nil) => 0.0,
                    _ => return self.fail(format!("\"{key}\" holds text or true / false, so it cannot be added to")),
                };
                self.host.var_set(key, Value::Num(cur + by));
                Value::Num(cur + by)
            }
            "destroy" => {
                let target = match args.first() {
                    None => self.me.to_string(),
                    Some(Value::Obj(o)) => o.clone(),
                    _ => return self.fail("`destroy` takes nothing (this object) or an object, like destroy(object(\"Door\"))"),
                };
                if target == "player" {
                    return self.fail("the player cannot be destroyed");
                }
                self.host.set(&target, Prop::Visible, 0.0).or_else(|m| self.fail(m))?;
                self.host.set(&target, Prop::Solid, 0.0).or_else(|m| self.fail(m))?;
                Value::Nil
            }
            "launch" => {
                need(1, self)?;
                let h = n(self, 0)?;
                self.host.launch(h);
                Value::Nil
            }
            "hud" => {
                need(1, self)?;
                let text = args[0].show();
                self.host.hud(&text);
                Value::Nil
            }
            "sound" => {
                need(1, self)?;
                let Value::Str(path) = &args[0] else { return self.fail("`sound` needs a .wav file name in quotes, like sound(\"coin.wav\")") };
                self.host.sound(path);
                Value::Nil
            }
            "str" => {
                need(1, self)?;
                Value::Str(args[0].show())
            }
            "move" | "spin" => {
                need(3, self)?;
                let me = self.me.to_string();
                let base = if name == "move" { Prop::PosX } else { Prop::RotX };
                for (k, p) in [base, next_prop(base, 1), next_prop(base, 2)].into_iter().enumerate() {
                    let amount = n(self, k)? * self.dt;
                    if amount != 0.0 {
                        let cur = self.host.get(&me, p).or_else(|m| self.fail(m))?;
                        self.host.set(&me, p, cur + amount).or_else(|m| self.fail(m))?;
                    }
                }
                Value::Nil
            }
            "random" => match args.len() {
                0 => Value::Num(self.rand()),
                2 => {
                    let (a, b) = (n(self, 0)?, n(self, 1)?);
                    Value::Num(a + (b - a) * self.rand())
                }
                _ => return self.fail("`random` takes no values or two (min, max)"),
            },
            "print" => {
                let text: Vec<String> = args.iter().map(|a| a.show()).collect();
                self.host.print(&text.join(" "));
                Value::Nil
            }
            "key" | "pressed" => {
                need(1, self)?;
                let Value::Str(k) = &args[0] else { return self.fail(format!("`{name}` needs a key name in quotes, like \"space\"")) };
                let k = k.to_lowercase();
                Value::Bool(if name == "key" { self.host.key_held(&k) } else { self.host.key_pressed(&k) })
            }
            "object" => {
                need(1, self)?;
                let Value::Str(o) = &args[0] else { return self.fail("`object` needs a name in quotes") };
                if o != "player" && !self.host.exists(o) {
                    return self.fail(format!("there is no object called \"{o}\""));
                }
                Value::Obj(o.clone())
            }
            "gui_text" => {
                let Some(Value::Str(g)) = args.first().cloned() else { return self.fail("`gui_text` needs the element's name in quotes, like gui_text(\"Score\", \"0\")") };
                match args.len() {
                    1 => match self.host.gui_text(&g) {
                        Some(t) => Value::Str(t),
                        None => return self.fail(format!("there is no GUI element called \"{g}\"")),
                    },
                    2 => {
                        let t = args[1].show();
                        if let Err(e) = self.host.gui_set_text(&g, &t) {
                            return self.fail(e);
                        }
                        Value::Nil
                    }
                    _ => return self.fail("`gui_text` takes a name and optionally the new text"),
                }
            }
            "gui_value" => {
                let Some(Value::Str(g)) = args.first().cloned() else { return self.fail("`gui_value` needs the element's name in quotes, like gui_value(\"Health\", 0.5)") };
                match args.len() {
                    1 => Value::Num(self.host.gui_get(&g, "value").or_else(|e| self.fail(e))?),
                    2 => {
                        let v = n(self, 1)?;
                        self.host.gui_set(&g, "value", v).or_else(|e| self.fail(e))?;
                        Value::Nil
                    }
                    _ => return self.fail("`gui_value` takes a name and optionally the new value (0 to 1 for a bar)"),
                }
            }
            "gui_show" => {
                need(2, self)?;
                let Value::Str(g) = &args[0] else { return self.fail("`gui_show` needs the element's name in quotes, like gui_show(\"Menu\", true)") };
                let on = args[1].truthy() as u8 as f64;
                self.host.gui_set(g, "visible", on).or_else(|e| self.fail(e))?;
                Value::Nil
            }
            "gui_move" => {
                need(3, self)?;
                let Value::Str(g) = &args[0] else { return self.fail("`gui_move` needs the element's name in quotes, like gui_move(\"Score\", 20, 40)") };
                let (x, y) = (n(self, 1)?, n(self, 2)?);
                self.host.gui_set(g, "x", x).or_else(|e| self.fail(e))?;
                self.host.gui_set(g, "y", y).or_else(|e| self.fail(e))?;
                Value::Nil
            }
            "gui_color" => {
                need(4, self)?;
                let Value::Str(g) = &args[0] else { return self.fail("`gui_color` needs the element's name in quotes, like gui_color(\"Health\", 1, 0, 0)") };
                let c = [n(self, 1)?, n(self, 2)?, n(self, 3)?];
                for (f, v) in ["r", "g", "b"].into_iter().zip(c) {
                    self.host.gui_set(g, f, v).or_else(|e| self.fail(e))?;
                }
                Value::Nil
            }
            "clicked" => {
                need(1, self)?;
                let Value::Str(g) = &args[0] else { return self.fail("`clicked` needs a button name in quotes, like clicked(\"Start\")") };
                Value::Bool(self.host.gui_clicked(g))
            }
            "cursor" => {
                need(1, self)?;
                match &args[0] {
                    Value::Str(s) if s == "auto" => self.host.set_cursor(None), // back to the GUI mode's rule
                    v => {
                        let free = v.truthy();
                        self.host.set_cursor(Some(free));
                    }
                }
                Value::Nil
            }
            "gui_mode" => {
                need(1, self)?;
                let Value::Str(m) = &args[0] else { return self.fail("`gui_mode` needs a mode in quotes: \"look\", \"cursor\", \"toggle\" or \"hold\"") };
                if let Err(e) = self.host.set_gui_mode(m) {
                    return self.fail(e);
                }
                Value::Nil
            }
            "parent" => {
                need(1, self)?;
                let Value::Obj(o) = &args[0] else { return self.fail("`parent` needs an object, like parent(self)") };
                match self.host.parent_of(o) {
                    Some(p) => Value::Obj(p),
                    None => Value::Nil,
                }
            }
            "children" => {
                need(1, self)?;
                let Value::Obj(o) = &args[0] else { return self.fail("`children` needs an object, like children(self)") };
                Value::Num(self.host.children_of(o).len() as f64)
            }
            "child" => {
                need(2, self)?;
                let (Value::Obj(o), Value::Num(k)) = (&args[0], &args[1]) else { return self.fail("`child` needs an object and a number, like child(self, 1)") };
                let kids = self.host.children_of(o);
                match (*k as i64 - 1).try_into().ok().and_then(|i: usize| kids.get(i)) {
                    Some(c) => Value::Obj(c.clone()),
                    None => Value::Nil,
                }
            }
            "set_parent" => {
                need(2, self)?;
                let Value::Obj(c) = &args[0] else { return self.fail("`set_parent` needs an object first, like set_parent(self, object(\"Door\"))") };
                let parent = match &args[1] {
                    Value::Obj(p) => Some(p.as_str()),
                    Value::Nil => None,
                    _ => return self.fail("`set_parent` needs an object (or nil for no parent) second"),
                };
                if let Err(e) = self.host.set_parent(c, parent) {
                    return self.fail(e);
                }
                Value::Nil
            }
            "exists" => {
                need(1, self)?;
                let Value::Str(o) = &args[0] else { return self.fail("`exists` needs a name in quotes") };
                Value::Bool(o == "player" || self.host.exists(o))
            }
            "distance" => {
                need(2, self)?;
                let (Value::Obj(a), Value::Obj(b)) = (&args[0], &args[1]) else { return self.fail("`distance` needs two objects, like distance(self, player)") };
                let mut d = 0.0;
                for p in [Prop::PosX, Prop::PosY, Prop::PosZ] {
                    let diff = self.host.get(a, p).or_else(|m| self.fail(m))? - self.host.get(b, p).or_else(|m| self.fail(m))?;
                    d += diff * diff;
                }
                Value::Num(d.sqrt())
            }
            _ => return self.fail(format!("unknown function `{name}`")),
        })
    }
}

fn next_prop(base: Prop, k: usize) -> Prop {
    use Prop::*;
    match (base, k) {
        (PosX, 1) => PosY,
        (PosX, _) => PosZ,
        (RotX, 1) => RotY,
        _ => RotZ,
    }
}

/// Checks a script's syntax without running it.
pub fn check(src: &str) -> Result<(), String> {
    Script::compile(src).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Mock {
        props: HashMap<(String, Prop), f64>,
        keys: Vec<String>,
        pressed: Vec<String>,
        log: Vec<String>,
        t: f64,
        vars: HashMap<String, Value>,
        launched: f64,
        hud: String,
        sounds: Vec<String>,
    }

    impl Host for Mock {
        fn get(&mut self, obj: &str, p: Prop) -> Result<f64, String> {
            Ok(*self.props.get(&(obj.to_string(), p)).unwrap_or(&0.0))
        }
        fn set(&mut self, obj: &str, p: Prop, v: f64) -> Result<(), String> {
            self.props.insert((obj.to_string(), p), v);
            Ok(())
        }
        fn exists(&self, obj: &str) -> bool {
            obj == "Door" || obj == "Cube"
        }
        fn key_held(&self, name: &str) -> bool {
            self.keys.iter().any(|k| k == name)
        }
        fn key_pressed(&self, name: &str) -> bool {
            self.pressed.iter().any(|k| k == name)
        }
        fn time(&self) -> f64 {
            self.t
        }
        fn print(&mut self, text: &str) {
            self.log.push(text.to_string());
        }
        fn var_get(&self, name: &str) -> Option<Value> {
            self.vars.get(name).cloned()
        }
        fn var_set(&mut self, name: &str, v: Value) {
            self.vars.insert(name.to_string(), v);
        }
        fn launch(&mut self, height: f64) {
            self.launched = height;
        }
        fn hud(&mut self, text: &str) {
            self.hud = text.to_string();
        }
        fn sound(&mut self, path: &str) {
            self.sounds.push(path.to_string());
        }
        // the scene in these tests: Door contains Cube
        fn parent_of(&self, obj: &str) -> Option<String> {
            (obj == "Cube").then(|| "Door".to_string())
        }
        fn children_of(&self, obj: &str) -> Vec<String> {
            if obj == "Door" { vec!["Cube".to_string()] } else { Vec::new() }
        }
    }

    #[test]
    fn scripts_can_read_the_hierarchy() {
        let h = run("on start {
 print(parent(self), children(object(\"Door\")), child(object(\"Door\"), 1), child(object(\"Door\"), 2), parent(object(\"Door\")))
}", 0);
        assert_eq!(h.log, ["<object Door> 1 <object Cube> nil nil"]);
    }

    fn run(src: &str, frames: usize) -> Mock {
        let mut s = Script::compile(src).unwrap_or_else(|e| panic!("{e}"));
        let mut h = Mock::default();
        s.start("Cube", &mut h).unwrap_or_else(|e| panic!("{e}"));
        for _ in 0..frames {
            s.update("Cube", 0.5, &mut h).unwrap_or_else(|e| panic!("{e}"));
        }
        h
    }

    fn p(h: &Mock, o: &str, prop: Prop) -> f64 {
        *h.props.get(&(o.to_string(), prop)).unwrap_or(&0.0)
    }

    #[test]
    fn spin_and_move_every_frame() {
        let h = run("var speed = 90\non update { rotation.y += speed * dt\n position.x = position.x + 1 }", 4);
        assert_eq!(p(&h, "Cube", Prop::RotY), 180.0);
        assert_eq!(p(&h, "Cube", Prop::PosX), 4.0);
    }

    #[test]
    fn start_runs_once_and_prints() {
        let h = run("on start { print(\"hi\", 1 + 2, self.name) }\non update { alpha = 0.5 }", 2);
        assert_eq!(h.log, vec!["hi 3 Cube"]);
        assert_eq!(p(&h, "Cube", Prop::Alpha), 0.5);
    }

    #[test]
    fn control_flow_loops_and_functions() {
        let src = "fn add(a, b) { return a + b }\non start {\n var total = 0\n for i in 0..5 { if i == 2 { continue } total += i }\n var n = 0\n while true { n += 1 if n >= 7 { break } }\n position.x = add(total, n)\n if total > 100 { position.y = 1 } else if total == 8 { position.y = 2 } else { position.y = 3 }\n}";
        let h = run(src, 0);
        assert_eq!(p(&h, "Cube", Prop::PosX), 15.0); // 0+1+3+4 = 8, plus 7
        assert_eq!(p(&h, "Cube", Prop::PosY), 2.0);
    }

    #[test]
    fn other_objects_keys_and_colors() {
        let mut s = Script::compile("on update { if key(\"w\") { object(\"Door\").position.y += 2 }\n color.g = 0.25\n visible = false }\non key(\"e\") { position.z = distance(self, player) + 10 }").unwrap();
        let mut h = Mock::default();
        h.keys.push("w".into());
        h.pressed.push("e".into());
        h.props.insert(("player".into(), Prop::PosX), 3.0);
        h.props.insert(("player".into(), Prop::PosY), 4.0);
        s.start("Cube", &mut h).unwrap();
        s.update("Cube", 0.1, &mut h).unwrap();
        assert_eq!(p(&h, "Door", Prop::PosY), 2.0);
        assert_eq!(p(&h, "Cube", Prop::ColG), 0.25);
        assert_eq!(p(&h, "Cube", Prop::Visible), 0.0);
        assert_eq!(p(&h, "Cube", Prop::PosZ), 15.0);
    }

    #[test]
    fn script_variables_persist_between_frames() {
        let h = run("var count = 0\non update { count += 1\n position.x = count * 10 }", 3);
        assert_eq!(p(&h, "Cube", Prop::PosX), 30.0);
    }

    #[test]
    fn errors_name_the_line() {
        let e = Script::compile("on update {\n  position.x = (1 +\n}").unwrap_err();
        assert!(e.starts_with("line 3"), "{e}");
        let e = Script::compile("print(1)").unwrap_err();
        assert!(e.contains("line 1") && e.contains("on start"), "{e}");
        let mut s = Script::compile("on update {\n  var a = 1\n  a = a + nothing\n}").unwrap();
        let mut h = Mock::default();
        let e = s.update("Cube", 0.1, &mut h).unwrap_err();
        assert!(e.contains("line 3") && e.contains("nothing"), "{e}");
        let mut s = Script::compile("on update { object(\"Nope\") }").unwrap();
        assert!(s.update("Cube", 0.1, &mut h).unwrap_err().contains("no object called"));
    }

    #[test]
    fn move_spin_touch_and_solid() {
        let mut s = Script::compile("on update { move(2, 0, 0) spin(0, 90, 0) }\non touch { solid = false\n color.r = 1 }\non leave { color.r = 0.5 }").unwrap();
        let mut h = Mock::default();
        s.start("Cube", &mut h).unwrap();
        s.update("Cube", 0.5, &mut h).unwrap();
        assert_eq!(p(&h, "Cube", Prop::PosX), 1.0);
        assert_eq!(p(&h, "Cube", Prop::RotY), 45.0);
        s.touch("Cube", false, &mut h).unwrap();
        assert_eq!((p(&h, "Cube", Prop::Solid), p(&h, "Cube", Prop::ColR)), (0.0, 1.0));
        s.touch("Cube", true, &mut h).unwrap();
        assert_eq!(p(&h, "Cube", Prop::ColR), 0.5);
        let e = Script::compile("on update { position = 3 }").unwrap();
        let mut e = e;
        assert!(e.update("Cube", 0.1, &mut h).is_err());
    }

    #[test]
    fn timers_shared_values_and_helpers() {
        let mut s = Script::compile("on every(1) { add(\"n\", 2) }\non start { set(\"label\", \"hi\") }\non touch { destroy() launch(5) hud(\"Score \" + str(get(\"n\"))) sound(\"a.wav\") print(round(3.14159, 2)) }").unwrap();
        let mut h = Mock::default();
        s.start("Cube", &mut h).unwrap();
        for _ in 0..5 {
            s.update("Cube", 0.5, &mut h).unwrap(); // 2.5 seconds -> the every(1) block ran twice
        }
        assert_eq!(h.vars.get("n"), Some(&Value::Num(4.0)));
        assert_eq!(h.vars.get("label"), Some(&Value::Str("hi".into())));
        s.touch("Cube", false, &mut h).unwrap();
        assert_eq!((p(&h, "Cube", Prop::Visible), p(&h, "Cube", Prop::Solid), h.launched), (0.0, 0.0, 5.0));
        assert_eq!((h.hud.as_str(), h.sounds.as_slice()), ("Score 4", &["a.wav".to_string()][..]));
        assert_eq!(h.log.last().map(String::as_str), Some("3.14"));
    }

    #[test]
    fn the_guide_examples_all_compile() {
        for (title, code) in crate::guide::EXAMPLES {
            Script::compile(code).unwrap_or_else(|e| panic!("example {title}: {e}"));
        }
    }

    #[test]
    fn endless_loops_are_stopped() {
        let mut s = Script::compile("on update { while true { } }").unwrap();
        let mut h = Mock::default();
        assert!(s.update("Cube", 0.1, &mut h).unwrap_err().contains("too long"));
    }

    #[test]
    fn math_strings_and_logic() {
        let h = run("on start { print(abs(-3) + max(2, 5) * 2, \"a\" + 1, 7 % 4, clamp(9, 0, 5), lerp(0, 10, 0.25), !false && (1 < 2 or 0)) }", 0);
        assert_eq!(h.log, vec!["13 a1 3 5 2.5 true"]);
        let h = run("on start { var r = random(5, 6)\n position.x = r }", 0);
        assert!((5.0..6.0).contains(&p(&h, "Cube", Prop::PosX)));
    }
}
