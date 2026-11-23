use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            Json::Str(s) => s.parse::<f64>().ok(),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Number(n) => Some(*n as i64),
            Json::Str(s) => s.parse::<i64>().ok(),
            _ => None,
        }
    }

    pub fn as_u8(&self) -> Option<u8> {
        self.as_i64().and_then(|v| u8::try_from(v).ok())
    }

    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    pub fn to_string_compact(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out, None, 0);
        out
    }

    pub fn to_string_pretty(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out, Some(2), 0);
        out.push('\n');
        out
    }

    fn write_json(&self, out: &mut String, indent: Option<usize>, depth: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Number(n) => {
                if n.is_finite() {
                    if n.fract() == 0.0 && n.abs() < (i64::MAX as f64) {
                        let _ = write!(out, "{}", *n as i64);
                    } else {
                        let _ = write!(out, "{}", n);
                    }
                } else {
                    out.push_str("null");
                }
            }
            Json::Str(s) => write_escaped_str(out, s),
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    if let Some(ind) = indent {
                        out.push('\n');
                        for _ in 0..((depth + 1) * ind) {
                            out.push(' ');
                        }
                    }
                    item.write_json(out, indent, depth + 1);
                }
                if let Some(ind) = indent {
                    out.push('\n');
                    for _ in 0..(depth * ind) {
                        out.push(' ');
                    }
                }
                out.push(']');
            }
            Json::Object(entries) => {
                if entries.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    if let Some(ind) = indent {
                        out.push('\n');
                        for _ in 0..((depth + 1) * ind) {
                            out.push(' ');
                        }
                    }
                    write_escaped_str(out, k);
                    if indent.is_some() {
                        out.push_str(": ");
                    } else {
                        out.push(':');
                    }
                    v.write_json(out, indent, depth + 1);
                }
                if let Some(ind) = indent {
                    out.push('\n');
                    for _ in 0..(depth * ind) {
                        out.push(' ');
                    }
                }
                out.push('}');
            }
        }
    }
}

fn write_escaped_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn parse(input: &str) -> Result<Json, String> {
    let mut parser = Parser {
        bytes: input.as_bytes(),
        pos: 0,
    };
    let val = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos < parser.bytes.len() {
        return Err(format!("Unexpected trailing data at byte {}", parser.pos));
    }
    Ok(val)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'n') => self.parse_null(),
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(b'"') => self.parse_string().map(Json::Str),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number(),
            Some(b) => Err(format!(
                "Unexpected character '{}' at position {}",
                b as char, self.pos
            )),
            None => Err("Unexpected end of JSON input".to_string()),
        }
    }

    fn parse_null(&mut self) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(b"null") {
            self.pos += 4;
            Ok(Json::Null)
        } else {
            Err(format!("Invalid literal at position {}", self.pos))
        }
    }

    fn parse_bool(&mut self) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(b"true") {
            self.pos += 4;
            Ok(Json::Bool(true))
        } else if self.bytes[self.pos..].starts_with(b"false") {
            self.pos += 5;
            Ok(Json::Bool(false))
        } else {
            Err(format!("Invalid boolean at position {}", self.pos))
        }
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while let Some(b'0'..=b'9') = self.peek() {
                self.pos += 1;
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            while let Some(b'0'..=b'9') = self.peek() {
                self.pos += 1;
            }
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|e| format!("Invalid UTF-8 in number: {e}"))?;
        let n = s
            .parse::<f64>()
            .map_err(|e| format!("Invalid number '{s}': {e}"))?;
        Ok(Json::Number(n))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err(format!("Expected '\"' at position {}", self.pos));
        }
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                self.pos += 1;
                if self.pos >= self.bytes.len() {
                    return Err("Unterminated escape sequence".to_string());
                }
                let esc = self.bytes[self.pos];
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{08}'),
                    b'f' => out.push('\u{0C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let code = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&code) {
                            // High surrogate, check for low surrogate \uDC00..=\uDFFF
                            if self.bytes[self.pos..].starts_with(b"\\u") {
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                let combined = 0x10000
                                    + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                if let Some(ch) = char::from_u32(combined) {
                                    out.push(ch);
                                }
                            }
                        } else if let Some(ch) = char::from_u32(code as u32) {
                            out.push(ch);
                        }
                    }
                    other => {
                        return Err(format!("Invalid escape '\\{}'", other as char));
                    }
                }
            } else {
                // Find the next quote or backslash to slice valid UTF-8 chunk
                let start = self.pos;
                while self.pos < self.bytes.len()
                    && self.bytes[self.pos] != b'"'
                    && self.bytes[self.pos] != b'\\'
                {
                    self.pos += 1;
                }
                let chunk = std::str::from_utf8(&self.bytes[start..self.pos])
                    .map_err(|e| format!("Invalid UTF-8 in string: {e}"))?;
                out.push_str(chunk);
            }
        }
        Err("Unterminated JSON string".to_string())
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        if self.pos + 4 > self.bytes.len() {
            return Err("Incomplete \\uXXXX escape".to_string());
        }
        let hex_str = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
            .map_err(|_| "Invalid hex digits".to_string())?;
        self.pos += 4;
        u16::from_str_radix(hex_str, 16).map_err(|e| format!("Invalid hex escape '{hex_str}': {e}"))
    }

    fn parse_array(&mut self) -> Result<Json, String> {
        self.pos += 1; // skip '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            let val = self.parse_value()?;
            items.push(val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(format!("Expected ',' or ']' at position {}", self.pos)),
            }
        }
    }

    fn parse_object(&mut self) -> Result<Json, String> {
        self.pos += 1; // skip '{'
        let mut entries = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(entries));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("Expected ':' at position {}", self.pos));
            }
            self.pos += 1;
            let val = self.parse_value()?;
            entries.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(entries));
                }
                _ => return Err(format!("Expected ',' or '}}' at position {}", self.pos)),
            }
        }
    }
}
