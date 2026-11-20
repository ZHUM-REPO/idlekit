//! Text KeyValues (VDF) parser for Steam's config files
//! (loginusers.vdf, registry.vdf, libraryfolders.vdf).
use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    /// Case-insensitive child lookup (Steam is inconsistent about key casing).
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(items) => items
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Vdf> {
        keys.iter().try_fold(self, |node, k| node.get(k))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(items) => items,
            Vdf::Str(_) => &[],
        }
    }
}

enum Tok {
    Str(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Result<Vec<Tok>> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() || c == '\u{feff}' => {
                chars.next();
            }
            '/' => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                } else {
                    out.push(Tok::Str("/".into()));
                }
            }
            '{' => {
                chars.next();
                out.push(Tok::Open);
            }
            '}' => {
                chars.next();
                out.push(Tok::Close);
            }
            '[' => {
                // Platform conditionals like [$WIN32] — ignore them.
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                }
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('\\') => s.push('\\'),
                            Some('"') => s.push('"'),
                            Some(other) => {
                                s.push('\\');
                                s.push(other);
                            }
                            None => break,
                        },
                        '"' => {
                            closed = true;
                            break;
                        }
                        c => s.push(c),
                    }
                }
                if !closed {
                    bail!("unterminated string in VDF");
                }
                out.push(Tok::Str(s));
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                out.push(Tok::Str(s));
            }
        }
    }
    Ok(out)
}

fn parse_items(toks: &[Tok], pos: &mut usize, nested: bool) -> Result<Vec<(String, Vdf)>> {
    let mut items = Vec::new();
    while *pos < toks.len() {
        match &toks[*pos] {
            Tok::Close => {
                *pos += 1;
                if nested {
                    return Ok(items);
                }
                // Stray close brace at top level: ignore.
            }
            Tok::Open => bail!("unexpected '{{' in VDF"),
            Tok::Str(key) => {
                *pos += 1;
                match toks.get(*pos) {
                    Some(Tok::Open) => {
                        *pos += 1;
                        let children = parse_items(toks, pos, true)?;
                        items.push((key.clone(), Vdf::Obj(children)));
                    }
                    Some(Tok::Str(v)) => {
                        *pos += 1;
                        items.push((key.clone(), Vdf::Str(v.clone())));
                    }
                    Some(Tok::Close) | None => {
                        items.push((key.clone(), Vdf::Str(String::new())));
                    }
                }
            }
        }
    }
    if nested {
        bail!("unexpected end of VDF (missing '}}')");
    }
    Ok(items)
}

/// Parse a VDF document. The result is an object holding the top-level keys.
pub fn parse(text: &str) -> Result<Vdf> {
    let toks = tokenize(text)?;
    let mut pos = 0;
    Ok(Vdf::Obj(parse_items(&toks, &mut pos, false)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_loginusers() {
        let text = r#"
"users"
{
	"76561198000000001"
	{
		"AccountName"		"alice"
		"PersonaName"		"Alice \"A\" Smith"
		"MostRecent"		"1"
	}
	// a comment
	"76561198000000002"
	{
		"AccountName"		"bob"
		"mostrecent"		"0"
	}
}
"#;
        let v = parse(text).unwrap();
        let users = v.get("users").unwrap();
        assert_eq!(users.entries().len(), 2);
        let a = users.get("76561198000000001").unwrap();
        assert_eq!(a.get("accountname").unwrap().as_str(), Some("alice"));
        assert_eq!(a.get("PersonaName").unwrap().as_str(), Some("Alice \"A\" Smith"));
        assert_eq!(
            v.path(&["users", "76561198000000002", "MostRecent"]).unwrap().as_str(),
            Some("0")
        );
    }

    #[test]
    fn parses_registry_vdf() {
        let text = "\"Registry\" { \"HKCU\" { \"Software\" { \"Valve\" { \"Steam\" { \"ActiveProcess\" { \"ActiveUser\" \"39734273\" \"pid\" \"1234\" } } } } } }";
        let v = parse(text).unwrap();
        let au = v
            .path(&["Registry", "HKCU", "Software", "Valve", "Steam", "ActiveProcess", "ActiveUser"])
            .unwrap();
        assert_eq!(au.as_str(), Some("39734273"));
    }

    #[test]
    fn rejects_unterminated() {
        assert!(parse("\"a\" { \"b\" \"c\"").is_err());
        assert!(parse("\"a\" \"unterminated").is_err());
    }
}
