//! recursive-descent parser — the grammar is small and flat on purpose, so a
//! screen reads top-to-bottom and errors point at one line

use crate::ast::*;
use crate::error::{CueError, Span};
use crate::lex::{lex, Tok, Token};

pub fn parse(src: &str) -> Result<Document, CueError> {
    let toks = lex(src)?;
    Parser { toks, pos: 0 }.document()
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn span(&self) -> Span {
        self.toks
            .get(self.pos)
            .or_else(|| self.toks.last())
            .map(|t| t.span)
            .unwrap_or_default()
    }

    fn bump(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Word(x)) if x == w) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_word(&mut self) -> Result<String, CueError> {
        let span = self.span();
        match self.bump().map(|t| t.tok) {
            Some(Tok::Word(w)) => Ok(w),
            other => Err(CueError::syntax(
                span,
                format!("expected a name, found {}", describe(&other)),
            )),
        }
    }

    fn expect_keyword(&mut self, w: &str) -> Result<(), CueError> {
        let span = self.span();
        if self.eat_word(w) {
            Ok(())
        } else {
            let found = self.peek().cloned();
            Err(CueError::syntax(
                span,
                format!("expected `{w}`, found {}", describe(&found)),
            ))
        }
    }

    fn expect(&mut self, t: Tok) -> Result<(), CueError> {
        let span = self.span();
        if self.peek() == Some(&t) {
            self.pos += 1;
            Ok(())
        } else {
            let found = self.peek().cloned();
            Err(CueError::syntax(
                span,
                format!(
                    "expected {}, found {}",
                    describe(&Some(t)),
                    describe(&found)
                ),
            ))
        }
    }

    /// a string literal, or a bare `{expr}` treated as a one-placeholder template
    fn expect_text(&mut self) -> Result<String, CueError> {
        let span = self.span();
        match self.bump().map(|t| t.tok) {
            Some(Tok::Str(s)) => Ok(s),
            Some(Tok::Expr(e)) => Ok(format!("{{{e}}}")),
            other => Err(CueError::syntax(
                span,
                format!("expected text, found {}", describe(&other)),
            )),
        }
    }

    /// a value position: string, `{expr}`, number, or bare word.
    ///
    /// a bare word is a VARIABLE reference, not a literal — `require existing
    /// is empty` must read the binding, and treating it as the text "existing"
    /// would make the condition trivially false
    fn expect_value(&mut self) -> Result<String, CueError> {
        let span = self.span();
        match self.bump().map(|t| t.tok) {
            Some(Tok::Str(s)) => Ok(s),
            Some(Tok::Expr(e)) => Ok(format!("{{{e}}}")),
            Some(Tok::Num(n)) => Ok(n.to_string()),
            Some(Tok::Word(w)) => Ok(format!("{{{w}}}")),
            other => Err(CueError::syntax(
                span,
                format!("expected a value, found {}", describe(&other)),
            )),
        }
    }

    /// a value that stays literal — colours and numeric bounds, never variables
    fn expect_literal(&mut self) -> Result<String, CueError> {
        let span = self.span();
        match self.bump().map(|t| t.tok) {
            Some(Tok::Str(s)) => Ok(s),
            Some(Tok::Num(n)) => Ok(n.to_string()),
            Some(Tok::Word(w)) => Ok(w),
            other => Err(CueError::syntax(
                span,
                format!("expected a value, found {}", describe(&other)),
            )),
        }
    }

    fn document(&mut self) -> Result<Document, CueError> {
        let mut fragments = Vec::new();
        let mut flow = None;

        while self.peek().is_some() {
            let span = self.span();
            if self.eat_word("fragment") {
                let name = self.expect_word()?;
                self.expect(Tok::LBrace)?;
                let items = self.items()?;
                self.expect(Tok::RBrace)?;
                fragments.push(Fragment { name, items, span });
            } else if self.eat_word("flow") {
                if flow.is_some() {
                    return Err(CueError::syntax(span, "only one flow per document"));
                }
                flow = Some(self.flow(span)?);
            } else {
                let found = self.peek().cloned();
                return Err(CueError::syntax(
                    span,
                    format!("expected `fragment` or `flow`, found {}", describe(&found)),
                ));
            }
        }

        flow.map(|flow| Document { fragments, flow })
            .ok_or_else(|| CueError::syntax(Span::default(), "document has no `flow`"))
    }

    fn flow(&mut self, span: Span) -> Result<Flow, CueError> {
        let name = self.expect_word()?;
        self.expect(Tok::LBrace)?;

        let mut entry = None;
        let mut entry_span = Span::default();
        let mut screens = Vec::new();

        loop {
            let s = self.span();
            match self.peek() {
                Some(Tok::RBrace) | None => break,
                _ => {}
            }
            if self.eat_word("entry") {
                entry_span = s;
                entry = Some(self.expect_word()?);
            } else if self.eat_word("screen") {
                let sname = self.expect_word()?;
                self.expect(Tok::LBrace)?;
                let items = self.items()?;
                self.expect(Tok::RBrace)?;
                screens.push(Screen {
                    name: sname,
                    items,
                    span: s,
                });
            } else {
                let found = self.peek().cloned();
                return Err(CueError::syntax(
                    s,
                    format!("expected `entry` or `screen`, found {}", describe(&found)),
                ));
            }
        }
        self.expect(Tok::RBrace)?;

        let entry = entry.ok_or_else(|| CueError::syntax(span, "flow has no `entry` screen"))?;
        Ok(Flow {
            name,
            entry,
            entry_span,
            screens,
            span,
        })
    }

    fn items(&mut self) -> Result<Vec<Item>, CueError> {
        let mut items = Vec::new();
        loop {
            let span = self.span();
            match self.peek() {
                Some(Tok::RBrace) | None => break,
                _ => {}
            }

            if self.eat_word("include") {
                items.push(Item::Include {
                    name: self.expect_word()?,
                    span,
                });
                continue;
            }

            if let Some(kind) = self.line_kind() {
                let text = self.expect_text()?;
                let (color, bold) = self.line_attrs()?;
                items.push(Item::Line {
                    kind,
                    text,
                    color,
                    bold,
                    span,
                });
                continue;
            }

            if self.eat_word("step") {
                let current = self.expect_num()?;
                self.expect_keyword("of")?;
                let total = self.expect_num()?;
                items.push(Item::Step {
                    current,
                    total,
                    span,
                });
                continue;
            }

            if self.eat_word("input") {
                let key = self.expect_word()?;
                let mut secret = false;
                let mut fill = None;
                loop {
                    if self.eat_word("secret") {
                        secret = true;
                    } else if self.eat_word("fill") {
                        self.expect(Tok::Eq)?;
                        fill = Some(self.expect_value()?);
                    } else {
                        break;
                    }
                }
                items.push(Item::Input {
                    key,
                    secret,
                    fill,
                    span,
                });
                continue;
            }

            if self.eat_word("button") {
                let label = self.expect_text()?;
                let style = if self.eat_word("primary") {
                    ButtonStyle::Primary
                } else if self.eat_word("quiet") {
                    ButtonStyle::Quiet
                } else {
                    let _ = self.eat_word("secondary");
                    ButtonStyle::Secondary
                };
                self.expect(Tok::LBrace)?;
                let steps = self.stmts()?;
                self.expect(Tok::RBrace)?;
                items.push(Item::Button {
                    label,
                    style,
                    steps,
                    span,
                });
                continue;
            }

            let found = self.peek().cloned();
            return Err(CueError::syntax(
                span,
                format!("unknown screen item {}", describe(&found)),
            ));
        }
        Ok(items)
    }

    fn line_kind(&mut self) -> Option<LineKind> {
        for (w, k) in [
            ("title", LineKind::Title),
            ("text", LineKind::Text),
            ("hint", LineKind::Hint),
            ("good", LineKind::Good),
            ("rule", LineKind::Rule),
        ] {
            if self.eat_word(w) {
                return Some(k);
            }
        }
        None
    }

    fn line_attrs(&mut self) -> Result<(Option<String>, bool), CueError> {
        let mut color = None;
        let mut bold = false;
        loop {
            if self.eat_word("color") {
                self.expect(Tok::Eq)?;
                color = Some(self.expect_literal()?);
            } else if self.eat_word("bold") {
                bold = true;
            } else {
                break;
            }
        }
        Ok((color, bold))
    }

    fn expect_num(&mut self) -> Result<i64, CueError> {
        let span = self.span();
        match self.bump().map(|t| t.tok) {
            Some(Tok::Num(n)) => Ok(n),
            other => Err(CueError::syntax(
                span,
                format!("expected a number, found {}", describe(&other)),
            )),
        }
    }

    fn stmts(&mut self) -> Result<Vec<Stmt>, CueError> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::RBrace) | None => break,
                _ => {}
            }
            out.push(self.stmt()?);
        }
        Ok(out)
    }

    fn stmt(&mut self) -> Result<Stmt, CueError> {
        let span = self.span();

        if self.eat_word("let") {
            let name = self.expect_word()?;
            self.expect(Tok::Eq)?;
            let value = self.expect_value()?;
            return Ok(Stmt::Let { name, value, span });
        }

        if self.eat_word("remember") {
            let name = self.expect_word()?;
            self.expect(Tok::Eq)?;
            let value = self.expect_value()?;
            return Ok(Stmt::Remember { name, value, span });
        }

        if self.eat_word("require") {
            let cond = self.cond()?;
            self.expect_keyword("else")?;
            let message = self.expect_text()?;
            return Ok(Stmt::Require {
                cond,
                message,
                span,
            });
        }

        if self.eat_word("lookup") {
            let table = self.expect_word()?;
            self.expect_keyword("by")?;
            let field = self.expect_word()?;
            self.expect(Tok::Eq)?;
            let value = self.expect_value()?;
            self.expect(Tok::Arrow)?;
            let bind = self.expect_word()?;
            return Ok(Stmt::Lookup {
                table,
                field,
                value,
                bind,
                span,
            });
        }

        if self.eat_word("call") {
            let effect = self.expect_word()?;
            let mut args = Vec::new();
            if self.eat_word("with") {
                loop {
                    let k = self.expect_word()?;
                    self.expect(Tok::Eq)?;
                    let v = self.expect_value()?;
                    args.push((k, v));
                    if self.peek() == Some(&Tok::Comma) {
                        self.pos += 1;
                        continue;
                    }
                    break;
                }
            }
            let mut handlers = Vec::new();
            while matches!(self.peek(), Some(Tok::Word(w)) if w == "on") {
                let hspan = self.span();
                self.pos += 1;
                let code = self.expect_word()?;
                self.expect(Tok::Arrow)?;
                // an un-braced arm is exactly ONE statement. without that rule a
                // step written after the last handler is swallowed into it —
                // which silently drops the success path of a `call`
                let actions = if self.peek() == Some(&Tok::LBrace) {
                    self.pos += 1;
                    let a = self.stmts()?;
                    self.expect(Tok::RBrace)?;
                    a
                } else {
                    vec![self.stmt()?]
                };
                handlers.push(Handler {
                    code,
                    actions,
                    span: hspan,
                });
            }
            return Ok(Stmt::Call {
                effect,
                args,
                handlers,
                span,
            });
        }

        if self.eat_word("open") {
            return Ok(Stmt::Open {
                screen: self.expect_word()?,
                span,
            });
        }
        if self.eat_word("stay") {
            return Ok(Stmt::Stay {
                message: self.expect_text()?,
                span,
            });
        }
        if self.eat_word("say") {
            return Ok(Stmt::Say {
                message: self.expect_text()?,
                span,
            });
        }
        if self.eat_word("kick") {
            return Ok(Stmt::Kick {
                message: self.expect_text()?,
                span,
            });
        }
        if self.eat_word("release") {
            return Ok(Stmt::Release { span });
        }

        let found = self.peek().cloned();
        Err(CueError::syntax(
            span,
            format!("unknown step {}", describe(&found)),
        ))
    }

    fn cond(&mut self) -> Result<Cond, CueError> {
        let span = self.span();
        let value = self.expect_value()?;

        if self.eat_word("matches") {
            let pattern = self.expect_text()?;
            return Ok(Cond::Matches { value, pattern });
        }

        if self.eat_word("has") {
            self.expect_keyword("no")?;
            self.expect_keyword("whitespace")?;
            return Ok(Cond::NoWhitespace { value });
        }

        if self.eat_word("length") {
            self.expect_keyword("between")?;
            let min = self.expect_literal()?;
            self.expect_keyword("and")?;
            let max = self.expect_literal()?;
            return Ok(Cond::LengthBetween { value, min, max });
        }

        if self.eat_word("is") {
            let negated = self.eat_word("not");
            if self.eat_word("empty") {
                return Ok(Cond::IsEmpty { value, negated });
            }
            let right = self.expect_value()?;
            return Ok(Cond::Equals {
                left: value,
                right,
                negated,
            });
        }

        let found = self.peek().cloned();
        Err(CueError::syntax(
            span,
            format!(
                "expected a condition after the value, found {}",
                describe(&found)
            ),
        ))
    }
}

fn describe(t: &Option<Tok>) -> String {
    match t {
        None => "end of file".into(),
        Some(Tok::Word(w)) => format!("`{w}`"),
        Some(Tok::Str(_)) => "a string".into(),
        Some(Tok::Num(n)) => format!("`{n}`"),
        Some(Tok::Expr(e)) => format!("`{{{e}}}`"),
        Some(Tok::LBrace) => "`{`".into(),
        Some(Tok::RBrace) => "`}`".into(),
        Some(Tok::Eq) => "`=`".into(),
        Some(Tok::Comma) => "`,`".into(),
        Some(Tok::Arrow) => "`->`".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI: &str = r#"
flow auth {
    entry welcome
    screen welcome {
        title "Welcome" color=#58C7F3 bold
        hint "It only takes a few seconds."
        button "Create an Account" primary { open register }
    }
    screen register {
        input username fill={player.offline_name}
        button "Check" primary {
            let name = {input.username|trim}
            require name matches "^[A-Za-z0-9_]{1,16}$"
                else "Use 1-16 letters."
            lookup player_core by username_lower = name -> existing
            require existing is empty
                else "{name} is taken."
            remember chosen = name
            open welcome
        }
    }
}
"#;

    #[test]
    fn parses_a_flow() {
        let doc = parse(MINI).unwrap();
        assert_eq!(doc.flow.entry, "welcome");
        assert_eq!(doc.flow.screens.len(), 2);
    }

    #[test]
    fn parses_steps_in_order() {
        let doc = parse(MINI).unwrap();
        let reg = &doc.flow.screens[1];
        let Item::Button { steps, .. } = reg
            .items
            .iter()
            .find(|i| matches!(i, Item::Button { .. }))
            .unwrap()
        else {
            unreachable!()
        };
        assert!(matches!(steps[0], Stmt::Let { .. }));
        assert!(matches!(steps[1], Stmt::Require { .. }));
        assert!(matches!(steps[2], Stmt::Lookup { .. }));
        assert!(matches!(steps[4], Stmt::Remember { .. }));
        assert!(matches!(steps[5], Stmt::Open { .. }));
    }

    #[test]
    fn parses_call_handlers() {
        let doc = parse(
            r#"
flow f { entry s
  screen s { button "Go" { call player_register with username = {state.u}, password = {state.p}
      on username_taken -> { say "taken" open s }
      on timeout -> stay "timed out"
      open s } } }
"#,
        )
        .unwrap();
        let Item::Button { steps, .. } = &doc.flow.screens[0].items[0] else {
            unreachable!()
        };
        let Stmt::Call { args, handlers, .. } = &steps[0] else {
            panic!("expected call, got {:?}", steps[0])
        };
        assert_eq!(args.len(), 2);
        assert_eq!(handlers.len(), 2);
        assert_eq!(handlers[0].code, "username_taken");
        assert_eq!(
            handlers[0].actions.len(),
            2,
            "a braced arm holds several steps"
        );
        assert_eq!(
            handlers[1].actions.len(),
            1,
            "an un-braced arm is exactly one step"
        );
        assert!(
            matches!(steps.last(), Some(Stmt::Open { .. })),
            "the step after the handlers belongs to the button, not the last arm"
        );
        assert_eq!(handlers[1].code, "timeout");
    }

    #[test]
    fn reports_the_line_of_a_bad_step() {
        let err = parse("flow f { entry s\n screen s { button \"x\" {\n   frobnicate\n } } }")
            .unwrap_err();
        assert!(format!("{err}").contains("line 3"), "{err}");
    }

    #[test]
    fn requires_an_entry() {
        let err = parse("flow f { screen s { title \"t\" } }").unwrap_err();
        assert!(format!("{err}").contains("entry"), "{err}");
    }
}
