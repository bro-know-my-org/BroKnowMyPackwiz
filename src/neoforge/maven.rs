//! Maven 3.8.5 ComparableVersion semantics used by FML 4.
use super::Result;
use std::cmp::Ordering;

enum Item {
    Number(String),
    Qualifier(String),
    List(Vec<Item>),
}
fn qualifier(value: &str) -> (u8, &str) {
    match value {
        "alpha" => (0, ""),
        "beta" => (1, ""),
        "milestone" => (2, ""),
        "rc" | "cr" => (3, ""),
        "snapshot" => (4, ""),
        "" | "ga" | "final" | "release" => (5, ""),
        "sp" => (6, ""),
        other => (7, other),
    }
}
impl Item {
    fn cmp(&self, other: Option<&Self>) -> Ordering {
        use Item::*;
        match (self, other) {
            (Number(a), Some(Number(b))) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
            (Number(a), None) => (!a.is_empty()).cmp(&false),
            (Number(_), _) | (List(_), Some(Qualifier(_))) => Ordering::Greater,
            (Qualifier(a), Some(Qualifier(b))) => qualifier(a).cmp(&qualifier(b)),
            (Qualifier(a), None) => qualifier(a).cmp(&qualifier("")),
            (Qualifier(_), _) | (List(_), Some(Number(_))) => Ordering::Less,
            (List(a), None) => a
                .iter()
                .map(|v| v.cmp(None))
                .find(|o| *o != Ordering::Equal)
                .unwrap_or(Ordering::Equal),
            (List(a), Some(List(b))) => (0..a.len().max(b.len()))
                .map(|i| {
                    a.get(i)
                        .map_or_else(|| b[i].cmp(None).reverse(), |v| v.cmp(b.get(i)))
                })
                .find(|o| *o != Ordering::Equal)
                .unwrap_or(Ordering::Equal),
        }
    }
    fn null(&self) -> bool {
        match self {
            Self::Number(n) => n.is_empty(),
            Self::Qualifier(q) => qualifier(q) == qualifier(""),
            Self::List(l) => l.is_empty(),
        }
    }
}
fn push(list: &mut Vec<Item>, value: &str, digit: bool, followed_by_digit: bool) {
    list.push(if digit {
        Item::Number(value.trim_start_matches('0').into())
    } else {
        Item::Qualifier(
            match (value, followed_by_digit) {
                ("a", true) => "alpha",
                ("b", true) => "beta",
                ("m", true) => "milestone",
                _ => value,
            }
            .into(),
        )
    });
}
fn parse(version: &str) -> Result<Item> {
    // Bound nesting and leave unresolved templates/invalid metadata unknown.
    if version.is_empty()
        || version.len() > 256
        || !version.is_ascii()
        || version
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b"[](),${}".contains(&b))
    {
        return Err(format!("unsupported Maven version {version}"));
    }
    let version = version.to_ascii_lowercase();
    let mut stack: Vec<Vec<Item>> = vec![vec![]];
    let mut digit = false;
    let mut start = 0;
    for (i, c) in version.bytes().enumerate() {
        if c == b'.' || c == b'-' {
            push(
                stack.last_mut().unwrap(),
                if i == start { "0" } else { &version[start..i] },
                i == start || digit,
                false,
            );
            start = i + 1;
            if c == b'-' {
                stack.push(vec![]);
            }
        } else if c.is_ascii_digit() {
            if !digit && i > start {
                push(stack.last_mut().unwrap(), &version[start..i], false, true);
                start = i;
                stack.push(vec![]);
            }
            digit = true;
        } else {
            if digit && i > start {
                push(stack.last_mut().unwrap(), &version[start..i], true, false);
                start = i;
                stack.push(vec![]);
            }
            digit = false;
        }
    }
    if version.len() > start {
        push(stack.last_mut().unwrap(), &version[start..], digit, false);
    }
    loop {
        let mut list = stack.pop().unwrap();
        let mut i = list.len();
        while i > 0 {
            i -= 1;
            if list[i].null() {
                list.remove(i);
            } else if !matches!(list[i], Item::List(_)) {
                break;
            }
        }
        let item = Item::List(list);
        if let Some(parent) = stack.last_mut() {
            parent.push(item);
        } else {
            return Ok(item);
        }
    }
}
pub fn compare(a: &str, b: &str) -> Result<Ordering> {
    Ok(parse(a)?.cmp(Some(&parse(b)?)))
}
