//! Syntax highlighting of read-only text tabs (decision 0014, design §6.2): the token styles, a small lexer with
//! keyword lists (not a parser) shared by the C-like languages, and the lookup of a language's highlighter. Rust is
//! core's own; a domain registers the languages of its generated files ([`crate::domain::DomainRegistrar::highlighter`],
//! Material: WESL). A language nobody registered shows as plain text.

use bevy::color::Color;

use crate::domain::DomainList;
use crate::theme::TEXT;

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

/// The colour of a token style (the text tabs, code blocks of the welcome screen and Quick Start).
pub fn style_color(style: Style) -> Color {
    match style {
        Style::Plain => TEXT,
        Style::Comment => Color::srgb_u8(0x7f, 0x84, 0x8e),
        Style::String => Color::srgb_u8(0x98, 0xc3, 0x79),
        Style::Number => Color::srgb_u8(0xd1, 0x9a, 0x66),
        Style::Keyword => Color::srgb_u8(0xc6, 0x78, 0xdd),
        Style::Type => Color::srgb_u8(0xe5, 0xc0, 0x7b),
        Style::Function => Color::srgb_u8(0x61, 0xaf, 0xef),
        Style::Attribute | Style::Macro => Color::srgb_u8(0x56, 0xb6, 0xc2),
        Style::Punct => Color::srgb_u8(0xab, 0xb2, 0xbf),
    }
}

/// What differs between the C-like languages the lexer handles: keyword lists, built-in types and the few lexical
/// forms only some of them have. Comments (`//`, nesting `/* */`), strings, numbers and identifiers are common.
pub struct Rules {
    pub keywords: &'static [&'static str],
    /// Built-in type names (`vec3f`, `u8`); other `CamelCase` words are types too.
    pub builtin_type: fn(&str) -> bool,
    /// `'x'` / `'\n'` are chars and `'a` without a closing quote is a lifetime (Rust).
    pub char_lifetimes: bool,
    /// `@vertex`, `@group(0)` attributes (WGSL / WESL).
    pub at_attributes: bool,
    /// `#[derive(..)]` / `#![..]` attributes (Rust).
    pub hash_attributes: bool,
    /// `name!` is a macro call (Rust).
    pub macro_calls: bool,
}

pub fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

pub fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Splits `text` into styled pieces; concatenated they give `text` back. Whitespace joins the piece before it,
/// neighbours of one style are merged.
pub fn lex<'a>(text: &'a str, rules: &Rules) -> Vec<(Style, &'a str)> {
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
            '\'' if rules.char_lifetimes => {
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
            '@' if rules.at_attributes && at(i + 1).is_some_and(is_ident_start) => {
                i += 1;
                while at(i).is_some_and(is_ident) {
                    i += 1;
                }
                Style::Attribute
            }
            '#' if rules.hash_attributes
                && matches!((at(i + 1), at(i + 2)), (Some('['), _) | (Some('!'), Some('['))) =>
            {
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
                let next = at(i);
                if rules.macro_calls && next == Some('!') && at(i + 1) != Some('=') {
                    i += 1;
                    Style::Macro
                } else if word == "true" || word == "false" {
                    Style::Number
                } else if rules.keywords.contains(&word) {
                    Style::Keyword
                } else if (rules.builtin_type)(word) {
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

/// Rust's rules (generated `crates/quillshade` modules).
pub const RUST: Rules = Rules {
    keywords: RUST_KEYWORDS,
    builtin_type: |w| RUST_TYPES.contains(&w),
    char_lifetimes: true,
    at_attributes: false,
    hash_attributes: true,
    macro_calls: true,
};

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
    "struct", "super", "trait", "type", "unsafe", "use", "where", "while",
];

const RUST_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8", "u16", "u32", "u64", "u128",
    "usize",
];

/// The Rust highlighter, registered for `"rust"` by core.
pub fn highlight_rust(text: &str) -> Vec<(Style, &str)> {
    lex(text, &RUST)
}

/// The highlighters core provides itself.
pub fn builtin(lang: &str) -> Option<crate::domain::Highlighter> {
    (lang == "rust").then_some(highlight_rust as crate::domain::Highlighter)
}

/// The styled pieces of `text` in language `lang`, through the highlighter a domain (or core) registered for it;
/// plain text when there is none.
pub fn highlight<'a>(list: &DomainList, text: &'a str, lang: &str) -> Vec<(Style, &'a str)> {
    match list.highlighter(lang) {
        Some(h) => h(text),
        None if text.is_empty() => Vec::new(),
        None => vec![(Style::Plain, text)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Style::{Attribute, Function, Keyword, Macro, Number, Type};

    /// Style of each piece's first whole-word occurrence (which must lie inside a single span).
    fn check(text: &str, lang: &str, expect: &[(Style, &[&str])]) {
        let spans = highlight(&DomainList::default(), text, lang);
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
    fn rust() {
        let src = "#[derive(Clone, Debug)]\npub struct A<'a> { s: &'a str, c: char }\n\
                   fn g() { let x = 'x'; let n = '\\n'; assert!(x != 'y'); let r = 0..10; MAX_N; \"a\\\"b\"; }";
        check(
            src,
            "rust",
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
            check(src, "rust", &[]);
        }
    }

    /// Spans captured from the lexer before it moved into core (it was `material_domain::highlight`): Rust keeps
    /// producing exactly these.
    #[test]
    fn rust_is_core() {
        let src = "#[derive(Clone, Debug)]\npub struct A<'a> { s: &'a str, c: char }\n\
         fn g() { let x = 'x'; let n = '\\n'; assert!(x != 'y'); let r = 0..10; MAX_N; \"a\\\"b\"; }\n\
         #![allow(dead_code)] impl Foo { fn bar(&self) -> Option<u8> { Some(1_000u8) } } // done\n";
        let expect: &[(Style, &str)] = &[
            (Style::Attribute, "#[derive(Clone, Debug)]\n"),
            (Style::Keyword, "pub struct "),
            (Style::Type, "A"),
            (Style::Punct, "<"),
            (Style::Macro, "'a"),
            (Style::Punct, "> { "),
            (Style::Plain, "s"),
            (Style::Punct, ": &"),
            (Style::Macro, "'a "),
            (Style::Type, "str"),
            (Style::Punct, ", "),
            (Style::Plain, "c"),
            (Style::Punct, ": "),
            (Style::Type, "char "),
            (Style::Punct, "}\n"),
            (Style::Keyword, "fn "),
            (Style::Function, "g"),
            (Style::Punct, "() { "),
            (Style::Keyword, "let "),
            (Style::Plain, "x "),
            (Style::Punct, "= "),
            (Style::String, "'x'"),
            (Style::Punct, "; "),
            (Style::Keyword, "let "),
            (Style::Plain, "n "),
            (Style::Punct, "= "),
            (Style::String, "'\\n'"),
            (Style::Punct, "; "),
            (Style::Macro, "assert!"),
            (Style::Punct, "("),
            (Style::Plain, "x "),
            (Style::Punct, "!= "),
            (Style::String, "'y'"),
            (Style::Punct, "); "),
            (Style::Keyword, "let "),
            (Style::Plain, "r "),
            (Style::Punct, "= "),
            (Style::Number, "0"),
            (Style::Punct, ".."),
            (Style::Number, "10"),
            (Style::Punct, "; "),
            (Style::Number, "MAX_N"),
            (Style::Punct, "; "),
            (Style::String, "\"a\\\"b\""),
            (Style::Punct, "; }\n"),
            (Style::Attribute, "#![allow(dead_code)] "),
            (Style::Keyword, "impl "),
            (Style::Type, "Foo "),
            (Style::Punct, "{ "),
            (Style::Keyword, "fn "),
            (Style::Function, "bar"),
            (Style::Punct, "(&"),
            (Style::Keyword, "self"),
            (Style::Punct, ") -> "),
            (Style::Type, "Option"),
            (Style::Punct, "<"),
            (Style::Type, "u8"),
            (Style::Punct, "> { "),
            (Style::Type, "Some"),
            (Style::Punct, "("),
            (Style::Number, "1_000u8"),
            (Style::Punct, ") } } "),
            (Style::Comment, "// done\n"),
        ];
        assert_eq!(lex(src, &RUST), expect);
    }

    /// A language nobody registered is plain text, byte for byte.
    #[test]
    fn unknown_lang_is_plain() {
        let list = DomainList::default();
        assert_eq!(highlight(&list, "fn main() {}\n// x", "nope"), [(Style::Plain, "fn main() {}\n// x")]);
        assert!(highlight(&list, "", "nope").is_empty());
        // WESL is a domain's: without Material core does not know it.
        assert_eq!(highlight(&list, "@vertex fn f() {}", "wesl").len(), 1);
    }

    #[test]
    fn rust_is_registered_by_core() {
        assert_eq!(highlight(&DomainList::default(), "fn f() {}", "rust"), lex("fn f() {}", &RUST));
    }
}
