//! A small cursor over a command's argument list. The Node CLI hand-parses
//! every command with index loops (`cli.ts`); this mirrors those loops so
//! each verb's parser reads like the original: consume a token, peek at the
//! next one, decide.

/// A forward-only cursor over `args`.
#[derive(Debug, Clone)]
pub struct Argv<'a> {
    args: &'a [String],
    pos: usize,
}

#[allow(dead_code)] // the socket verbs' parsers use the rest
impl<'a> Argv<'a> {
    /// Start at the first token.
    pub fn new(args: &'a [String]) -> Self {
        Argv { args, pos: 0 }
    }

    /// The current token without consuming it.
    pub fn peek(&self) -> Option<&'a str> {
        self.args.get(self.pos).map(String::as_str)
    }

    /// The token after the current one, without consuming anything.
    pub fn peek_next(&self) -> Option<&'a str> {
        self.args.get(self.pos + 1).map(String::as_str)
    }

    /// Consume and return the current token.
    pub fn next(&mut self) -> Option<&'a str> {
        let tok = self.peek()?;
        self.pos += 1;
        Some(tok)
    }

    /// Consume the current token and return the one after it, if any
    /// (`--flag <value>`: the flag is at the cursor, the value follows).
    pub fn take_value(&mut self) -> Option<&'a str> {
        self.pos += 1;
        self.next()
    }

    /// Is the current token present and does it start with `-`?
    pub fn at_dash(&self) -> bool {
        self.peek().is_some_and(|t| t.starts_with('-'))
    }

    /// Is there a token after the current one?
    pub fn has_next(&self) -> bool {
        self.pos + 1 < self.args.len()
    }

    /// Everything from the cursor on, unconsumed.
    pub fn rest(&self) -> &'a [String] {
        &self.args[self.pos.min(self.args.len())..]
    }

    /// Is the cursor past the end?
    pub fn is_empty(&self) -> bool {
        self.pos >= self.args.len()
    }
}

/// JavaScript's `parseFloat`: the longest leading decimal prefix (an
/// optional sign, digits, a fraction, an exponent), `NaN` when there is
/// none. Leading whitespace is skipped, trailing garbage ignored
/// (`parseFloat("5s") === 5`).
pub fn js_parse_float(s: &str) -> f64 {
    let s = s.trim_start();
    let b = s.as_bytes();
    let mut end = 0;
    if end < b.len() && (b[end] == b'+' || b[end] == b'-') {
        end += 1;
    }
    let int_start = end;
    while end < b.len() && b[end].is_ascii_digit() {
        end += 1;
    }
    let mut digits = end - int_start;
    if end < b.len() && b[end] == b'.' {
        let frac_start = end + 1;
        let mut f = frac_start;
        while f < b.len() && b[f].is_ascii_digit() {
            f += 1;
        }
        if f > frac_start || digits > 0 {
            digits += f - frac_start;
            end = f;
        }
    }
    if digits == 0 {
        if s.starts_with("Infinity") || s.starts_with("+Infinity") {
            return f64::INFINITY;
        }
        if s.starts_with("-Infinity") {
            return f64::NEG_INFINITY;
        }
        return f64::NAN;
    }
    if end < b.len() && (b[end] == b'e' || b[end] == b'E') {
        let mut e = end + 1;
        if e < b.len() && (b[e] == b'+' || b[e] == b'-') {
            e += 1;
        }
        let exp_start = e;
        while e < b.len() && b[e].is_ascii_digit() {
            e += 1;
        }
        if e > exp_start {
            end = e;
        }
    }
    s[..end].parse().unwrap_or(f64::NAN)
}

/// JavaScript's `parseInt(s, 10)`: leading whitespace, an optional sign,
/// then digits; `None` (NaN) when there are no digits.
pub fn js_parse_int(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let end = rest.bytes().take_while(u8::is_ascii_digit).count();
    if end == 0 {
        return None;
    }
    let v: i64 = rest[..end].parse().ok()?;
    Some(if neg { -v } else { v })
}

/// Format an `f64` the way JavaScript's template literal does for the
/// numbers this CLI prints (`10` not `10.0`, `1.5`, `NaN`).
pub fn js_number(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if v == v.trunc() && v.abs() < 1e21 {
        return format!("{}", v as i64);
    }
    format!("{v}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_float_matches_js() {
        assert_eq!(js_parse_float("10"), 10.0);
        assert_eq!(js_parse_float("1.5"), 1.5);
        assert_eq!(js_parse_float("5s"), 5.0);
        assert_eq!(js_parse_float(" 2"), 2.0);
        assert!(js_parse_float("abc").is_nan());
        assert!(js_parse_float("").is_nan());
        assert_eq!(js_parse_float(".5"), 0.5);
        assert_eq!(js_parse_float("1e2x"), 100.0);
    }

    #[test]
    fn parse_int_matches_js() {
        assert_eq!(js_parse_int("30"), Some(30));
        assert_eq!(js_parse_int("15abc"), Some(15));
        assert_eq!(js_parse_int("-5"), Some(-5));
        assert_eq!(js_parse_int("abc"), None);
        assert_eq!(js_parse_int(""), None);
    }

    #[test]
    fn number_formatting() {
        assert_eq!(js_number(10.0), "10");
        assert_eq!(js_number(1.5), "1.5");
        assert_eq!(js_number(f64::NAN), "NaN");
    }

    #[test]
    fn cursor_basics() {
        let args: Vec<String> = ["--wait", "bell", "name"].iter().map(|s| s.to_string()).collect();
        let mut a = Argv::new(&args);
        assert!(a.at_dash());
        assert_eq!(a.take_value(), Some("bell"));
        assert_eq!(a.peek(), Some("name"));
        assert_eq!(a.rest(), &args[2..]);
        assert_eq!(a.next(), Some("name"));
        assert!(a.is_empty());
    }
}
