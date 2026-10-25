//! Syntax highlighting of generated WESL and Rust for read-only text tabs (decision 0014): a small lexer
//! with keyword lists, not a parser. Both languages share the C-like lexical structure; they differ in
//! keywords, attributes (`@vertex` / `#[derive]`), macros and built-in types.

use crate::tab::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Comment,
    String,
    /// Numbers, `true`/`false`, `ALL_CAPS` constants.
    Number,
    Keyword,
    Type,
    Function,
    /// `@vertex`, `#[derive(..)]`.
    Attribute,
    /// `format!`, lifetimes.
    Macro,
    Punct,
}

const WESL_KEYWORDS: &[&str] = &[
    "alias",
    "as",
    "break",
    "case",
    "const",
    "const_assert",
    "continue",
    "continuing",
    "default",
    "diagnostic",
    "discard",
    "else",
    "enable",
    "fn",
    "for",
    "function",
    "if",
    "import",
    "let",
    "loop",
    "override",
    "package",
    "private",
    "read",
    "read_write",
    "requires",
    "return",
    "self",
    "storage",
    "struct",
    "super",
    "switch",
    "uniform",
    "var",
    "while",
    "workgroup",
    "write",
];

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
    "struct", "super", "trait", "type", "unsafe", "use", "where", "while",
];

const WESL_TYPES: &[&str] = &["array", "atomic", "bool", "f16", "f32", "i32", "ptr", "u32"];

const RUST_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8", "u16", "u32", "u64", "u128",
    "usize",
];

fn is_wesl_type(w: &str) -> bool {
    if WESL_TYPES.contains(&w) || w.starts_with("texture_") || w.starts_with("sampler") {
        return true;
    }
    // vec3, vec3f, mat4x4, mat4x4f …
    let b = w.as_bytes();
    let dim = |c: u8| (b'2'..=b'4').contains(&c);
    let suffix = |rest: &[u8]| rest.is_empty() || (rest.len() == 1 && b"fhiu".contains(&rest[0]));
    match b {
        [b'v', b'e', b'c', d, rest @ ..] => dim(*d) && suffix(rest),
        [b'm', b'a', b't', c, b'x', r, rest @ ..] => dim(*c) && dim(*r) && suffix(rest),
        _ => false,
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Splits `text` into styled pieces; concatenated they give `text` back. Whitespace joins the piece before it,
/// neighbours of one style are merged.
pub fn highlight(text: &str, lang: Lang) -> Vec<(Style, &str)> {
    let mut out: Vec<(Style, &str)> = Vec::new();
    let mut push = |style: Style, start: usize, end: usize| {
        if start == end {
            return;
        }
        let blank = text[start..end].chars().all(char::is_whitespace);
        if let Some(last) = out.last_mut()
            && (last.0 == style || blank)
        {
            last.1 = &text[start - last.1.len()..end];
            return;
        }
        out.push((if blank { Style::Plain } else { style }, &text[start..end]));
    };
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let at = |i: usize| chars.get(i).map(|c| c.1);
    let offset = |i: usize| chars.get(i).map_or(text.len(), |c| c.0);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        let start = i;
        let style = match c {
            _ if c.is_whitespace() => {
                while at(i).is_some_and(char::is_whitespace) {
                    i += 1;
                }
                Style::Plain
            }
            '/' if at(i + 1) == Some('/') => {
                while at(i).is_some_and(|c| c != '\n') {
                    i += 1;
                }
                Style::Comment
            }
            '/' if at(i + 1) == Some('*') => {
                // Block comments nest in both languages.
                let mut depth = 0;
                while i < chars.len() {
                    match (chars[i].1, at(i + 1)) {
                        ('/', Some('*')) => (depth, i) = (depth + 1, i + 2),
                        ('*', Some('/')) => (depth, i) = (depth - 1, i + 2),
                        _ => i += 1,
                    }
                    if depth == 0 {
                        break;
                    }
                }
                Style::Comment
            }
            '"' => {
                i += 1;
                while let Some(c) = at(i) {
                    i += if c == '\\' { 2 } else { 1 };
                    if c == '"' {
                        break;
                    }
                }
                i = i.min(chars.len());
                Style::String
            }
            // Rust: 'x' / '\n' are chars, 'a without a closing quote is a lifetime.
            '\'' if lang == Lang::Rust => {
                if at(i + 1) == Some('\\') {
                    i += 2;
                    while at(i).is_some_and(|c| c != '\'') {
                        i += 1;
                    }
                    i = (i + 1).min(chars.len());
                    Style::String
                } else if at(i + 2) == Some('\'') {
                    i += 3;
                    Style::String
                } else {
                    i += 1;
                    while at(i).is_some_and(is_ident) {
                        i += 1;
                    }
                    Style::Macro
                }
            }
            _ if c.is_ascii_digit() => {
                let hex = c == '0' && matches!(at(i + 1), Some('x' | 'X'));
                i += 1;
                while let Some(c) = at(i) {
                    let exponent = !hex && matches!(c, 'e' | 'E') && matches!(at(i + 1), Some('+' | '-'));
                    if exponent {
                        i += 2;
                    } else if is_ident(c) || (c == '.' && at(i + 1).is_some_and(|n| n.is_ascii_digit())) {
                        i += 1;
                    } else {
                        break;
                    }
                }
                Style::Number
            }
            '@' if lang == Lang::Wesl && at(i + 1).is_some_and(is_ident_start) => {
                i += 1;
                while at(i).is_some_and(is_ident) {
                    i += 1;
                }
                Style::Attribute
            }
            '#' if lang == Lang::Rust && matches!((at(i + 1), at(i + 2)), (Some('['), _) | (Some('!'), Some('['))) => {
                let mut depth = 0;
                while let Some(c) = at(i) {
                    i += 1;
                    match c {
                        '[' => depth += 1,
                        ']' => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 && c == ']' {
                        break;
                    }
                }
                Style::Attribute
            }
            _ if is_ident_start(c) => {
                while at(i).is_some_and(is_ident) {
                    i += 1;
                }
                let word = &text[offset(start)..offset(i)];
                let (keywords, builtin): (&[&str], fn(&str) -> bool) = match lang {
                    Lang::Wesl => (WESL_KEYWORDS, is_wesl_type),
                    Lang::Rust => (RUST_KEYWORDS, |w| RUST_TYPES.contains(&w)),
                };
                let next = at(i);
                if lang == Lang::Rust && next == Some('!') && at(i + 1) != Some('=') {
                    i += 1;
                    Style::Macro
                } else if word == "true" || word == "false" {
                    Style::Number
                } else if keywords.contains(&word) {
                    Style::Keyword
                } else if builtin(word) {
                    Style::Type
                } else if word.len() > 1
                    && word.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                {
                    Style::Number
                } else if word.starts_with(|c: char| c.is_ascii_uppercase()) {
                    Style::Type
                } else if next == Some('(') {
                    Style::Function
                } else {
                    Style::Plain
                }
            }
            _ => {
                i += 1;
                Style::Punct
            }
        };
        push(style, offset(start), offset(i));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use Style::{Attribute, Comment, Function, Keyword, Macro, Number, Type};

    /// Style of each piece's first whole-word occurrence (which must lie inside a single span).
    fn check(text: &str, lang: Lang, expect: &[(Style, &[&str])]) {
        let spans = highlight(text, lang);
        assert_eq!(spans.iter().map(|s| s.1).collect::<String>(), text, "spans must cover the text");
        let mut starts = Vec::new();
        let mut at = 0;
        for (style, piece) in &spans {
            starts.push((at, at + piece.len(), *style));
            at += piece.len();
        }
        for (style, pieces) in expect {
            for piece in *pieces {
                let word = |i: Option<char>| i.is_some_and(is_ident);
                let pos = text
                    .match_indices(piece)
                    .map(|(i, _)| i)
                    .find(|&i| !word(text[..i].chars().last()) && !word(text[i + piece.len()..].chars().next()))
                    .unwrap_or_else(|| panic!("{piece} not in the text"));
                let span = starts.iter().find(|(s, e, _)| *s <= pos && pos < *e).unwrap();
                assert!(pos + piece.len() <= span.1, "{piece} is split: {spans:?}");
                assert_eq!(span.2, *style, "{piece}");
            }
        }
    }

    #[test]
    fn wesl() {
        let src = "// AUTOGENERATED\nimport super::stdlib::noise;\n@group(2) @binding(100) var<uniform> p: Params;\n\
                   fn f(uv: vec2f) -> vec4<f32> { /* a /* nested */ b */ return vec4(noise::hash(uv), 1.5e-3f, 0x1Fu); }";
        check(
            src,
            Lang::Wesl,
            &[
                (Comment, &["// AUTOGENERATED", "/* a /* nested */ b */"]),
                (Keyword, &["import", "var", "uniform", "fn", "return"]),
                (Attribute, &["@group", "@binding"]),
                (Number, &["2", "100", "1.5e-3f", "0x1Fu"]),
                (Type, &["Params", "vec2f", "vec4", "f32"]),
                (Function, &["f", "hash"]),
                (Style::Plain, &["noise"]),
            ],
        );
    }

    #[test]
    fn rust() {
        let src = "#[derive(Clone, Debug)]\npub struct A<'a> { s: &'a str, c: char }\n\
                   fn g() { let x = 'x'; let n = '\\n'; assert!(x != 'y'); let r = 0..10; MAX_N; \"a\\\"b\"; }";
        check(
            src,
            Lang::Rust,
            &[
                (Attribute, &["#[derive(Clone, Debug)]"]),
                (Keyword, &["pub", "struct", "let"]),
                (Macro, &["'a", "assert!"]),
                (Style::String, &["'x'", "'\\n'", "\"a\\\"b\""]),
                (Type, &["str", "char", "A"]),
                (Number, &["0", "10", "MAX_N"]),
                (Function, &["g"]),
            ],
        );
    }

    #[test]
    fn unterminated_input_is_covered() {
        for src in ["/* open", "\"open", "'", "#[x", "@", "1e+", "a\u{e9}b /* \u{444} */"] {
            for lang in [Lang::Wesl, Lang::Rust] {
                check(src, lang, &[]);
            }
        }
    }
}
