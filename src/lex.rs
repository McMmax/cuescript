//! tokenizer — line/column tracked so an author gets a useful error, not an offset

use crate::error::{CueError, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// bare word: keyword, identifier, or dotted path (`register.username`)
    Word(String),
    /// "quoted string" — may contain {placeholders}, kept raw for the compiler
    Str(String),
    /// integer literal
    Num(i64),
    /// `{...}` interpolation used as a standalone value, kept raw
    Expr(String),
    LBrace,
    RBrace,
    Eq,
    Comma,
    /// `->`
    Arrow,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// step keywords that could otherwise look like a one-word interpolation
fn is_step_keyword(w: &str) -> bool {
    matches!(
        w,
        "release"
            | "open"
            | "stay"
            | "say"
            | "kick"
            | "let"
            | "require"
            | "lookup"
            | "call"
            | "remember"
    )
}

/// splits source into tokens. `#` starts a comment to end of line
pub fn lex(src: &str) -> Result<Vec<Token>, CueError> {
    let mut out = Vec::with_capacity(src.len() / 4);
    let bytes: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut line = 1u32;
    let mut col = 1u32;

    macro_rules! push {
        ($t:expr, $l:expr, $c:expr) => {
            out.push(Token {
                tok: $t,
                span: Span { line: $l, col: $c },
            })
        };
    }

    while i < bytes.len() {
        let c = bytes[i];
        let (sl, sc) = (line, col);

        // whitespace
        if c == '\n' {
            line += 1;
            col = 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            col += 1;
            continue;
        }

        // comment — but `#` also starts a hex colour (`color=#58C7F3`), so it
        // only opens a comment when followed by whitespace or another `#`
        if c == '#'
            && bytes
                .get(i + 1)
                .is_none_or(|n| n.is_whitespace() || *n == '#')
        {
            while i < bytes.len() && bytes[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // string literal
        if c == '"' {
            i += 1;
            col += 1;
            let mut s = String::new();
            let mut closed = false;
            while i < bytes.len() {
                let ch = bytes[i];
                if ch == '\\' && i + 1 < bytes.len() {
                    // keep escapes for the template layer, except \" which ends nothing
                    let next = bytes[i + 1];
                    if next == '"' {
                        s.push('"');
                    } else {
                        s.push('\\');
                        s.push(next);
                    }
                    i += 2;
                    col += 2;
                    continue;
                }
                if ch == '"' {
                    closed = true;
                    i += 1;
                    col += 1;
                    break;
                }
                if ch == '\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
                s.push(ch);
                i += 1;
            }
            if !closed {
                return Err(CueError::syntax(
                    Span { line: sl, col: sc },
                    "unterminated string",
                ));
            }
            push!(Tok::Str(s), sl, sc);
            continue;
        }

        // `{` is either an interpolation (`fill={player.name}`) or a block.
        // an interpolation is a single path: it closes on the same line and
        // contains no whitespace. a block always separates keywords with
        // spaces (`{ open b }`), and a lone keyword (`{release}`) is still a
        // block, so keywords are excluded explicitly
        if c == '{' {
            let mut j = i + 1;
            let mut inner = String::new();
            let mut closed = false;
            while j < bytes.len() {
                let ch = bytes[j];
                if ch == '\n' || ch == '{' {
                    break;
                }
                if ch == '}' {
                    closed = true;
                    break;
                }
                inner.push(ch);
                j += 1;
            }
            let is_interpolation = closed
                && !inner.is_empty()
                && !inner.chars().any(char::is_whitespace)
                && !is_step_keyword(&inner);
            if is_interpolation {
                col += (j - i + 1) as u32;
                i = j + 1;
                push!(Tok::Expr(inner), sl, sc);
                continue;
            }
            i += 1;
            col += 1;
            push!(Tok::LBrace, sl, sc);
            continue;
        }

        if c == '}' {
            i += 1;
            col += 1;
            push!(Tok::RBrace, sl, sc);
            continue;
        }
        if c == '=' {
            i += 1;
            col += 1;
            push!(Tok::Eq, sl, sc);
            continue;
        }
        if c == ',' {
            i += 1;
            col += 1;
            push!(Tok::Comma, sl, sc);
            continue;
        }
        if c == '-' && i + 1 < bytes.len() && bytes[i + 1] == '>' {
            i += 2;
            col += 2;
            push!(Tok::Arrow, sl, sc);
            continue;
        }

        // number
        if c.is_ascii_digit() {
            let mut n = String::new();
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                n.push(bytes[i]);
                i += 1;
                col += 1;
            }
            let v = n
                .parse::<i64>()
                .map_err(|_| CueError::syntax(Span { line: sl, col: sc }, "number out of range"))?;
            push!(Tok::Num(v), sl, sc);
            continue;
        }

        // word: letters, digits, _ . : # / -
        if c.is_alphanumeric() || c == '_' || c == '.' || c == '#' || c == ':' || c == '/' {
            let mut w = String::new();
            while i < bytes.len() {
                let ch = bytes[i];
                if ch.is_alphanumeric()
                    || ch == '_'
                    || ch == '.'
                    || ch == '#'
                    || ch == ':'
                    || ch == '/'
                    || ch == '-'
                {
                    // `->` must not be swallowed into a word
                    if ch == '-' && i + 1 < bytes.len() && bytes[i + 1] == '>' {
                        break;
                    }
                    w.push(ch);
                    i += 1;
                    col += 1;
                } else {
                    break;
                }
            }
            push!(Tok::Word(w), sl, sc);
            continue;
        }

        return Err(CueError::syntax(
            Span { line: sl, col: sc },
            format!("unexpected character `{c}`"),
        ));
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_a_screen_header() {
        let toks = lex(r#"screen login { title "Log In" color=#58C7F3 }"#).unwrap();
        assert_eq!(toks[0].tok, Tok::Word("screen".into()));
        assert_eq!(toks[1].tok, Tok::Word("login".into()));
        assert_eq!(toks[2].tok, Tok::LBrace);
        assert_eq!(toks[3].tok, Tok::Word("title".into()));
        assert_eq!(toks[4].tok, Tok::Str("Log In".into()));
        assert_eq!(toks[5].tok, Tok::Word("color".into()));
        assert_eq!(toks[6].tok, Tok::Eq);
        assert_eq!(toks[7].tok, Tok::Word("#58C7F3".into()));
    }

    /// `{...}` is a value; `{` alone opens a block. the lexer must not confuse them
    #[test]
    fn distinguishes_interpolation_from_block() {
        let toks = lex("input username fill={player.name}\nbutton \"Go\" { open x }").unwrap();
        assert!(toks
            .iter()
            .any(|t| t.tok == Tok::Expr("player.name".into())));
        assert!(toks.iter().any(|t| t.tok == Tok::LBrace));
    }

    #[test]
    fn tracks_line_numbers_for_errors() {
        let err = lex("screen a {\n  title \"unterminated\n}").unwrap_err();
        assert!(format!("{err}").contains("line 2"), "{err}");
    }

    #[test]
    fn arrow_is_not_part_of_a_word() {
        let toks = lex("on timeout -> stay").unwrap();
        assert_eq!(toks[1].tok, Tok::Word("timeout".into()));
        assert_eq!(toks[2].tok, Tok::Arrow);
    }

    #[test]
    fn comments_are_skipped() {
        let toks = lex("# a comment\nopen x").unwrap();
        assert_eq!(toks[0].tok, Tok::Word("open".into()));
    }
}
