//! AST → Program
//!
//! this is where the cost goes. after compiling: no name lookups, no regex
//! construction, no template parsing, no fragment indirection. what is left is
//! indices into flat vectors.

use std::collections::HashMap;

use regex_lite::Regex;
use smallvec::SmallVec;

use crate::ast::{self, Item, LineKind, Stmt};
use crate::error::{CueError, Span};
use crate::program::{self as prog, Modifier, Op, Program, ScreenId, SlotId, Text};

pub fn compile(doc: &ast::Document) -> Result<Program, CueError> {
    let mut c = Compiler {
        text: String::with_capacity(1024),
        slots: Vec::new(),
        slot_index: HashMap::new(),
        fragments: HashMap::new(),
    };

    for f in &doc.fragments {
        if c.fragments.insert(f.name.clone(), f.items.clone()).is_some() {
            return Err(CueError::resolve(f.span, format!("fragment `{}` defined twice", f.name)));
        }
    }

    // screen order fixes ids, so `open` can resolve to an index in one pass after
    let mut order: HashMap<String, ScreenId> = HashMap::with_capacity(doc.flow.screens.len());
    for (i, s) in doc.flow.screens.iter().enumerate() {
        if i > u16::MAX as usize {
            return Err(CueError::resolve(s.span, "too many screens"));
        }
        if order.insert(s.name.clone(), i as ScreenId).is_some() {
            return Err(CueError::resolve(s.span, format!("screen `{}` defined twice", s.name)));
        }
    }

    let entry = *order
        .get(&doc.flow.entry)
        .ok_or_else(|| CueError::resolve(doc.flow.entry_span, format!("entry screen `{}` does not exist", doc.flow.entry)))?;

    let mut screens = Vec::with_capacity(doc.flow.screens.len());
    let mut programs: Vec<Vec<prog::Step>> = Vec::new();

    for s in &doc.flow.screens {
        let mut screen = prog::Screen { name: s.name.clone(), ..Default::default() };
        let items = c.expand(&s.items, s.span)?;
        for item in &items {
            match item {
                Item::Include { .. } => unreachable!("expand() inlines includes"),
                Item::Line { kind, text, color, bold, .. } => screen.lines.push(prog::Line {
                    kind: *kind,
                    text: c.text(text),
                    color: color.clone(),
                    bold: *bold,
                }),
                Item::Step { current, total, .. } => screen.lines.push(prog::Line {
                    kind: LineKind::Hint,
                    text: c.text(&format!("Step {current} of {total}")),
                    color: None,
                    bold: false,
                }),
                Item::Input { key, secret, fill, .. } => screen.inputs.push(prog::Input {
                    key: key.clone(),
                    secret: *secret,
                    fill: fill.as_deref().map(|f| c.text(f)).unwrap_or_default(),
                }),
                Item::Button { label, style, steps, span } => {
                    let compiled = c.steps(steps, &order, *span)?;
                    programs.push(compiled);
                    screen.buttons.push(prog::Button {
                        label: c.text(label),
                        style: *style,
                        steps: (programs.len() - 1) as u16,
                    });
                }
            }
        }

        // a screen with no way out strands the player: the dialog cannot be
        // dismissed, so there would be no path back to any other screen
        if screen.buttons.is_empty() {
            return Err(CueError::resolve(
                s.span,
                format!("screen `{}` has no buttons — the player could never leave it", s.name),
            ));
        }
        screens.push(screen);
    }

    let program = Program {
        flow: doc.flow.name.clone(),
        entry,
        screens,
        programs,
        slots: c.slots,
        text: c.text,
    };

    check_reachable(&program, &doc.flow)?;
    Ok(program)
}

/// a screen nothing opens can never be shown — almost always an editing slip
fn check_reachable(p: &Program, flow: &ast::Flow) -> Result<(), CueError> {
    let mut seen = vec![false; p.screens.len()];
    let mut stack = vec![p.entry as usize];
    while let Some(i) = stack.pop() {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        for b in &p.screens[i].buttons {
            collect_opens(&p.programs[b.steps as usize], &mut stack);
        }
    }
    for (i, hit) in seen.iter().enumerate() {
        if !hit {
            let span = flow.screens.get(i).map(|s| s.span).unwrap_or(flow.span);
            return Err(CueError::resolve(
                span,
                format!("screen `{}` is unreachable from `{}`", p.screens[i].name, p.screens[p.entry as usize].name),
            ));
        }
    }
    Ok(())
}

fn collect_opens(steps: &[prog::Step], out: &mut Vec<usize>) {
    for s in steps {
        match s {
            prog::Step::Open(id) => out.push(*id as usize),
            prog::Step::Call { handlers, .. } => {
                for h in handlers {
                    collect_opens(&h.actions, out);
                }
            }
            _ => {}
        }
    }
}

struct Compiler {
    text: String,
    slots: Vec<String>,
    slot_index: HashMap<String, SlotId>,
    fragments: HashMap<String, Vec<Item>>,
}

impl Compiler {
    /// inlines `include`, so a fragment costs nothing at render time
    fn expand(&self, items: &[Item], span: Span) -> Result<Vec<Item>, CueError> {
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            match it {
                Item::Include { name, span } => {
                    let f = self
                        .fragments
                        .get(name)
                        .ok_or_else(|| CueError::resolve(*span, format!("no fragment named `{name}`")))?;
                    // one level: a fragment including a fragment would need cycle
                    // detection, and nothing needs it yet
                    for inner in f {
                        if let Item::Include { name, span } = inner {
                            return Err(CueError::resolve(
                                *span,
                                format!("fragment `{name}` is included from another fragment; nesting is not supported"),
                            ));
                        }
                        out.push(inner.clone());
                    }
                }
                other => out.push(other.clone()),
            }
        }
        let _ = span;
        Ok(out)
    }

    fn intern(&mut self, s: &str) -> (u32, u32) {
        // literals repeat across screens (rules, spacers); reuse the blob
        if let Some(at) = self.text.find(s) {
            return (at as u32, s.len() as u32);
        }
        let at = self.text.len() as u32;
        self.text.push_str(s);
        (at, s.len() as u32)
    }

    fn slot(&mut self, path: &str) -> SlotId {
        if let Some(id) = self.slot_index.get(path) {
            return *id;
        }
        let id = self.slots.len() as SlotId;
        self.slots.push(path.to_string());
        self.slot_index.insert(path.to_string(), id);
        id
    }

    /// parses `{path|modifier}` interpolation into flat ops
    fn text(&mut self, src: &str) -> Text {
        let mut ops: SmallVec<[Op; 4]> = SmallVec::new();
        let mut lit = String::new();
        let mut rest = src;

        while let Some(open) = rest.find('{') {
            if rest[..open].ends_with('\\') {
                lit.push_str(&rest[..open - 1]);
                lit.push('{');
                rest = &rest[open + 1..];
                continue;
            }
            let Some(close) = rest[open..].find('}') else { break };
            lit.push_str(&rest[..open]);
            let inner = &rest[open + 1..open + close];
            rest = &rest[open + close + 1..];

            if !lit.is_empty() {
                let r = self.intern(&lit);
                ops.push(Op::Literal(r));
                lit.clear();
            }

            let (path, modifier) = match inner.split_once('|') {
                Some((p, m)) => (p.trim(), Some(m.trim())),
                None => (inner.trim(), None),
            };
            let slot = self.slot(path);
            match modifier {
                None => ops.push(Op::Slot(slot)),
                Some(m) => ops.push(Op::Modified(slot, parse_modifier(m))),
            }
        }

        lit.push_str(rest);
        if !lit.is_empty() {
            let r = self.intern(&lit);
            ops.push(Op::Literal(r));
        }

        // merge adjacent literals so `"a" "b"` is one memcpy, not two
        let mut merged: SmallVec<[Op; 4]> = SmallVec::new();
        for op in ops {
            match (merged.last(), &op) {
                (Some(Op::Literal(a)), Op::Literal(b)) if a.0 + a.1 == b.0 => {
                    let a = *a;
                    merged.pop();
                    merged.push(Op::Literal((a.0, a.1 + b.1)));
                }
                _ => merged.push(op),
            }
        }

        Text { ops: merged }
    }

    fn steps(
        &mut self,
        steps: &[Stmt],
        order: &HashMap<String, ScreenId>,
        span: Span,
    ) -> Result<Vec<prog::Step>, CueError> {
        let mut out = Vec::with_capacity(steps.len());
        for s in steps {
            out.push(self.step(s, order)?);
        }
        let _ = span;
        Ok(out)
    }

    fn step(&mut self, s: &Stmt, order: &HashMap<String, ScreenId>) -> Result<prog::Step, CueError> {
        Ok(match s {
            Stmt::Let { name, value, .. } => prog::Step::Let { name: name.clone(), value: self.text(value) },
            Stmt::Remember { name, value, .. } => {
                prog::Step::Remember { name: name.clone(), value: self.text(value) }
            }
            Stmt::Require { cond, message, .. } => {
                prog::Step::Require { cond: self.cond(cond)?, message: self.text(message) }
            }
            Stmt::Lookup { table, field, value, bind, .. } => prog::Step::Lookup {
                table: table.clone(),
                field: field.clone(),
                value: self.text(value),
                bind: bind.clone(),
            },
            Stmt::Call { effect, args, handlers, .. } => {
                let args = args.iter().map(|(k, v)| (k.clone(), self.text(v))).collect();
                let mut hs = Vec::with_capacity(handlers.len());
                for h in handlers {
                    let mut actions = Vec::with_capacity(h.actions.len());
                    for a in &h.actions {
                        actions.push(self.step(a, order)?);
                    }
                    hs.push(prog::Handler { code: h.code.clone(), actions });
                }
                prog::Step::Call { effect: effect.clone(), args, handlers: hs }
            }
            Stmt::Open { screen, span } => {
                let id = order
                    .get(screen)
                    .ok_or_else(|| CueError::resolve(*span, format!("`open {screen}` — no such screen")))?;
                prog::Step::Open(*id)
            }
            Stmt::Stay { message, .. } => prog::Step::Stay(self.text(message)),
            Stmt::Say { message, .. } => prog::Step::Say(self.text(message)),
            Stmt::Kick { message, .. } => prog::Step::Kick(self.text(message)),
            Stmt::Release { .. } => prog::Step::Release,
        })
    }

    fn cond(&mut self, c: &ast::Cond) -> Result<prog::Cond, CueError> {
        Ok(match c {
            // compiled once, at publish — never per evaluation
            ast::Cond::Matches { value, pattern } => prog::Cond::Matches {
                value: self.text(value),
                re: Regex::new(pattern).map_err(|e| CueError::Regex(e.to_string()))?,
            },
            ast::Cond::IsEmpty { value, negated } => {
                prog::Cond::IsEmpty { value: self.text(value), negated: *negated }
            }
            ast::Cond::Equals { left, right, negated } => prog::Cond::Equals {
                left: self.text(left),
                right: self.text(right),
                negated: *negated,
            },
            ast::Cond::NoWhitespace { value } => prog::Cond::NoWhitespace { value: self.text(value) },
            ast::Cond::LengthBetween { value, min, max } => prog::Cond::LengthBetween {
                value: self.text(value),
                min: min.parse().unwrap_or(0),
                max: max.parse().unwrap_or(usize::MAX),
            },
        })
    }
}

fn parse_modifier(m: &str) -> Modifier {
    match m {
        "trim" => Modifier::Trim,
        "upper" => Modifier::Upper,
        "lower" => Modifier::Lower,
        other => Modifier::Named(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn compile_src(src: &str) -> Result<Program, CueError> {
        compile(&parse(src).unwrap())
    }

    #[test]
    fn resolves_open_to_an_index() {
        let p = compile_src(
            r#"flow f { entry a
                screen a { button "go" { open b } }
                screen b { button "back" { open a } } }"#,
        )
        .unwrap();
        assert_eq!(p.entry, 0);
        let Some(prog::Step::Open(id)) = p.programs[0].first() else { panic!() };
        assert_eq!(*id, 1);
    }

    #[test]
    fn rejects_open_to_a_missing_screen() {
        let err = compile_src(r#"flow f { entry a screen a { button "go" { open nowhere } } }"#).unwrap_err();
        assert!(format!("{err}").contains("no such screen"), "{err}");
    }

    #[test]
    fn rejects_an_unreachable_screen() {
        let err = compile_src(
            r#"flow f { entry a
                screen a { button "go" { stay "x" } }
                screen orphan { button "b" { open a } } }"#,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("unreachable"), "{err}");
    }

    #[test]
    fn rejects_a_screen_with_no_way_out() {
        let err = compile_src(r#"flow f { entry a screen a { title "stuck" } }"#).unwrap_err();
        assert!(format!("{err}").contains("no buttons"), "{err}");
    }

    #[test]
    fn interns_and_dedupes_literals() {
        let p = compile_src(
            r#"flow f { entry a
                screen a { text "same" text "same" button "b" { stay "x" } } }"#,
        )
        .unwrap();
        // the repeated literal is stored once
        assert_eq!(p.text.matches("same").count(), 1);
    }

    #[test]
    fn placeholders_become_slots() {
        let p = compile_src(
            r#"flow f { entry a
                screen a { text "hi {player.name} and {player.name}" button "b" { stay "x" } } }"#,
        )
        .unwrap();
        assert_eq!(p.slots, vec!["player.name".to_string()], "same path shares one slot");
    }

    #[test]
    fn inlines_fragments() {
        let p = compile_src(
            r#"
            fragment header { text "H" }
            flow f { entry a screen a { include header text "body" button "b" { stay "x" } } }"#,
        )
        .unwrap();
        assert_eq!(p.screens[0].lines.len(), 2);
    }

    #[test]
    fn merges_adjacent_literals() {
        let p = compile_src(
            r#"flow f { entry a screen a { text "abc" button "b" { stay "x" } } }"#,
        )
        .unwrap();
        assert_eq!(p.screens[0].lines[0].text.ops.len(), 1);
    }
}
