//! errors carry a source position — an author edits text, so "line 12" beats a byte offset

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Error)]
pub enum CueError {
    #[error("line {}, col {}: {msg}", span.line, span.col)]
    Syntax { span: Span, msg: String },

    /// a reference that parsed fine but points at nothing — the class of
    /// mistake that would otherwise strand a player on a dead screen
    #[error("line {}, col {}: {msg}", span.line, span.col)]
    Resolve { span: Span, msg: String },

    #[error("{0}")]
    Regex(String),
}

impl CueError {
    pub fn syntax(span: Span, msg: impl Into<String>) -> Self {
        Self::Syntax {
            span,
            msg: msg.into(),
        }
    }

    pub fn resolve(span: Span, msg: impl Into<String>) -> Self {
        Self::Resolve {
            span,
            msg: msg.into(),
        }
    }
}
