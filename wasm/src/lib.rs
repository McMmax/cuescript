//! browser bindings — the panel runs the SAME compiler and renderer as the proxy
//!
//! This is the point of the whole exercise: a preview that approximates would
//! let staff design against a lie, and a second linter written in TypeScript
//! would drift from the language on its first change. Both problems disappear
//! when there is one implementation compiled to two targets.
//!
//! What is exact here: parsing, compilation, every diagnostic, placeholder
//! substitution, which lines a screen has, which buttons and inputs it carries,
//! and which screen a button leads to. What is NOT exact is pixel appearance —
//! the panel draws HTML, the client draws a Minecraft dialog.

use std::collections::HashMap;

use serde::Serialize;
use wasm_bindgen::prelude::*;

#[derive(Serialize)]
struct Diagnostic {
    line: u32,
    col: u32,
    message: String,
    severity: &'static str,
}

#[derive(Serialize)]
struct CheckResult {
    ok: bool,
    diagnostics: Vec<Diagnostic>,
    /// screen names in declaration order, for the outline
    screens: Vec<String>,
    entry: Option<String>,
}

/// compiles the source and reports what the proxy would report
#[wasm_bindgen]
pub fn check(src: &str) -> String {
    let result = match cuescript::build(src) {
        Ok(p) => CheckResult {
            ok: true,
            diagnostics: Vec::new(),
            screens: p.screens.iter().map(|s| s.name.clone()).collect(),
            entry: p.screens.get(p.entry as usize).map(|s| s.name.clone()),
        },
        Err(e) => {
            let (line, col, message) = describe(&e);
            CheckResult {
                ok: false,
                diagnostics: vec![Diagnostic { line, col, message, severity: "error" }],
                screens: Vec::new(),
                entry: None,
            }
        }
    };
    serde_json::to_string(&result).unwrap_or_else(|_| "{\"ok\":false,\"diagnostics\":[]}".into())
}

#[derive(Serialize)]
struct LineOut {
    kind: String,
    text: String,
    color: Option<String>,
    bold: bool,
}

#[derive(Serialize)]
struct InputOut {
    key: String,
    secret: bool,
    initial: String,
}

#[derive(Serialize)]
struct ButtonOut {
    label: String,
    style: String,
    index: u16,
    /// where this button leads, when it is pure navigation — so the preview can
    /// be clicked through exactly as a player would
    goes_to: Option<String>,
}

#[derive(Serialize)]
struct ScreenOut {
    name: String,
    title: String,
    lines: Vec<LineOut>,
    inputs: Vec<InputOut>,
    buttons: Vec<ButtonOut>,
}

/// renders one screen with the supplied placeholder values.
///
/// `slots_json` is `{ "player.offline_name": "Steve", ... }` — the panel passes
/// a sample player so `{placeholders}` resolve to something real rather than
/// being shown raw.
#[wasm_bindgen]
pub fn preview(src: &str, screen: &str, slots_json: &str) -> String {
    let Ok(program) = cuescript::build(src) else {
        return "null".into();
    };
    let slots: HashMap<String, String> = serde_json::from_str(slots_json).unwrap_or_default();

    let id = if screen.is_empty() {
        program.entry
    } else {
        program.screen_by_name(screen).unwrap_or(program.entry)
    };

    let view = cuescript::render(&program, id, &slots);
    let steps_of = |b: usize| program.screens[id as usize].buttons[b].steps;

    let out = ScreenOut {
        name: program.screens[id as usize].name.clone(),
        title: view.title,
        lines: view
            .lines
            .iter()
            .map(|l| LineOut {
                kind: format!("{:?}", l.kind).to_lowercase(),
                text: l.text.clone(),
                color: l.color.clone(),
                bold: l.bold,
            })
            .collect(),
        inputs: view
            .inputs
            .iter()
            .map(|i| InputOut { key: i.key.clone(), secret: i.secret, initial: i.initial.clone() })
            .collect(),
        buttons: view
            .buttons
            .iter()
            .map(|b| ButtonOut {
                label: b.label.clone(),
                style: format!("{:?}", b.style).to_lowercase(),
                index: b.index,
                goes_to: navigation_target(&program, steps_of(b.index as usize)),
            })
            .collect(),
    };
    serde_json::to_string(&out).unwrap_or_else(|_| "null".into())
}

/// the screen a button opens when its steps are pure navigation.
///
/// `None` when the button runs a guard, a lookup or a reducer call — the
/// preview cannot know which way those branch without a live database, and
/// pretending otherwise is exactly the lie this crate exists to avoid.
fn navigation_target(p: &cuescript::Program, steps_id: u16) -> Option<String> {
    let steps = &p.programs[steps_id as usize];
    let mut only_open = None;
    for s in steps {
        match s {
            cuescript::program::Step::Open(id) => {
                if only_open.is_some() {
                    return None;
                }
                only_open = Some(p.screens[*id as usize].name.clone());
            }
            cuescript::program::Step::Kick(_) | cuescript::program::Step::Release => return None,
            // anything conditional makes the destination unknowable here
            cuescript::program::Step::Require { .. }
            | cuescript::program::Step::Lookup { .. }
            | cuescript::program::Step::Call { .. } => return None,
            _ => {}
        }
    }
    only_open
}

/// every screen name, for the preview's screen picker
#[wasm_bindgen]
pub fn screens(src: &str) -> String {
    let names: Vec<String> = match cuescript::build(src) {
        Ok(p) => p.screens.iter().map(|s| s.name.clone()).collect(),
        Err(_) => Vec::new(),
    };
    serde_json::to_string(&names).unwrap_or_else(|_| "[]".into())
}

/// the placeholder paths a document reads — the panel offers a value box per
/// path so a preview can be driven with realistic data
#[wasm_bindgen]
pub fn slots(src: &str) -> String {
    let paths: Vec<String> = match cuescript::build(src) {
        Ok(p) => p.slot_paths().to_vec(),
        Err(_) => Vec::new(),
    };
    serde_json::to_string(&paths).unwrap_or_else(|_| "[]".into())
}

fn describe(e: &cuescript::CueError) -> (u32, u32, String) {
    match e {
        cuescript::CueError::Syntax { span, msg } | cuescript::CueError::Resolve { span, msg } => {
            (span.line, span.col, msg.clone())
        }
        cuescript::CueError::Regex(m) => (1, 1, format!("Invalid regular expression: {m}")),
    }
}
