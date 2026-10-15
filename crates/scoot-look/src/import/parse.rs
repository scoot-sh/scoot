//! Input parsing for `import-base16`: JSON (serde_json, already a
//! workspace dependency) and the flat YAML subset scheme files use
//! (`key: value` mappings, `#` comments, quotes, one nested-map level by
//! indentation). YAML scalars stay strings: a hue like `090300` must not
//! become a number and lose its leading zero.

use std::collections::BTreeMap;

use crate::grammar::GrammarError;

#[derive(Debug, Clone)]
pub(crate) enum Node {
    Str(String),
    Num(f64),
    Map(BTreeMap<String, Node>),
}

impl Node {
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_map(&self) -> Option<&BTreeMap<String, Node>> {
        match self {
            Node::Map(m) => Some(m),
            _ => None,
        }
    }

    /// A number, or a string that parses as one (companions spell sizes
    /// quoted as often as bare).
    pub(crate) fn as_num(&self) -> Option<f64> {
        match self {
            Node::Num(n) => Some(*n),
            Node::Str(s) => s.trim().parse::<f64>().ok(),
            Node::Map(_) => None,
        }
    }
}

pub(crate) fn parse_json(text: &str) -> Result<BTreeMap<String, Node>, GrammarError> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| GrammarError::new("scheme", format!("bad JSON: {e}")))?;
    let map = value
        .as_object()
        .ok_or_else(|| GrammarError::new("scheme", "JSON scheme must be an object"))?;
    let mut out = BTreeMap::new();
    for (k, v) in map {
        out.insert(k.clone(), json_node(v)?);
    }
    Ok(out)
}

fn json_node(value: &serde_json::Value) -> Result<Node, GrammarError> {
    match value {
        serde_json::Value::String(s) => Ok(Node::Str(s.clone())),
        serde_json::Value::Number(n) => n
            .as_f64()
            .map(Node::Num)
            .ok_or_else(|| GrammarError::new("scheme", "number out of range")),
        serde_json::Value::Object(m) => {
            let mut out = BTreeMap::new();
            for (k, v) in m {
                if out.insert(k.clone(), json_node(v)?).is_some() {
                    return Err(GrammarError::new("scheme", format!("duplicate key `{k}`")));
                }
            }
            Ok(Node::Map(out))
        }
        _ => Err(GrammarError::new(
            "scheme",
            "only strings, numbers and maps are accepted",
        )),
    }
}

/// The flat YAML subset scheme files use: `key: value` lines with `#`
/// comments and one level of nested maps by indentation.
pub(crate) fn parse_yaml(text: &str) -> Result<BTreeMap<String, Node>, GrammarError> {
    // (indent, key, value): value `None` opens a nested map.
    let mut lines: Vec<(usize, String, Option<String>)> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line_no = n + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" || trimmed == "..." {
            continue;
        }
        if raw
            .bytes()
            .take_while(|b| *b == b' ' || *b == b'\t')
            .any(|b| b == b'\t')
        {
            return Err(GrammarError::new(
                "scheme",
                format!("line {line_no}: tabs never indent YAML"),
            ));
        }
        let indent = indent_of(raw);
        let rest = &raw[indent..];
        if rest.starts_with("- ") || rest == "-" {
            return Err(GrammarError::new(
                "scheme",
                format!("line {line_no}: lists are not accepted"),
            ));
        }
        let colon = rest.find(':').ok_or_else(|| {
            GrammarError::new("scheme", format!("line {line_no}: expected `key: value`"))
        })?;
        let (key, after) = (&rest[..colon], &rest[colon + 1..]);
        let key = key.trim();
        if key.is_empty() {
            return Err(GrammarError::new(
                "scheme",
                format!("line {line_no}: empty key"),
            ));
        }
        // `key:value` without a space is a plain scalar, not a mapping.
        if !after.is_empty() && !after.starts_with([' ', '\t']) {
            return Err(GrammarError::new(
                "scheme",
                format!("line {line_no}: expected a space after the `:`"),
            ));
        }
        let value = yaml_scalar(after.trim(), line_no)?;
        lines.push((indent, unquote(key, line_no)?.to_string(), value));
    }
    stack_maps(&lines)
}

fn indent_of(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b' ').count()
}

/// One scalar: strip a trailing `#` comment outside quotes, unquote.
fn yaml_scalar(rest: &str, line_no: usize) -> Result<Option<String>, GrammarError> {
    if rest.is_empty() || rest.starts_with('#') {
        return Ok(None);
    }
    let token = if rest.starts_with(['\'', '"']) {
        let quote = rest.as_bytes()[0];
        let mut end = None;
        let bytes = rest.as_bytes();
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' && quote == b'"' {
                i += 2;
                continue;
            }
            if bytes[i] == quote {
                end = Some(i);
                break;
            }
            i += 1;
        }
        let end = end.ok_or_else(|| {
            GrammarError::new("scheme", format!("line {line_no}: unterminated quote"))
        })?;
        let inner = &rest[1..end];
        let tail = rest[end + 1..].trim();
        if !tail.is_empty() && !tail.starts_with('#') {
            return Err(GrammarError::new(
                "scheme",
                format!("line {line_no}: text after the quoted value"),
            ));
        }
        unescape(inner, quote, line_no)?.into_owned()
    } else {
        let end = rest
            .find(" #")
            .or_else(|| rest.find('\t').filter(|i| rest[i + 1..].starts_with('#')));
        rest[..end.unwrap_or(rest.len())].trim().to_string()
    };
    if token.is_empty() {
        return Ok(None);
    }
    Ok(Some(token))
}

fn unquote(key: &str, line_no: usize) -> Result<std::borrow::Cow<'_, str>, GrammarError> {
    let key = key.trim();
    if key.len() >= 2
        && ((key.starts_with('"') && key.ends_with('"'))
            || (key.starts_with('\'') && key.ends_with('\'')))
    {
        let q = key.as_bytes()[0];
        unescape(&key[1..key.len() - 1], q, line_no)
    } else if key.contains(['\'', '"', '#']) {
        Err(GrammarError::new(
            "scheme",
            format!("line {line_no}: bad key `{key}`"),
        ))
    } else {
        Ok(std::borrow::Cow::Borrowed(key))
    }
}

fn unescape(s: &str, quote: u8, line_no: usize) -> Result<std::borrow::Cow<'_, str>, GrammarError> {
    if quote == b'\'' {
        if s.contains("''") {
            Ok(std::borrow::Cow::Owned(s.replace("''", "'")))
        } else {
            Ok(std::borrow::Cow::Borrowed(s))
        }
    } else if s.contains('\\') {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(c) => out.push(c),
                    None => {
                        return Err(GrammarError::new(
                            "scheme",
                            format!("line {line_no}: dangling escape"),
                        ));
                    }
                }
            } else {
                out.push(c);
            }
        }
        Ok(std::borrow::Cow::Owned(out))
    } else {
        Ok(std::borrow::Cow::Borrowed(s))
    }
}

/// Fold the flat `(indent, key, value)` lines into nested maps. Only one
/// nested level is usual (`palette:`); deeper nesting refuses past 8.
fn stack_maps(
    lines: &[(usize, String, Option<String>)],
) -> Result<BTreeMap<String, Node>, GrammarError> {
    // Recursive descent over slices: children are the following lines
    // with a greater indent than the opener.
    fn block(
        lines: &[(usize, String, Option<String>)],
        from: usize,
        indent: usize,
        depth: usize,
    ) -> Result<(BTreeMap<String, Node>, usize), GrammarError> {
        if depth > 8 {
            return Err(GrammarError::new("scheme", "nesting deeper than 8"));
        }
        let mut map = BTreeMap::new();
        let mut i = from;
        while i < lines.len() {
            let (li, key, value) = &lines[i];
            if *li < indent {
                break;
            }
            if *li > indent {
                return Err(GrammarError::new(
                    "scheme",
                    format!("bad indent before `{key}`"),
                ));
            }
            if map.contains_key(key) {
                return Err(GrammarError::new(
                    "scheme",
                    format!("duplicate key `{key}`"),
                ));
            }
            match value {
                Some(v) => {
                    map.insert(key.clone(), scalar_node(v));
                    i += 1;
                }
                None => {
                    if i + 1 >= lines.len() || lines[i + 1].0 <= *li {
                        return Err(GrammarError::new(
                            "scheme",
                            format!("`{key}:` names no values"),
                        ));
                    }
                    let (child, next) = block(lines, i + 1, lines[i + 1].0, depth + 1)?;
                    map.insert(key.clone(), Node::Map(child));
                    i = next;
                }
            }
        }
        Ok((map, i))
    }
    if lines.is_empty() {
        return Err(GrammarError::new("scheme", "empty scheme file"));
    }
    if lines.iter().any(|(li, _, _)| *li != lines[0].0) && lines[0].0 != 0 {
        return Err(GrammarError::new("scheme", "bad top-level indent"));
    }
    let (map, _) = block(lines, 0, lines[0].0, 0)?;
    Ok(map)
}

fn scalar_node(s: &str) -> Node {
    // YAML scalars stay strings: a hue like `090300` is all digits and
    // must not become a number (and lose its leading zero). Numbers for
    // the companion sizes parse on read through `as_num`, which coerces
    // numeric strings; JSON keeps real numbers through `json_node`.
    Node::Str(s.to_string())
}
