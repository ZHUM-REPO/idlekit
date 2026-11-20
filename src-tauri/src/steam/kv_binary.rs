//! Binary KeyValues parser, used for Steam's `UserGameStatsSchema_<appid>.bin`
//! files (the full achievement + stat definitions for a game).
use anyhow::{anyhow, bail, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Kv {
    Obj(Vec<(String, Kv)>),
    Str(String),
    Int(i32),
    Float(f32),
    Ptr(u32),
    WStr(String),
    Color(u32),
    U64(u64),
    I64(i64),
}

impl Kv {
    pub fn get(&self, key: &str) -> Option<&Kv> {
        match self {
            Kv::Obj(items) => items
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Kv> {
        keys.iter().try_fold(self, |node, k| node.get(k))
    }

    pub fn children(&self) -> &[(String, Kv)] {
        match self {
            Kv::Obj(items) => items,
            _ => &[],
        }
    }

    /// String form of any scalar (numbers are formatted).
    pub fn as_string(&self) -> Option<String> {
        match self {
            Kv::Str(s) | Kv::WStr(s) => Some(s.clone()),
            Kv::Int(v) => Some(v.to_string()),
            Kv::Float(v) => Some(v.to_string()),
            Kv::Ptr(v) | Kv::Color(v) => Some(v.to_string()),
            Kv::U64(v) => Some(v.to_string()),
            Kv::I64(v) => Some(v.to_string()),
            Kv::Obj(_) => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Kv::Int(v) => Some(*v as f64),
            Kv::Float(v) => Some(*v as f64),
            Kv::Ptr(v) | Kv::Color(v) => Some(*v as f64),
            Kv::U64(v) => Some(*v as f64),
            Kv::I64(v) => Some(*v as f64),
            Kv::Str(s) | Kv::WStr(s) => s.trim().parse().ok(),
            Kv::Obj(_) => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Kv::Int(v) => Some(*v as i64),
            Kv::Ptr(v) | Kv::Color(v) => Some(*v as i64),
            Kv::U64(v) => i64::try_from(*v).ok(),
            Kv::I64(v) => Some(*v),
            Kv::Float(v) => Some(*v as i64),
            Kv::Str(s) | Kv::WStr(s) => {
                let t = s.trim();
                t.parse::<i64>().ok().or_else(|| t.parse::<f64>().ok().map(|f| f as i64))
            }
            Kv::Obj(_) => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Kv::Str(s) | Kv::WStr(s) => match s.trim().to_ascii_lowercase().as_str() {
                "1" | "true" => Some(true),
                "0" | "false" | "" => Some(false),
                _ => None,
            },
            other => other.as_i64().map(|v| v != 0),
        }
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            bail!("unexpected end of binary KV at byte {}", self.pos);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }

    fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| anyhow!("unterminated string in binary KV"))?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1;
        Ok(s)
    }

    fn wstr(&mut self) -> Result<String> {
        let mut units = Vec::new();
        loop {
            let b = self.take(2)?;
            let u = u16::from_le_bytes([b[0], b[1]]);
            if u == 0 {
                break;
            }
            units.push(u);
        }
        Ok(String::from_utf16_lossy(&units))
    }
}

const T_OBJ: u8 = 0;
const T_STR: u8 = 1;
const T_INT: u8 = 2;
const T_FLOAT: u8 = 3;
const T_PTR: u8 = 4;
const T_WSTR: u8 = 5;
const T_COLOR: u8 = 6;
const T_U64: u8 = 7;
const T_END: u8 = 8;
const T_I64: u8 = 10;
const T_END_ALT: u8 = 11;

fn read_items(cur: &mut Cursor, top_level: bool) -> Result<Vec<(String, Kv)>> {
    let mut items = Vec::new();
    loop {
        if top_level && cur.pos >= cur.data.len() {
            return Ok(items);
        }
        let t = cur.u8()?;
        if t == T_END || t == T_END_ALT {
            return Ok(items);
        }
        let name = cur.cstr()?;
        let value = match t {
            T_OBJ => Kv::Obj(read_items(cur, false)?),
            T_STR => Kv::Str(cur.cstr()?),
            T_INT => Kv::Int(cur.u32()? as i32),
            T_FLOAT => Kv::Float(f32::from_bits(cur.u32()?)),
            T_PTR => Kv::Ptr(cur.u32()?),
            T_WSTR => Kv::WStr(cur.wstr()?),
            T_COLOR => Kv::Color(cur.u32()?),
            T_U64 => Kv::U64(cur.u64()?),
            T_I64 => Kv::I64(cur.u64()? as i64),
            other => bail!("unknown binary KV type {other} at byte {}", cur.pos),
        };
        items.push((name, value));
    }
}

/// Parse a binary KV blob into its top-level items.
pub fn parse(data: &[u8]) -> Result<Kv> {
    let mut cur = Cursor { data, pos: 0 };
    Ok(Kv::Obj(read_items(&mut cur, true)?))
}

/// Test/debug helper: serialize back to the binary format.
#[cfg(test)]
pub fn write(items: &[(String, Kv)], out: &mut Vec<u8>) {
    for (k, v) in items {
        let (t, payload): (u8, Vec<u8>) = match v {
            Kv::Obj(children) => {
                let mut inner = Vec::new();
                write(children, &mut inner);
                inner.push(T_END);
                (T_OBJ, inner)
            }
            Kv::Str(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                (T_STR, b)
            }
            Kv::Int(i) => (T_INT, i.to_le_bytes().to_vec()),
            Kv::Float(f) => (T_FLOAT, f.to_le_bytes().to_vec()),
            Kv::Ptr(p) => (T_PTR, p.to_le_bytes().to_vec()),
            Kv::Color(c) => (T_COLOR, c.to_le_bytes().to_vec()),
            Kv::U64(u) => (T_U64, u.to_le_bytes().to_vec()),
            Kv::I64(i) => (T_I64, i.to_le_bytes().to_vec()),
            Kv::WStr(s) => {
                let mut b: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
                b.extend_from_slice(&[0, 0]);
                (T_WSTR, b)
            }
        };
        out.push(t);
        out.extend_from_slice(k.as_bytes());
        out.push(0);
        out.extend_from_slice(&payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_types() {
        let doc = vec![(
            "730".to_string(),
            Kv::Obj(vec![
                ("gamename".into(), Kv::Str("Test Game ü".into())),
                ("version".into(), Kv::Int(7)),
                ("f".into(), Kv::Float(1.5)),
                ("big".into(), Kv::U64(u64::MAX - 1)),
                ("neg".into(), Kv::I64(-42)),
                ("w".into(), Kv::WStr("wide ★".into())),
                ("nested".into(), Kv::Obj(vec![("x".into(), Kv::Str("1".into()))])),
            ]),
        )];
        let mut bytes = Vec::new();
        write(&doc, &mut bytes);
        bytes.push(T_END);
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed, Kv::Obj(doc));
        let root = parsed.get("730").unwrap();
        assert_eq!(root.get("GAMENAME").unwrap().as_string().unwrap(), "Test Game ü");
        assert_eq!(root.get("version").unwrap().as_i64(), Some(7));
        assert_eq!(root.path(&["nested", "x"]).unwrap().as_bool(), Some(true));
    }

    #[test]
    fn truncated_input_errors() {
        let mut bytes = Vec::new();
        write(&[("a".into(), Kv::Int(5))], &mut bytes);
        bytes.truncate(bytes.len() - 2);
        assert!(parse(&bytes).is_err());
        assert!(parse(&[0x63]).is_err());
    }
}
