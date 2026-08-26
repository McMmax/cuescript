//! the parsed tree — one step above source, still holding names rather than indices

use crate::error::Span;

#[derive(Debug, Clone)]
pub struct Document {
    pub fragments: Vec<Fragment>,
    pub flow: Flow,
}

#[derive(Debug, Clone)]
pub struct Fragment {
    pub name: String,
    pub items: Vec<Item>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Flow {
    pub name: String,
    pub entry: String,
    pub entry_span: Span,
    pub screens: Vec<Screen>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Screen {
    pub name: String,
    pub items: Vec<Item>,
    pub span: Span,
}

/// one line inside a screen or fragment
#[derive(Debug, Clone)]
pub enum Item {
    Include {
        name: String,
        span: Span,
    },
    /// title/text/hint/good/rule share a shape; `kind` selects the styling role
    Line {
        kind: LineKind,
        text: String,
        color: Option<String>,
        bold: bool,
        span: Span,
    },
    Step {
        current: i64,
        total: i64,
        span: Span,
    },
    Input {
        key: String,
        secret: bool,
        fill: Option<String>,
        span: Span,
    },
    Button {
        label: String,
        style: ButtonStyle,
        steps: Vec<Stmt>,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Title,
    Text,
    Hint,
    Good,
    Rule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Quiet,
}

/// a statement inside a button body
#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        value: String,
        span: Span,
    },
    Remember {
        name: String,
        value: String,
        span: Span,
    },
    Require {
        cond: Cond,
        message: String,
        span: Span,
    },
    Lookup {
        table: String,
        field: String,
        value: String,
        bind: String,
        span: Span,
    },
    Call {
        effect: String,
        args: Vec<(String, String)>,
        handlers: Vec<Handler>,
        span: Span,
    },
    Open {
        screen: String,
        span: Span,
    },
    Stay {
        message: String,
        span: Span,
    },
    Say {
        message: String,
        span: Span,
    },
    Kick {
        message: String,
        span: Span,
    },
    Release {
        span: Span,
    },
}

/// `on <code> -> <actions>` attached to a call
#[derive(Debug, Clone)]
pub struct Handler {
    /// reducer error code, `timeout`, or `*` for anything unmatched
    pub code: String,
    pub actions: Vec<Stmt>,
    pub span: Span,
}

/// conditions are total and side-effect free — evaluating one can never call out
#[derive(Debug, Clone)]
pub enum Cond {
    Matches {
        value: String,
        pattern: String,
    },
    IsEmpty {
        value: String,
        negated: bool,
    },
    Equals {
        left: String,
        right: String,
        negated: bool,
    },
    NoWhitespace {
        value: String,
    },
    LengthBetween {
        value: String,
        min: String,
        max: String,
    },
}
