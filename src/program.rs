//! the compiled form — names are gone, everything is an index
//!
//! a `Program` is built once per document revision and shared immutably across
//! every player. rendering walks flat vectors and writes into a caller-owned
//! buffer, so steady state allocates nothing.

use regex_lite::Regex;
use smallvec::SmallVec;

use crate::ast::{ButtonStyle, LineKind};

/// index into `Program.screens`
pub type ScreenId = u16;
/// index into `Program.slots` — a placeholder path, resolved once at compile
pub type SlotId = u16;
/// (start, len) into `Program.text` — the interned literal blob
pub type TextRef = (u32, u32);

/// one instruction of a compiled template
#[derive(Debug, Clone)]
pub enum Op {
    /// copy interned bytes — never allocates
    Literal(TextRef),
    /// write the value of a slot
    Slot(SlotId),
    /// write the value of a slot through a named modifier
    Modified(SlotId, Modifier),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modifier {
    Trim,
    Upper,
    Lower,
    /// unknown modifiers are kept by name so a host can extend the set without
    /// a compiler change — the manifest is the vocabulary, not the grammar
    Named(String),
}

/// a compiled template: a flat op list, no tree walk
#[derive(Debug, Clone, Default)]
pub struct Text {
    pub ops: SmallVec<[Op; 4]>,
}

impl Text {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct Line {
    pub kind: LineKind,
    pub text: Text,
    pub color: Option<String>,
    pub bold: bool,
}

#[derive(Debug, Clone)]
pub struct Input {
    pub key: String,
    pub secret: bool,
    pub fill: Text,
}

#[derive(Debug, Clone)]
pub struct Button {
    pub label: Text,
    pub style: ButtonStyle,
    /// index into `Program.programs` — the step list this button runs
    pub steps: u16,
}

#[derive(Debug, Clone, Default)]
pub struct Screen {
    pub name: String,
    pub lines: Vec<Line>,
    pub inputs: Vec<Input>,
    pub buttons: Vec<Button>,
}

/// a compiled condition — total and side-effect free by construction
#[derive(Debug, Clone)]
pub enum Cond {
    Matches {
        value: Text,
        re: Regex,
    },
    IsEmpty {
        value: Text,
        negated: bool,
    },
    Equals {
        left: Text,
        right: Text,
        negated: bool,
    },
    NoWhitespace {
        value: Text,
    },
    LengthBetween {
        value: Text,
        min: usize,
        max: usize,
    },
}

#[derive(Debug, Clone)]
pub enum Step {
    Let {
        name: String,
        value: Text,
    },
    Remember {
        name: String,
        value: Text,
    },
    Require {
        cond: Cond,
        message: Text,
    },
    Lookup {
        table: String,
        field: String,
        value: Text,
        bind: String,
    },
    Call {
        effect: String,
        args: Vec<(String, Text)>,
        handlers: Vec<Handler>,
    },
    Open(ScreenId),
    Stay(Text),
    Say(Text),
    Kick(Text),
    Release,
}

#[derive(Debug, Clone)]
pub struct Handler {
    pub code: String,
    pub actions: Vec<Step>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub flow: String,
    pub entry: ScreenId,
    pub screens: Vec<Screen>,
    /// step lists, referenced by `Button.steps`
    pub programs: Vec<Vec<Step>>,
    /// placeholder paths, deduped. a `SlotId` indexes this
    pub slots: Vec<String>,
    /// every literal, concatenated once
    pub text: String,
}

impl Program {
    #[inline]
    pub fn literal(&self, r: TextRef) -> &str {
        &self.text[r.0 as usize..(r.0 + r.1) as usize]
    }

    pub fn screen_by_name(&self, name: &str) -> Option<ScreenId> {
        self.screens
            .iter()
            .position(|s| s.name == name)
            .map(|i| i as ScreenId)
    }

    /// every placeholder path this program can read. a host fills exactly these
    /// — and a runtime can subscribe to only these to know when to re-render
    pub fn slot_paths(&self) -> &[String] {
        &self.slots
    }
}
