//! The message V8's `JSON.parse` throws for invalid input (Node 24): the Node server returns it as
//! `{ "error": … }` when a request body is not JSON. A scanner that finds the first error the way V8's
//! JsonParser does; positions are UTF-16 indexes, as in JS.

struct Scan {
    units: Vec<u16>,
    i: usize,
}

enum Fail {
    Eos,
    /// Unexpected token at a position (message with source context)
    Token(usize),
    /// A message template ending in "in JSON at position %"
    At(&'static str, usize),
}

const MAX_CONTEXT: usize = 10;
const MIN_LENGTH_FOR_CONTEXT: usize = MAX_CONTEXT * 2 + 1;

impl Scan {
    fn peek(&self) -> Option<u16> {
        self.units.get(self.i).copied()
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(0x20 | 0x09 | 0x0A | 0x0D)) {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Result<(), Fail> {
        self.skip_ws();
        let Some(c) = self.peek() else { return Err(Fail::Eos) };
        match c {
            0x7B => self.object(),
            0x5B => self.array(),
            0x22 => self.string(),
            0x2D | 0x30..=0x39 => self.number(),
            0x74 => self.literal("true"),
            0x66 => self.literal("false"),
            0x6E => self.literal("null"),
            _ => Err(Fail::Token(self.i)),
        }
    }
    fn literal(&mut self, word: &str) -> Result<(), Fail> {
        for w in word.encode_utf16() {
            match self.peek() {
                None => return Err(Fail::Eos),
                Some(c) if c == w => self.i += 1,
                Some(_) => return Err(Fail::Token(self.i)),
            }
        }
        Ok(())
    }
    fn digits(&mut self) -> usize {
        let start = self.i;
        while matches!(self.peek(), Some(0x30..=0x39)) {
            self.i += 1;
        }
        self.i - start
    }
    fn number(&mut self) -> Result<(), Fail> {
        if self.peek() == Some(0x2D) {
            self.i += 1;
            if !matches!(self.peek(), Some(0x30..=0x39)) {
                return Err(Fail::At("No number after minus sign", self.i));
            }
        }
        if self.peek() == Some(0x30) {
            self.i += 1;
            if matches!(self.peek(), Some(0x30..=0x39)) {
                return Err(Fail::At("Unexpected number", self.i));
            }
        } else {
            self.digits();
        }
        if self.peek() == Some(0x2E) {
            self.i += 1;
            if self.digits() == 0 {
                return Err(Fail::At("Unterminated fractional number", self.i));
            }
        }
        if matches!(self.peek(), Some(0x65 | 0x45)) {
            self.i += 1;
            if matches!(self.peek(), Some(0x2B | 0x2D)) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(Fail::At("Exponent part is missing a number", self.i));
            }
        }
        Ok(())
    }
    fn string(&mut self) -> Result<(), Fail> {
        self.i += 1; // opening quote
        loop {
            let Some(c) = self.peek() else { return Err(Fail::At("Unterminated string", self.i)) };
            match c {
                0x22 => {
                    self.i += 1;
                    return Ok(());
                }
                0x5C => {
                    self.i += 1;
                    match self.peek() {
                        None => return Err(Fail::At("Unterminated string", self.i)),
                        Some(0x22 | 0x5C | 0x2F | 0x62 | 0x66 | 0x6E | 0x72 | 0x74) => self.i += 1,
                        Some(0x75) => {
                            self.i += 1;
                            for _ in 0..4 {
                                match self.peek() {
                                    Some(h) if (h as u8 as u16 == h) && (h as u8).is_ascii_hexdigit() => self.i += 1,
                                    _ => return Err(Fail::At("Bad Unicode escape", self.i)),
                                }
                            }
                        }
                        Some(_) => return Err(Fail::At("Bad escaped character", self.i)),
                    }
                }
                c if c < 0x20 => return Err(Fail::At("Bad control character in string literal", self.i)),
                _ => self.i += 1,
            }
        }
    }
    fn object(&mut self) -> Result<(), Fail> {
        self.i += 1;
        self.skip_ws();
        if self.peek() == Some(0x7D) {
            self.i += 1;
            return Ok(());
        }
        if self.peek() != Some(0x22) {
            return Err(Fail::At("Expected property name or '}'", self.i));
        }
        loop {
            self.string()?;
            self.skip_ws();
            if self.peek() != Some(0x3A) {
                return Err(Fail::At("Expected ':' after property name", self.i));
            }
            self.i += 1;
            self.value()?;
            self.skip_ws();
            match self.peek() {
                Some(0x2C) => {
                    self.i += 1;
                    self.skip_ws();
                    if self.peek() != Some(0x22) {
                        return Err(Fail::At("Expected double-quoted property name", self.i));
                    }
                }
                Some(0x7D) => {
                    self.i += 1;
                    return Ok(());
                }
                _ => return Err(Fail::At("Expected ',' or '}' after property value", self.i)),
            }
        }
    }
    fn array(&mut self) -> Result<(), Fail> {
        self.i += 1;
        self.skip_ws();
        if self.peek() == Some(0x5D) {
            self.i += 1;
            return Ok(());
        }
        loop {
            self.value()?;
            self.skip_ws();
            match self.peek() {
                Some(0x2C) => self.i += 1,
                Some(0x5D) => {
                    self.i += 1;
                    return Ok(());
                }
                _ => return Err(Fail::At("Expected ',' or ']' after array element", self.i)),
            }
        }
    }
}

fn line_col(units: &[u16], pos: usize) -> (usize, usize) {
    let mut line = 1;
    let mut start = 0;
    for (i, u) in units.iter().enumerate().take(pos) {
        if *u == 0x0A {
            line += 1;
            start = i + 1;
        }
    }
    (line, pos - start + 1)
}

/// The SyntaxError message of `JSON.parse(text)`; None when the text is valid JSON.
pub fn error_message(text: &str) -> Option<String> {
    let units: Vec<u16> = text.encode_utf16().collect();
    if matches!(text, "undefined" | "NaN" | "Infinity" | "[object Object]") {
        return Some(format!("\"{text}\" is not valid JSON"));
    }
    let mut s = Scan { units, i: 0 };
    let result = s.value().and_then(|_| {
        s.skip_ws();
        if s.i < s.units.len() {
            Err(Fail::At("Unexpected non-whitespace character after JSON", s.i))
        } else {
            Ok(())
        }
    });
    let units = &s.units;
    match result {
        Ok(()) => None,
        Err(Fail::Eos) => Some("Unexpected end of JSON input".into()),
        Err(Fail::At(what, pos)) => {
            let (line, col) = line_col(units, pos);
            let what = if what.ends_with("after JSON") { what.to_string() } else { format!("{what} in JSON") };
            Some(format!("{what} at position {pos} (line {line} column {col})"))
        }
        Err(Fail::Token(pos)) => {
            let token = String::from_utf16_lossy(&units[pos..pos + 1]);
            let len = units.len();
            let sub = |a: usize, b: usize| String::from_utf16_lossy(&units[a..b.min(len)]);
            Some(if len >= MIN_LENGTH_FOR_CONTEXT {
                if pos < MAX_CONTEXT {
                    format!("Unexpected token '{token}', \"{}\"... is not valid JSON", sub(0, pos + MAX_CONTEXT))
                } else if pos < len - MAX_CONTEXT {
                    format!("Unexpected token '{token}', ...\"{}\"... is not valid JSON", sub(pos - MAX_CONTEXT, pos + MAX_CONTEXT))
                } else {
                    format!("Unexpected token '{token}', ...\"{}\" is not valid JSON", sub(pos - MAX_CONTEXT, len))
                }
            } else {
                format!("Unexpected token '{token}', \"{text}\" is not valid JSON")
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::error_message;

    #[test]
    fn messages_match_node_24() {
        let b = "\\";
        let cases: Vec<(String, &str)> = vec![
            ("{nope".into(), "Expected property name or '}' in JSON at position 1 (line 1 column 2)"),
            ("{".into(), "Expected property name or '}' in JSON at position 1 (line 1 column 2)"),
            ("{\"a\"".into(), "Expected ':' after property name in JSON at position 4 (line 1 column 5)"),
            ("{\"a\":".into(), "Unexpected end of JSON input"),
            ("{\"a\":1".into(), "Expected ',' or '}' after property value in JSON at position 6 (line 1 column 7)"),
            ("{\"a\":1,}".into(), "Expected double-quoted property name in JSON at position 7 (line 1 column 8)"),
            ("[1".into(), "Expected ',' or ']' after array element in JSON at position 2 (line 1 column 3)"),
            ("[1,]".into(), "Unexpected token ']', \"[1,]\" is not valid JSON"),
            ("\"abc".into(), "Unterminated string in JSON at position 4 (line 1 column 5)"),
            ("nope".into(), "Unexpected token 'o', \"nope\" is not valid JSON"),
            ("tru".into(), "Unexpected end of JSON input"),
            ("1 2".into(), "Unexpected non-whitespace character after JSON at position 2 (line 1 column 3)"),
            ("-".into(), "No number after minus sign in JSON at position 1 (line 1 column 2)"),
            ("1.".into(), "Unterminated fractional number in JSON at position 2 (line 1 column 3)"),
            ("1e".into(), "Exponent part is missing a number in JSON at position 2 (line 1 column 3)"),
            ("\"a\u{1}\"".into(), "Bad control character in string literal in JSON at position 2 (line 1 column 3)"),
            (format!("\"{b}x\""), "Bad escaped character in JSON at position 2 (line 1 column 3)"),
            (format!("\"{b}u12\""), "Bad Unicode escape in JSON at position 5 (line 1 column 6)"),
            ("   ".into(), "Unexpected end of JSON input"),
            ("{\"aaaaaaaaaaaaaaaaaaaaa\": nope}".into(), "Unexpected token 'o', ...\"aaaaaa\": nope}\" is not valid JSON"),
            ("[1,2,3,4,5,6,7,8,9,10,11,x".into(), "Unexpected token 'x', ...\"8,9,10,11,x\" is not valid JSON"),
            ("undefined".into(), "\"undefined\" is not valid JSON"),
            ("01".into(), "Unexpected number in JSON at position 1 (line 1 column 2)"),
            ("ก{".into(), "Unexpected token 'ก', \"ก{\" is not valid JSON"),
        ];
        for (input, want) in cases {
            assert_eq!(error_message(&input).as_deref(), Some(want), "{input}");
        }
        assert_eq!(error_message("{\"a\":[1,true,null,\"x\\n\"]}"), None);
    }
}
