//! File naming schemes and safe file names.

use cs_model::ItemReport;

#[derive(Debug, Clone, PartialEq)]
pub enum Naming {
    /// Keep the original name (default).
    Keep,
    /// `2005-03-12_14-22-07` (or `2005-03-12` when only the day is known).
    DateTime,
    /// Template with tokens: {original} {year} {month} {day} {hour} {minute} {second}
    Template(String),
}

/// Why a naming pattern was refused. Messages are written for non-technical users.
#[derive(Debug, PartialEq)]
pub enum NamingError {
    Empty,
    TooLong,
    UnknownToken(String),
    Unbalanced,
    BadCharacter(char),
    TooMuchText,
    NoUniquePart,
    EdgeDotOrSpace,
}

impl std::error::Error for NamingError {}

impl std::fmt::Display for NamingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "The pattern is empty."),
            Self::TooLong => write!(f, "The pattern is too long (at most {MAX_TEMPLATE} characters)."),
            Self::UnknownToken(t) => write!(
                f,
                "{{{t}}} is not a known part. Use {{original}}, {{year}}, {{month}}, {{day}}, {{hour}}, {{minute}} or {{second}}."
            ),
            Self::Unbalanced => write!(f, "Every {{ needs a matching }}."),
            Self::BadCharacter(c) => {
                write!(
                    f,
                    "The character {c:?} cannot be used in file names. Letters, digits, spaces and - _ . ( ) [ ] are fine."
                )
            }
            Self::TooMuchText => {
                write!(f, "Too much fixed text (at most {MAX_LITERAL} characters besides the {{parts}}).")
            }
            Self::NoUniquePart => write!(
                f,
                "Include {{original}}, or the full date and time ({{year}} {{month}} {{day}} {{hour}} {{minute}} {{second}}), \
                 so different photos get different names."
            ),
            Self::EdgeDotOrSpace => write!(f, "A name cannot start or end with a dot or a space."),
        }
    }
}

const TOKENS: &[&str] = &["original", "year", "month", "day", "hour", "minute", "second"];
const MAX_TEMPLATE: usize = 80;
const MAX_LITERAL: usize = 40;

fn allowed_literal(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '(' | ')' | '[' | ']' | '+' | ',' | '&' | '\'' | '#')
}

impl Naming {
    /// Parse a user choice: `keep`, `datetime`, or a template containing `{…}`.
    /// Refuses patterns that would produce unsafe, confusing or clashing names.
    pub fn parse(s: &str) -> Result<Self, NamingError> {
        match s {
            "keep" => return Ok(Naming::Keep),
            "datetime" => return Ok(Naming::DateTime),
            _ => {}
        }
        if s.trim().is_empty() {
            return Err(NamingError::Empty);
        }
        if s.chars().count() > MAX_TEMPLATE {
            return Err(NamingError::TooLong);
        }
        if s.starts_with(['.', ' ']) || s.ends_with(['.', ' ']) {
            return Err(NamingError::EdgeDotOrSpace);
        }
        let mut rest = s;
        let mut found = Vec::new();
        let mut literal = 0usize;
        loop {
            let (text, next) = match rest.find('{') {
                Some(i) => (&rest[..i], Some(i)),
                None => (rest, None),
            };
            if text.contains('}') {
                return Err(NamingError::Unbalanced);
            }
            if let Some(c) = text.chars().find(|c| !allowed_literal(*c)) {
                return Err(NamingError::BadCharacter(c));
            }
            literal += text.chars().count();
            let Some(i) = next else { break };
            let j = rest[i..].find('}').ok_or(NamingError::Unbalanced)? + i;
            let tok = &rest[i + 1..j];
            if tok.contains('{') {
                return Err(NamingError::Unbalanced);
            }
            if !TOKENS.contains(&tok) {
                return Err(NamingError::UnknownToken(tok.into()));
            }
            found.push(tok);
            rest = &rest[j + 1..];
        }
        if literal > MAX_LITERAL {
            return Err(NamingError::TooMuchText);
        }
        let full_time = ["year", "month", "day", "hour", "minute", "second"].iter().all(|t| found.contains(t));
        if !found.contains(&"original") && !full_time {
            return Err(NamingError::NoUniquePart);
        }
        Ok(Naming::Template(s.into()))
    }

    /// New stem (no extension) for `item`, or `None` to keep the original.
    pub fn stem_for(&self, item: &ItemReport, year: i16) -> Option<String> {
        let original = || {
            let name = item.rel_path.rsplit('/').next().unwrap_or(&item.rel_path);
            name.rfind('.').filter(|&i| i > 0).map_or(name, |i| &name[..i]).to_string()
        };
        match self {
            Naming::Keep => None,
            Naming::DateTime => {
                let d = crate::best_datetime(item, year)?;
                let mut s = format!("{:04}-{:02}-{:02}", d.year, d.month?, d.day?);
                if let Some((h, m, sec)) = d.time {
                    s += &format!("_{h:02}-{m:02}-{sec:02}");
                }
                Some(s)
            }
            Naming::Template(t) => {
                let d = crate::best_datetime(item, year);
                let two = |v: Option<u8>| v.map(|v| format!("{v:02}"));
                let time = d.and_then(|d| d.time);
                let mut out = t.clone();
                for (tok, val) in [
                    ("original", Some(original())),
                    ("year", Some(format!("{year:04}"))),
                    ("month", two(d.and_then(|d| d.month))),
                    ("day", two(d.and_then(|d| d.day))),
                    ("hour", time.map(|t| format!("{:02}", t.0))),
                    ("minute", time.map(|t| format!("{:02}", t.1))),
                    ("second", time.map(|t| format!("{:02}", t.2))),
                ] {
                    let key = format!("{{{tok}}}");
                    if out.contains(&key) {
                        out = out.replace(&key, &val?); // missing value -> keep original name
                    }
                }
                let s = sanitize(&out);
                (!s.is_empty()).then_some(s)
            }
        }
    }
}

/// Make a string safe as a file/folder name on Windows, macOS and Linux.
pub fn sanitize(s: &str) -> String {
    let mut out: String =
        s.chars()
            .map(|c| {
                if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
    while out.ends_with(['.', ' ']) {
        out.pop();
    }
    let upper = out.to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    let reserved = matches!(base, "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.insert(0, '_');
    }
    // Keep names well under the 255-byte limit, on a char boundary.
    while out.len() > 180 {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_templates() {
        assert_eq!(Naming::parse("keep").unwrap(), Naming::Keep);
        assert!(Naming::parse("{year}-{month}_{original}").is_ok());
        assert!(Naming::parse("{year}-{month}-{day} {hour}.{minute}.{second}").is_ok());
        assert!(Naming::parse("Holiday ({year}) {original}").is_ok());
        assert_eq!(Naming::parse("{yaer}_{original}"), Err(NamingError::UnknownToken("yaer".into())));
        assert_eq!(Naming::parse("{year"), Err(NamingError::Unbalanced));
        assert_eq!(Naming::parse("year}{original}"), Err(NamingError::Unbalanced));
        assert_eq!(Naming::parse("{year}"), Err(NamingError::NoUniquePart));
        assert_eq!(Naming::parse("{second}"), Err(NamingError::NoUniquePart), "seconds alone repeat every minute");
        assert_eq!(Naming::parse("photos/{original}"), Err(NamingError::BadCharacter('/')));
        assert_eq!(Naming::parse("{original}?"), Err(NamingError::BadCharacter('?')));
        assert_eq!(Naming::parse("   "), Err(NamingError::Empty));
        assert_eq!(Naming::parse(".{original}"), Err(NamingError::EdgeDotOrSpace));
        assert_eq!(Naming::parse(&format!("{}{{original}}", "x".repeat(41))), Err(NamingError::TooMuchText));
        assert_eq!(Naming::parse(&"{original}".repeat(9)), Err(NamingError::TooLong));
    }

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize("a:b*c?.jpg"), "a_b_c_.jpg");
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("com1.txt"), "_com1.txt");
        assert_eq!(sanitize("name. "), "name");
        assert_eq!(sanitize("Café"), "Café");
    }
}
