//! A small JSON reader / writer (the MCP protocol and the editor's control port speak JSON).

#[derive(Clone, Debug, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    pub fn obj() -> J {
        J::Obj(Vec::new())
    }

    /// Adds (or replaces) a field of an object.
    pub fn with(mut self, key: &str, v: impl Into<J>) -> J {
        if let J::Obj(f) = &mut self {
            let v = v.into();
            match f.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = v,
                None => f.push((key.to_string(), v)),
            }
        }
        self
    }

    pub fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(f) => f.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            J::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn num(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            J::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn boolean(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            J::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[J]> {
        match self {
            J::Arr(a) => Some(a),
            _ => None,
        }
    }

    /// `[x, y, z]` of numbers.
    pub fn vec3(&self, key: &str) -> Option<[f32; 3]> {
        let a = self.get(key)?.as_arr()?;
        if a.len() != 3 {
            return None;
        }
        let mut out = [0.0; 3];
        for (o, v) in out.iter_mut().zip(a) {
            match v {
                J::Num(n) => *o = *n as f32,
                _ => return None,
            }
        }
        Some(out)
    }

    pub fn to_string(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }

    fn write(&self, out: &mut String) {
        match self {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Num(n) => {
                if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                    out.push_str(&format!("{}", *n as i64));
                } else if n.is_finite() {
                    out.push_str(&format!("{n}"));
                } else {
                    out.push_str("null");
                }
            }
            J::Str(s) => write_str(s, out),
            J::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            J::Obj(f) => {
                out.push('{');
                for (i, (k, v)) in f.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<&str> for J {
    fn from(s: &str) -> J {
        J::Str(s.to_string())
    }
}
impl From<String> for J {
    fn from(s: String) -> J {
        J::Str(s)
    }
}
impl From<f64> for J {
    fn from(n: f64) -> J {
        J::Num(n)
    }
}
impl From<f32> for J {
    fn from(n: f32) -> J {
        J::Num((n as f64 * 1e5).round() / 1e5)
    }
}
impl From<usize> for J {
    fn from(n: usize) -> J {
        J::Num(n as f64)
    }
}
impl From<i32> for J {
    fn from(n: i32) -> J {
        J::Num(n as f64)
    }
}
impl From<bool> for J {
    fn from(b: bool) -> J {
        J::Bool(b)
    }
}
impl From<[f32; 3]> for J {
    fn from(v: [f32; 3]) -> J {
        J::Arr(v.iter().map(|&x| J::from(x)).collect())
    }
}
impl From<Vec<J>> for J {
    fn from(v: Vec<J>) -> J {
        J::Arr(v)
    }
}

pub fn parse(src: &str) -> Result<J, String> {
    let mut p = Parser { s: src.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err("extra text after the JSON value".into());
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at {}", c as char, self.i))
        }
    }

    fn lit(&mut self, word: &str, v: J) -> Result<J, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(format!("bad JSON at {}", self.i))
        }
    }

    fn value(&mut self) -> Result<J, String> {
        self.ws();
        match self.s.get(self.i) {
            None => Err("unexpected end of JSON".into()),
            Some(b'{') => {
                self.i += 1;
                let mut f = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(J::Obj(f));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    f.push((k, self.value()?));
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(J::Obj(f));
                        }
                        _ => return Err(format!("expected ',' or '}}' at {}", self.i)),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(J::Arr(a));
                }
                loop {
                    a.push(self.value()?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(J::Arr(a));
                        }
                        _ => return Err(format!("expected ',' or ']' at {}", self.i)),
                    }
                }
            }
            Some(b'"') => Ok(J::Str(self.string()?)),
            Some(b't') => self.lit("true", J::Bool(true)),
            Some(b'f') => self.lit("false", J::Bool(false)),
            Some(b'n') => self.lit("null", J::Null),
            Some(_) => {
                let start = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i]).ok().and_then(|t| t.parse::<f64>().ok()).map(J::Num).ok_or_else(|| format!("bad JSON at {start}"))
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if self.s.get(self.i) != Some(&b'"') {
            return Err(format!("expected a string at {}", self.i));
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.s.get(self.i).ok_or("unterminated string")?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.s.get(self.i).ok_or("unterminated string")?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.s[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| "invalid UTF-8 in a string".to_string())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let t = self.s.get(self.i..self.i + 4).ok_or("bad \\u escape")?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(t).map_err(|_| "bad \\u escape")?, 16).map_err(|_| "bad \\u escape".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let src = r#"{"a":1,"b":[true,null,"x\nyé"],"c":{"d":-2.5}}"#;
        let v = parse(src).unwrap();
        assert_eq!(v.num("a"), Some(1.0));
        assert_eq!(v.get("c").and_then(|c| c.num("d")), Some(-2.5));
        assert_eq!(parse(&v.to_string()).unwrap(), v);
        assert_eq!(v.get("b").unwrap().as_arr().unwrap()[2], J::Str("x\nyé".into()));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("{").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse("1 2").is_err());
    }

    #[test]
    fn vec3_reads_numbers() {
        let v = parse(r#"{"p":[1,2,3.5]}"#).unwrap();
        assert_eq!(v.vec3("p"), Some([1.0, 2.0, 3.5]));
        assert_eq!(parse(r#"{"p":[1,2]}"#).unwrap().vec3("p"), None);
    }
}
