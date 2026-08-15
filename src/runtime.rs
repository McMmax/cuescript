//! render and execute
//!
//! rendering is a loop over ops writing into a caller-owned buffer — no
//! allocation in steady state. execution is a loop over steps; the only step
//! that can await is `call`, and the host decides how to run it, so nothing
//! here can stall a tick thread on its own.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::ast::{ButtonStyle, LineKind};

/// a boxed future — defined here so the crate needs no futures dependency
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
use crate::program::{Cond, Modifier, Op, Program, ScreenId, Step, Text};

/// supplies placeholder values. a host fills exactly `Program::slot_paths()`
///
/// `Sync` so a step list holding one can be a `Send` future — internode runs
/// these on a task, which requires the whole future to cross threads
pub trait Slots: Sync {
    /// value for a slot, or "" when the host has nothing for it
    fn get(&self, path: &str) -> &str;
}

impl Slots for HashMap<String, String> {
    fn get(&self, path: &str) -> &str {
        HashMap::get(self, path).map(String::as_str).unwrap_or("")
    }
}

/// one row from a `lookup`. `None` means no row matched
pub type Row = Option<HashMap<String, String>>;

/// what a `call` returned
pub enum CallResult {
    Ok,
    /// reducer error code, matched against `on <code> ->` handlers
    Err(String),
    Timeout,
}

/// the boundary between the language and the network stack.
///
/// `lookup` is deliberately **synchronous**: it must be a fast in-memory read,
/// so it is safe to run inline while stepping. A host that made it a network
/// round trip would put I/O wherever the step list happens to be running.
///
/// `call` is the authoritative write and the **only** await point in the whole
/// language — which is what lets a host guarantee that nothing reachable from a
/// tick thread can stall.
pub trait Host: Send {
    fn lookup(&self, table: &str, field: &str, value: &str) -> Row;
    fn call<'a>(&'a mut self, effect: &'a str, args: &'a [(String, String)]) -> BoxFuture<'a, CallResult>;
}

/// what the runtime wants done after a button ran
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// re-show the current screen, optionally with an error line
    Stay { message: Option<String> },
    /// show another screen, optionally with a message on it
    Open { screen: ScreenId, message: Option<String> },
    Kick { message: String },
    /// let the player out — internode's limbo release
    Release,
}

/// a screen ready to be turned into whatever the surface speaks
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenView {
    pub title: String,
    pub lines: Vec<LineView>,
    pub inputs: Vec<InputView>,
    pub buttons: Vec<ButtonView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineView {
    pub kind: LineKind,
    pub text: String,
    pub color: Option<String>,
    pub bold: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputView {
    pub key: String,
    pub secret: bool,
    pub initial: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ButtonView {
    pub label: String,
    pub style: ButtonStyle,
    /// index of the button on its screen — what a host echoes back on click
    pub index: u16,
}

/// writes a compiled template into `out`. the hot path: no allocation, no
/// hashing beyond the host's own slot lookup, no string compare
pub fn write(p: &Program, t: &Text, slots: &dyn Slots, out: &mut String) {
    for op in &t.ops {
        match op {
            Op::Literal(r) => out.push_str(p.literal(*r)),
            Op::Slot(i) => out.push_str(slots.get(&p.slots[*i as usize])),
            Op::Modified(i, m) => {
                let v = slots.get(&p.slots[*i as usize]);
                match m {
                    Modifier::Trim => out.push_str(v.trim()),
                    Modifier::Upper => out.extend(v.chars().flat_map(char::to_uppercase)),
                    Modifier::Lower => out.extend(v.chars().flat_map(char::to_lowercase)),
                    // an unknown modifier passes the value through rather than
                    // erroring: a host that has not learned it yet still renders
                    Modifier::Named(_) => out.push_str(v),
                }
            }
        }
    }
}

fn eval(p: &Program, t: &Text, slots: &dyn Slots) -> String {
    let mut s = String::new();
    write(p, t, slots, &mut s);
    s
}

/// builds the view for a screen
pub fn render(p: &Program, screen: ScreenId, slots: &dyn Slots) -> ScreenView {
    let s = &p.screens[screen as usize];
    let mut title = String::new();
    let mut lines = Vec::with_capacity(s.lines.len());

    for l in &s.lines {
        let text = eval(p, &l.text, slots);
        if l.kind == LineKind::Title && title.is_empty() {
            title = text;
            continue;
        }
        lines.push(LineView { kind: l.kind, text, color: l.color.clone(), bold: l.bold });
    }

    ScreenView {
        title,
        lines,
        inputs: s
            .inputs
            .iter()
            .map(|i| InputView {
                key: i.key.clone(),
                secret: i.secret,
                initial: eval(p, &i.fill, slots),
            })
            .collect(),
        buttons: s
            .buttons
            .iter()
            .enumerate()
            .map(|(n, b)| ButtonView { label: eval(p, &b.label, slots), style: b.style, index: n as u16 })
            .collect(),
    }
}

/// values a step list accumulates: `let` bindings, `remember`ed state, and the
/// inputs the player submitted. lookups bind here too, as `<name>.<column>`
#[derive(Default)]
pub struct Scope {
    pub vars: HashMap<String, String>,
}

impl Scope {
    pub fn with_inputs(inputs: HashMap<String, String>) -> Self {
        let mut vars = HashMap::with_capacity(inputs.len() * 2);
        for (k, v) in inputs {
            vars.insert(format!("input.{k}"), v);
        }
        Self { vars }
    }

    pub fn set_state(&mut self, k: &str, v: &str) {
        self.vars.insert(format!("state.{k}"), v.to_string());
    }
}

fn layered<'a>(scope: &'a Scope, outer: &'a dyn Slots) -> Layered<'a> {
    Layered { scope, outer }
}

/// resolves a slot from the scope first, then the host — so `{input.username}`
/// and `{state.password}` work without the host knowing about them
struct Layered<'a> {
    scope: &'a Scope,
    outer: &'a dyn Slots,
}

impl Slots for Layered<'_> {
    fn get(&self, path: &str) -> &str {
        match self.scope.vars.get(path) {
            Some(v) => v.as_str(),
            None => self.outer.get(path),
        }
    }
}

/// runs a button's step list.
///
/// returns as soon as a step decides the outcome; steps after an `open`,
/// `stay`, `kick` or `release` are unreachable and simply not run.
pub async fn run(
    p: &Program,
    steps_id: u16,
    scope: &mut Scope,
    slots: &dyn Slots,
    host: &mut dyn Host,
) -> Outcome {
    exec(p, &p.programs[steps_id as usize], scope, slots, host).await
}

/// boxed because a handler arm re-enters `exec`, and an async fn cannot recurse
/// into itself without an indirection
fn exec<'a>(
    p: &'a Program,
    steps: &'a [Step],
    scope: &'a mut Scope,
    slots: &'a dyn Slots,
    host: &'a mut dyn Host,
) -> BoxFuture<'a, Outcome> {
    Box::pin(async move {
    for step in steps {
        match step {
            Step::Let { name, value } => {
                let v = eval(p, value, &layered(scope, slots));
                scope.vars.insert(name.clone(), v);
            }
            Step::Remember { name, value } => {
                let v = eval(p, value, &layered(scope, slots));
                scope.vars.insert(name.clone(), v.clone());
                scope.set_state(name, &v);
            }
            Step::Require { cond, message } => {
                if !test(p, cond, scope, slots) {
                    return Outcome::Stay { message: Some(eval(p, message, &layered(scope, slots))) };
                }
            }
            Step::Lookup { table, field, value, bind } => {
                let v = eval(p, value, &layered(scope, slots));
                match host.lookup(table, field, &v) {
                    Some(row) => {
                        // a hit binds `<name>` truthy plus every column
                        scope.vars.insert(bind.clone(), "1".into());
                        for (col, val) in row {
                            scope.vars.insert(format!("{bind}.{col}"), val);
                        }
                    }
                    None => {
                        scope.vars.insert(bind.clone(), String::new());
                    }
                }
            }
            Step::Call { effect, args, handlers } => {
                let resolved: Vec<(String, String)> = args
                    .iter()
                    .map(|(k, t)| (k.clone(), eval(p, t, &layered(scope, slots))))
                    .collect();
                let code = match host.call(effect, &resolved).await {
                    CallResult::Ok => None,
                    CallResult::Err(c) => Some(c),
                    CallResult::Timeout => Some("timeout".to_string()),
                };
                if let Some(code) = code {
                    let arm = handlers
                        .iter()
                        .find(|h| h.code == code)
                        .or_else(|| handlers.iter().find(|h| h.code == "*"));
                    return match arm {
                        Some(h) => exec(p, &h.actions, scope, slots, host).await,
                        // an unhandled failure must never fall through into the
                        // success path — that would advance as if it worked
                        None => Outcome::Stay { message: Some(code) },
                    };
                }
            }
            // a `say` before a navigation is the message shown on arrival
            Step::Open(id) => {
                return Outcome::Open { screen: *id, message: scope.vars.remove("__say") }
            }
            Step::Stay(m) => return Outcome::Stay { message: Some(eval(p, m, &layered(scope, slots))) },
            Step::Say(m) => {
                // `say` does not decide the outcome; it annotates whichever
                // navigation follows, so it is stashed rather than emitted.
                // it must therefore come BEFORE the `open` it belongs to
                let v = eval(p, m, &layered(scope, slots));
                scope.vars.insert("__say".into(), v);
            }
            Step::Kick(m) => return Outcome::Kick { message: eval(p, m, &layered(scope, slots)) },
            Step::Release => return Outcome::Release,
        }
    }

        // ran off the end without deciding: stay put, carrying any `say`
        Outcome::Stay { message: scope.vars.remove("__say") }
    })
}

fn test(p: &Program, c: &Cond, scope: &Scope, slots: &dyn Slots) -> bool {
    let ctx = Layered { scope, outer: slots };
    match c {
        Cond::Matches { value, re } => re.is_match(&eval(p, value, &ctx)),
        Cond::IsEmpty { value, negated } => {
            let empty = eval(p, value, &ctx).is_empty();
            empty != *negated
        }
        Cond::Equals { left, right, negated } => {
            let eq = eval(p, left, &ctx) == eval(p, right, &ctx);
            eq != *negated
        }
        Cond::NoWhitespace { value } => !eval(p, value, &ctx).chars().any(char::is_whitespace),
        Cond::LengthBetween { value, min, max } => {
            let n = eval(p, value, &ctx).chars().count();
            n >= *min && n <= *max
        }
    }
}

/// resolves the outcome of a run into the screen to show and the message on it
pub fn settle(outcome: &Outcome, current: ScreenId) -> (ScreenId, Option<String>) {
    match outcome {
        Outcome::Stay { message } => (current, message.clone()),
        Outcome::Open { screen, message } => (*screen, message.clone()),
        _ => (current, None),
    }
}
