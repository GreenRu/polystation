//! The console: a log of what the editor and the scripts say, and a command line.
//!
//! A command is the name of an editor tool (the same tools an AI assistant uses over MCP, see the mcp module)
//! followed by values: `add_object kind=Cube position=[0,2,0]`, `select_object Cube`, `set_view top`.
//! A value without `name=` goes to the first value the tool needs.
use crate::json::{self, J};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Info,
    Warn,
    Error,
    /// What a script printed.
    Script,
    /// A command that was typed.
    Command,
    /// What a command answered.
    Result,
}

impl Level {
    /// The three letters that start a line of this level (the colouring looks at them).
    pub fn tag(self) -> &'static str {
        match self {
            Level::Info => "INF",
            Level::Warn => "WRN",
            Level::Error => "ERR",
            Level::Script => "SCR",
            Level::Command => " > ",
            Level::Result => " < ",
        }
    }

    pub fn from_tag(t: &str) -> Option<Level> {
        [Level::Info, Level::Warn, Level::Error, Level::Script, Level::Command, Level::Result].into_iter().find(|l| l.tag() == t)
    }

    /// The level a status message most likely has.
    pub fn of_note(text: &str) -> Level {
        let t = text.to_lowercase();
        if t.contains("error") || t.contains("could not") || t.contains("failed") || t.contains("cannot") || t.contains("not possible") {
            Level::Error
        } else if t.contains("warning") || t.starts_with("stop playing") || t.starts_with("select ") || t.contains("nothing") {
            Level::Warn
        } else {
            Level::Info
        }
    }
}

const MAX_LINES: usize = 3000;
/// Where the text of a line starts: "  12.3 INF ".
pub const TEXT_COL: usize = 11;

pub struct Console {
    lines: Vec<(f32, Level, String)>,
    start: Instant,
    /// The log changed since the text box was last filled.
    pub dirty: bool,
    pub history: Vec<String>,
    hist_pos: Option<usize>,
}

impl Default for Console {
    fn default() -> Console {
        Console::new()
    }
}

impl Console {
    pub fn new() -> Console {
        Console { lines: Vec::new(), start: Instant::now(), dirty: true, history: Vec::new(), hist_pos: None }
    }

    /// Adds a message (a message of several lines becomes several lines). The same message twice in a row
    /// within a second is kept once.
    pub fn log(&mut self, level: Level, text: &str) {
        let t = self.start.elapsed().as_secs_f32();
        for (k, line) in text.lines().enumerate() {
            if k == 0 && self.lines.last().is_some_and(|(lt, ll, lx)| *ll == level && lx == line && t - lt < 1.0 && level != Level::Command) {
                continue;
            }
            self.lines.push((t, level, line.to_string()));
        }
        if self.lines.len() > MAX_LINES {
            let extra = self.lines.len() - MAX_LINES;
            self.lines.drain(..extra);
        }
        self.dirty = true;
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.dirty = true;
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The text of the log box: "  12.3 INF Reloaded".
    pub fn text(&self) -> String {
        let mut s = String::new();
        for (t, l, x) in &self.lines {
            s += &format!("{:>6.1} {} {}\n", t, l.tag(), x);
        }
        s
    }

    /// The last `n` lines, for the tools.
    pub fn tail(&self, n: usize) -> Vec<(f32, Level, String)> {
        self.lines.iter().rev().take(n).rev().cloned().collect()
    }

    pub fn remember(&mut self, line: &str) {
        if self.history.last().is_none_or(|l| l != line) {
            self.history.push(line.to_string());
        }
        self.hist_pos = None;
    }

    /// Up / Down in the command line: the older (or newer) command; None goes back to an empty line.
    pub fn step_history(&mut self, up: bool) -> Option<String> {
        if self.history.is_empty() {
            return None;
        }
        let last = self.history.len() - 1;
        self.hist_pos = match (self.hist_pos, up) {
            (None, true) => Some(last),
            (Some(p), true) => Some(p.saturating_sub(1)),
            (Some(p), false) if p < last => Some(p + 1),
            _ => None,
        };
        self.hist_pos.map(|p| self.history[p].clone())
    }
}

/// Splits a command line into words. Quotes, brackets and braces keep their contents together,
/// so `name="My Cube" position=[0, 1, 0]` is two words.
pub fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut cur, mut quote, mut depth) = (String::new(), None::<char>, 0i32);
    for ch in line.chars() {
        match quote {
            Some(q) => {
                cur.push(ch);
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    cur.push(ch);
                }
                '[' | '{' => {
                    depth += 1;
                    cur.push(ch);
                }
                ']' | '}' => {
                    depth -= 1;
                    cur.push(ch);
                }
                c if c.is_whitespace() && depth <= 0 => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                c => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A typed value: a number, true / false, a list, or text (quotes optional).
pub fn value(text: &str) -> J {
    let t = text.trim();
    let single = t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'');
    if single {
        return J::from(&t[1..t.len() - 1]);
    }
    match json::parse(t) {
        Ok(v) => v,
        Err(_) => J::from(t),
    }
}

/// The first value a tool needs (what a lone word after its name means).
fn first_required(tool: &str) -> Option<String> {
    let list = crate::mcp::tool_list();
    let t = list.iter().find(|t| t.str("name") == Some(tool))?;
    match t.get("inputSchema")?.get("required")?.as_arr()?.first()? {
        J::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// What a line asks for: a tool and its values, or an error that explains what is wrong.
pub fn parse_command(line: &str) -> Result<(String, J), String> {
    let w = words(line);
    let Some(name) = w.first() else { return Err("empty command".into()) };
    // shortcuts for the commands used most
    let (tool, preset): (&str, Option<(&str, &str)>) = match name.as_str() {
        "play" => ("play", Some(("action", "start"))),
        "stop" => ("play", Some(("action", "stop"))),
        "save" => ("save_project", None),
        "undo" => ("history", Some(("action", "undo"))),
        "redo" => ("history", Some(("action", "redo"))),
        other => (other, None),
    };
    if !crate::mcp::tool_list().iter().any(|t| t.str("name") == Some(tool)) {
        return Err(format!("no command called \"{tool}\": type help, or tools for the list"));
    }
    let mut args = J::obj();
    if let Some((k, v)) = preset {
        args = args.with(k, v);
    }
    let mut lone = 0;
    for token in &w[1..] {
        match token.split_once('=') {
            Some((k, v)) if !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_') => args = args.with(k, value(v)),
            _ => {
                // a word without name=: the first value the tool needs
                let Some(k) = first_required(tool).filter(|_| lone == 0) else { return Err(format!("\"{token}\" needs a name: write name=value")) };
                args = args.with(&k, value(token));
                lone += 1;
            }
        }
    }
    Ok((tool.to_string(), args))
}

pub const HELP: &str = "The console shows what the editor and your scripts say (print(...) shows up here).
Type a command below and press Enter. A command is the name of an editor tool and its values:
  select_object Cube                   a lone value goes to the first value the tool needs
  add_object kind=Cube name=Crate position=[0,2,0]
  modify_object name=Crate color=#ff8800 scale=2
  set_environment fog=dithered vertex_snap=320
  play    stop    save    undo    redo
  tools   lists every command        clear   empties the log
Up and Down bring back earlier commands. Press ` to open or close the console.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_keep_quotes_and_lists_together() {
        assert_eq!(words(r#"add_object kind=Cube name="My Cube" position=[0, 2, 0]"#), vec!["add_object", "kind=Cube", r#"name="My Cube""#, "position=[0, 2, 0]"]);
        assert!(words("   ").is_empty());
    }

    #[test]
    fn values_are_numbers_lists_booleans_or_text() {
        assert_eq!(value("3").to_string(), "3");
        assert_eq!(value("[1,2]").to_string(), "[1,2]");
        assert_eq!(value("true").to_string(), "true");
        assert_eq!(value("Cube").to_string(), "\"Cube\"");
        assert_eq!(value("\"a b\"").to_string(), "\"a b\"");
        assert_eq!(value("'x'").to_string(), "\"x\"");
    }

    #[test]
    fn commands_become_tool_calls() {
        let (tool, args) = parse_command(r#"add_object kind=Cube name="My Cube" position=[0,2,0]"#).unwrap();
        assert_eq!(tool, "add_object");
        assert_eq!(args.str("kind"), Some("Cube"));
        assert_eq!(args.str("name"), Some("My Cube"));
        assert_eq!(args.vec3("position"), Some([0.0, 2.0, 0.0]));
        // a lone word goes to the first required value; shortcuts fill in the action
        let (tool, args) = parse_command("select_object Crate").unwrap();
        assert_eq!((tool.as_str(), args.str("name")), ("select_object", Some("Crate")));
        let (tool, args) = parse_command("stop").unwrap();
        assert_eq!((tool.as_str(), args.str("action")), ("play", Some("stop")));
        assert!(parse_command("nonsense").unwrap_err().contains("no command"));
        assert!(parse_command("").is_err());
    }

    #[test]
    fn the_log_keeps_levels_drops_repeats_and_caps_its_size() {
        let mut c = Console::new();
        c.log(Level::Info, "hello");
        c.log(Level::Info, "hello"); // the same message twice in a row
        c.log(Level::Error, "bad\nsecond line");
        assert_eq!(c.len(), 3);
        let text = c.text();
        assert!(text.lines().next().unwrap().ends_with("INF hello"));
        assert!(text.contains("ERR bad") && text.contains("ERR second line"));
        for i in 0..MAX_LINES + 50 {
            c.log(Level::Script, &format!("line {i}"));
        }
        assert_eq!(c.len(), MAX_LINES);
        c.clear();
        assert!(c.is_empty());
        assert_eq!(Level::of_note("Could not save the project"), Level::Error);
        assert_eq!(Level::of_note("Autosaved"), Level::Info);
        assert_eq!(Level::from_tag("ERR"), Some(Level::Error));
    }

    #[test]
    fn history_steps_up_and_down() {
        let mut c = Console::new();
        assert_eq!(c.step_history(true), None);
        c.remember("one");
        c.remember("two");
        c.remember("two");
        assert_eq!(c.history.len(), 2);
        assert_eq!(c.step_history(true).as_deref(), Some("two"));
        assert_eq!(c.step_history(true).as_deref(), Some("one"));
        assert_eq!(c.step_history(false).as_deref(), Some("two"));
        assert_eq!(c.step_history(false), None, "past the newest: back to an empty line");
    }
}
