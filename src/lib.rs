//! CueScript — the authoring language for player-facing surfaces.
//!
//! Source text is parsed and compiled **once per document revision**, then the
//! resulting [`Program`] is shared immutably across every player. Rendering
//! walks flat op lists over integer slots and writes into a caller-owned
//! buffer, so steady-state rendering allocates nothing.
//!
//! ```
//! use cuescript::{build, runtime};
//! use std::collections::HashMap;
//!
//! let program = build(r#"
//!     flow auth {
//!         entry welcome
//!         screen welcome {
//!             title "Welcome, {player.name}"
//!             button "Continue" primary { release }
//!         }
//!     }
//! "#).unwrap();
//!
//! let mut slots = HashMap::new();
//! slots.insert("player.name".to_string(), "Steve".to_string());
//!
//! let view = runtime::render(&program, program.entry, &slots);
//! assert_eq!(view.title, "Welcome, Steve");
//! ```
//!
//! The language is deliberately **not Turing-complete**: there is no loop
//! construct, so a script always terminates and the runtime needs no watchdog.

pub mod ast;
pub mod compile;
pub mod error;
pub mod lex;
pub mod parse;
pub mod program;
pub mod runtime;

pub use error::{CueError, Span};
pub use program::Program;
pub use runtime::{
    render, run, BoxFuture, CallResult, Host, Outcome, Row, Scope, ScreenView, Slots,
};

/// parse and compile source into a runnable program
pub fn build(src: &str) -> Result<Program, CueError> {
    let doc = parse::parse(src)?;
    compile::compile(&doc)
}

/// parse and compile, discarding the program — for a panel's "validate" button
pub fn check(src: &str) -> Result<(), CueError> {
    build(src).map(|_| ())
}
