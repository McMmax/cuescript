# 📜 CueScript

**CueScript** is a custom, highly optimized scripting language purpose-built for writing dialogues, events, and logic sequences. It features runtimes in both **Rust** and **WebAssembly (WASM)**.

---

## 🎯 What does it do?
Game designers and writers need a way to author complex NPC dialogues, cutscenes, and dynamic events without having to write native Rust or Java code. 

CueScript is a domain-specific language (DSL) that looks and feels like a simple script, but compiles down into a fast, safe execution format. It allows creative teams to build complex logic trees, branching dialogues, and interactive events with zero programming overhead.

## 🏗️ Architecture

- **`src/` (Rust Engine)**: The core compiler (`compile.rs`), lexer (`lex.rs`), parser (`parse.rs`), and runtime virtual machine (`runtime.rs`). It executes CueScript at blazing-fast native speeds.
- **`wasm/` (WebAssembly)**: Binds the Rust engine to WASM, allowing CueScript to be compiled, validated, and even executed directly inside a web browser (perfect for web-based dialog editors or admin panels!).

## 🚀 Key Features
- **Lightning Fast**: Built in Rust for zero-cost abstractions and immediate execution.
- **Web Ready**: Ships with full WASM support. Write a script on the web, test it in the browser, and deploy it to the server seamlessly.
- **Safe Execution**: Scripts run in a sandboxed runtime, meaning a bad script cannot crash the game server or access unauthorized memory.
