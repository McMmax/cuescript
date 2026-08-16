# CueScript — the network's authoring language for player-facing surfaces

**Crate / package**: `cuescript` (`libraries/cuescript`)
**Status**: design, not started
**Scope**: one language and one intermediate representation shared by every
library that renders text or UI to a player — Holster, Jingle, Acmd, Univue, the
Internode auth dialog, scoreboard and tablist.

A *cue* is the instruction that makes something happen. CueScript is what staff
write; the compiler turns it into a program each host runs at native speed.

---

## 1. Why

Player-facing text is currently authored in as many ways as there are libraries:

| Surface | Where the text lives today | Editable at runtime? |
|---|---|---|
| Auth dialog | Rust structs in `internode-core/src/limbo/auth_dialog.rs` | no — recompile |
| Chat messages | Jingle catalog + `internode-core/src/commands/messages.rs` | partly, via cold overrides |
| Placeholders | Holster `{key}` / `{key:arg}` tokenizer (Java) | n/a |
| Scoreboard / tablist | per-gamemode Java | no |
| Univue menus | planned serializable menu spec | planned |
| Code of conduct | a file on the artifacts mount | yes, by hand |

The consequence is visible in the auth dialog we just shipped: the divider rule,
the `Offline Identity:` header, the step markers and every colour are **hardcoded
in Rust**. Changing a dash means a rebuild, a binary copy and a pod restart. The
`/interfaces` panel can script the *flow*, but it cannot touch that chrome,
because the chrome was never data.

The goal is that **every glyph a player sees is data**, authored once, in one
language, rendered identically everywhere.

## 2. What already exists (build on, don't replace)

- **Holster** (`libraries/holster`, Java) — `{key}` and `{key:arg1:arg2}`
  tokenizer with a placeholder registry, resolver and scopes. This is the
  *value* layer and it works. Its modifier idea (`:12h`, `:smallcaps`) is the
  right instinct at too small a scale.
- **Jingle** (`libraries/jingle`, Rust, **implemented** — ~2.5k lines with
  tests) — a message catalog whose template language already parses conditionals,
  key references, defaults, joins, plurals, named styles, gradients and hex
  blocks into a validated `Template` tree (`src/template/mod.rs`). **This is
  Layer 1, and it already exists.** It should not be reinvented; it should be
  promoted from "internode's message library" to the text layer of the platform.

  What it does *not* yet have is the compiled render path this plan needs:
  `Template::render` walks a `Vec<TemplateNode>` and returns an owned `String`,
  and placeholder keys are `String`s resolved per render. Section 5 is therefore
  a set of changes *to Jingle* — flat ops, interned literals, integer slots, a
  caller-owned buffer — not a competing implementation. Its own plan already
  commits to the principle ("pre-compiled, not re-parsed at render time"); this
  finishes the job.
- **Acmd** — command declaration. Already the model the platform likes:
  declarative, registered, introspectable.
- **Univue** — the recorded decision is a *serializable menu spec* rendered
  backend-side with a dashvue previewer. That is this design, scoped to menus.

So the work is not inventing a language. It is **unifying three half-languages
and giving them a structural layer, a data layer and an editor**. CueScript is
the name for the whole of that — Jingle remains the crate that owns Layer 1.

## 3. The four layers

The mistake to avoid is treating this as one language. It is four, stacked.

```
┌───────────────────────────────────────────────────────────┐
│ 4. BINDING     what a control does                        │
│                actions, guards, navigation                │
├───────────────────────────────────────────────────────────┤
│ 3. DOCUMENT    what is on screen                          │
│                components, layout, flow between screens   │
├───────────────────────────────────────────────────────────┤
│ 2. DATA        what the script knows and changes          │
│                SpacetimeDB queries and reducer effects    │
├───────────────────────────────────────────────────────────┤
│ 1. TEXT        what a string says                         │
│                placeholders, conditionals, styling        │
└───────────────────────────────────────────────────────────┘
```

Layer 1 is Holster + Jingle. Layer 2 is what makes this a scripting language
rather than a template language. Layers 3 and 4 are new, and the manifest that
governs 2 and 4 is the safety boundary.

### Layer 1 — text

Jingle's designed syntax, adopted verbatim so there is one dialect:

```
{player}                          value
{player|upper}                    modifier
{time|12h:America/New_York}       modifier with args   (Holster's idea, generalised)
{friends} friend{friends|plural::s}
{username|default:Unknown}
{if:in_party}In {party_name}{/if}
{key:staff.prefix}               embed another catalog entry
{style:error}Denied{/style}       named style from the registry
{gradient:#FF0000:#0000FF}Hi{/gradient}
```

Every surface uses this for **every** string — a scoreboard line, a button
label, a dialog body line, a command's error. One dialect, one parser, one set
of docs for staff.

### Layer 2 — data (SpacetimeDB)

A script must be able to ask the database a question and branch on the answer.
Username availability is the motivating case: today that check lives in Rust
(`AuthStorage::username_available`) and the author cannot see it, let alone
change it. It should be visible script.

Two verbs, deliberately different, because reads and writes have nothing in
common operationally:

**`query`** — a read. Resolved against the proxy's **live subscription cache**,
not a SQL round trip. This matters: the proxy already holds cold tables in
memory (that is what `find_player_by_username` reads), so a query costs
microseconds and cannot time out mid-login. Queries are declared in the
manifest, parameterised, and read-only.

**`effect`** — a reducer call. Authoritative, may fail, may mutate production
state. Allowlisted per host, with declared arguments and mapped error codes.

Both are **named in the capability manifest** — the panel never writes raw SQL.
That is not a convenience choice: arbitrary SQL from an editor is an injection
surface, an unbounded-latency surface on the login path, and unreviewable.

```jsonc
// capability manifest — the host declares what a script may touch
"queries": [
  {
    "id": "account_by_username",
    "table": "player_core",
    "index": "username_lower",     // case-insensitive, matches the reducer rule
    "params": ["username"],
    "returns": "row?",             // optional row
    "source": "subscription_cache",
    "advisory": true               // cache may lag; the reducer stays authoritative
  }
],
"effects": [
  {
    "id": "player_register",
    "reducer": "player_register",
    "args": ["username", "password"],
    "errors": ["username_taken", "invalid_username", "already_registered"]
  }
]
```

### The availability check, as script

This is the logic the author can now read and change, with the boolean that
gates the next page sitting in plain sight:

```jsonc
{
  "label": "Check Availability",
  "run": [
    { "let": "name", "be": "{input.username|trim}" },

    { "guard": "{name|matches:^[A-Za-z0-9_]{1,16}$}",
      "else": { "stay": true, "error": "Use 1-16 letters, numbers or underscores." } },

    { "query": "account_by_username",
      "with": { "username": "{name}" },
      "as": "existing" },

    { "guard": "{existing|is_empty}",                 // ← true/false gating the next page
      "else": { "stay": true, "error": "{name} is already taken. Try another." } },

    { "set": { "chosen_username": "{name}" } },
    { "goto": "register_password" }
  ]
}
```

`guard` is the whole idea: an expression that must be true to continue, with an
`else` branch that stays on the screen and shows a reason. A step list reads
top-to-bottom like the check it replaces, and the flow graph can render each
`goto` as an edge and each `guard` as a labelled branch.

Creation is the same shape, with an effect and mapped errors:

```jsonc
{
  "label": "Create My Account",
  "run": [
    { "guard": "{input.confirm|eq:{state.password}}",
      "else": { "stay": true, "error": "Those passwords do not match." } },

    { "effect": "player_register",
      "with": { "username": "{state.chosen_username}", "password": "{state.password}" },
      "as": "result",
      "on_error": {
        // the advisory read above can go stale between check and create;
        // that race is expected, so it routes back to step 1 rather than
        // dead-ending on the confirm screen
        "username_taken": { "goto": "register_username", "error": "That username was just taken." },
        "*":              { "stay": true, "error": "Could not create the account." }
      } },

    { "goto": "success" }
  ]
}
```

**Expressions.** Guards need more than Holster's `{if:flag}`: comparison
(`eq`, `ne`, `gt`), emptiness (`is_empty`, `exists`), regex (`matches`), and
boolean composition (`and`, `or`, `not`). Small, total, side-effect free — an
expression can never call a reducer, so evaluating one is always safe and
always terminates.

**Rules that keep this safe on the login path:**

1. **Reads are cache-backed, writes are reducers.** A script cannot issue
   unbounded SQL.
2. **Every step is bounded.** Effects declare a timeout and an `on_timeout`
   branch; a script that hangs would hold a player in limbo forever.
3. **Effects are not idempotent — treat them as such.** A step list runs once
   per activation; the runtime must not retry an effect automatically.
4. **The reducer is always authoritative.** Cache reads are advisory by
   declaration, so the "checked free, then taken" race is a designed-for path,
   not a bug to be surprised by.
5. **Step budget.** A capped number of steps and effects per activation, so a
   loop in a hand-edited document cannot spin.

### Layer 3 — document

A small component vocabulary, shared across surfaces:

```
text      a styled line (its content is a Layer-1 template)
divider   a rule — character, width and colour are properties, not a hardcoded "──────"
spacer    vertical gap
input     text / number / toggle / select, with a key
button    label + binding
group     rows/columns for layout
list      repeat a child template over a collection
```

The `Offline Identity:` header and the `─────` rule stop being Rust and become
two `text`/`divider` components in a reusable **fragment** the author can
include per screen — or not:

```jsonc
{ "include": "fragment:auth_header" }     // present on 5 screens, absent on the 6th
```

Fragments are the answer to "add it only to certain pages" without copy-paste.

### Layer 4 — binding

A button names an action. Actions come from a **capability manifest** that each
host publishes:

```jsonc
{
  "host": "internode.auth",
  "version": 1,
  "actions": [
    { "id": "login", "inputs": ["username", "password"] },
    { "id": "register_check_username", "inputs": ["username"] },
    { "id": "goto", "params": ["screen"] }
  ],
  "placeholders": ["offline_name", "offline_uuid", "username", "spacetime_uid"],
  "queries": ["account_by_username"],
  "effects": ["player_register", "player_login"],
  "components": ["text", "divider", "spacer", "input", "button"]
}
```

This is the load-bearing safety idea. The editor reads the manifest and can only
offer what the host implements, so a saved document cannot reference a behaviour
that does not exist. We already learned this the expensive way: a `SCustomClickAction`
that was defined, registered and tested but never *consumed* produced buttons that
rendered perfectly and did nothing. A manifest makes that class of bug unrepresentable.

## 4. Portable IR, and why the preview must share it

Authoring format → **compiled IR** (versioned JSON) → renderers.

```
        dashvue editor ─┐
        .ui source files ┼─► compiler ─► IR (JSON, versioned)
        panel forms ────┘                    │
                          ┌──────────────────┼──────────────────┐
                          ▼                  ▼                  ▼
                  Rust renderer       JVM renderer       WASM/TS renderer
                  (internode:         (Stomzy: scoreboard, (dashvue live
                   dialogs, limbo)     tablist, univue)     preview)
```

Three renderers will drift. The discipline that prevents it:

1. **One conformance suite** — golden `(IR, context) → output` fixtures in the
   repo. Every renderer runs them in its own CI. A renderer that disagrees fails.
2. **The preview is a renderer, not a mock.** If the panel approximates, staff
   will design against a lie. Preferred: compile the Rust renderer to WASM and
   run *that* in dashvue, so preview and Internode are the same code.
3. **IR carries `minRendererVersion`.** A host that cannot render a document
   refuses it and falls back, rather than rendering half of it.

## 5. Performance — compiled, not interpreted

The rule: **nothing is parsed, resolved or allocated on the hot path.** A script
must cost what the equivalent hand-written Rust or Java costs, because it will
be replaced by hand-written code the first time it shows up in a profile.

### The pipeline moves work to publish time

```
authoring text ──parse──► AST ──compile──► PROGRAM ──load──► renderer
    (staff, once)            (publish, once)        (per host, once)      (per render)
```

Everything expensive happens left of `PROGRAM`:

| Work | Naive | Here |
|---|---|---|
| Parse the template | per render | at publish |
| Look up a placeholder by name | `HashMap<String, _>` per render | compile to a `u16` slot index |
| Compile a `matches:` regex | per evaluation | at publish, stored in the program |
| Resolve a `{key:…}` catalog reference | per render | inlined, or a direct index |
| Decide which components are visible | branch per render | static segments pre-merged |
| Parse the document | per fetch | binary IR (postcard), `mmap`-able |

A compiled `PROGRAM` is a flat instruction list over a slot table — not a tree
walked with dynamic dispatch. Rendering is a `for` loop over opcodes writing
into a reused buffer.

### The render hot path

```rust
// no allocation, no hashing, no string compare
pub fn render(program: &Program, ctx: &Slots, out: &mut String) {
    for op in &program.ops {
        match op {
            Op::Literal(range)   => out.push_str(program.text(*range)), // memcpy from an interned blob
            Op::Slot(i)          => ctx.write(*i, out),
            Op::JumpIfFalse(..)  => { /* precomputed branch */ }
        }
    }
}
```

- **Static text is interned once** into a single blob; a literal is a `(start,len)`
  into it, so emitting it is a `memcpy`, never a `String` allocation.
- **`out` is a caller-owned reusable buffer**, so steady-state rendering allocates
  **zero** times.
- **Slots are integers.** Placeholder *names* exist only in the editor.
- Adjacent literals are merged at compile time, so `"a" + "b" + "c"` is one op.

### Where the real win is: render only on change

This is why the claim "it should actually buff performance" is credible rather
than optimistic. A scoreboard today re-renders on a fixed tick regardless of
whether anything changed. A compiled program **declares its dependencies** — the
compiler knows exactly which slots it reads — so the runtime can:

1. subscribe each program to only the slots it touches,
2. mark it dirty when one of those slots changes,
3. re-render **only dirty programs**, and
4. skip the packet entirely when the rendered bytes are unchanged.

For a 15-line scoreboard where one line holds a timer and fourteen are static,
that is 14/15 of the work deleted, plus the network write. Hand-written Java
rarely bothers with that bookkeeping; a compiler gets it for free because it
already knows the dependency set.

### Async, and never on the tick thread

Layer 2's two verbs have deliberately different execution profiles:

- **`lookup` (read) is synchronous and in-memory.** It resolves against the
  proxy's live subscription cache — a `DashMap` hit, not I/O — so it is safe
  inside a render or a tick. It cannot block, cannot time out, cannot fail
  slowly. This is the same read `AuthStorage::username_available` performs today.
- **`call` (reducer) is the only await point.** It runs on a task, never on the
  game tick or the netty read thread. A step list containing a `call` is executed
  as an async task; the screen shows its pending state meanwhile.

So the tick thread only ever runs pure, in-memory, bounded work. The scripting
runtime cannot introduce a stall into the tick loop, because nothing that can
stall is reachable from it.

### Budgets, enforced where they are free

- **Step and effect counts are capped at compile time** — a document that could
  exceed them is rejected at publish, so the runtime needs no counter.
- **Loops are bounded by construction**: `list` iterates a collection, there is
  no `while`. The language is not Turing-complete, and that is a feature — a
  script always terminates.
- **Per-`call` timeout with an explicit `on_timeout` branch**, so a slow reducer
  degrades to a message instead of holding a player forever.

### Memory

One compiled program per document per host, shared immutably across all players
(`Arc`/singleton), never cloned per session. Per-player state is a small slot
vector, `SmallVec`-backed for the common case of a handful of values.

## 6. Feel — it should read like our stack

A language nobody enjoys reading gets bypassed. The syntax borrows the vocabulary
staff already use for Minecraft and for this network, so the first read is the
only explanation needed.

```
screen register.username {
    include  auth_header
    title    "Create an Account"
    body     "Pick your username"
    hint     "1-16 characters — letters, numbers and underscores."
    input    username

    button "Check Availability" primary {
        let name = {input.username|trim}

        require name matches "^[A-Za-z0-9_]{1,16}$"
            else "Use 1-16 letters, numbers or underscores."

        lookup player_core by username_lower = name -> existing
        require existing is empty
            else "{name} is already taken. Try another."

        remember chosen_username = name
        open register.password
    }

    button "Back" quiet {
        open welcome
    }
}
```

Creating the account, with the effect reading like a server action:

```
button "Create My Account" primary {
    require {input.confirm} is {state.password}
        else "Those passwords do not match."

    call player_register with username = {state.chosen_username},
                              password = {state.password}
        -> result
        on username_taken -> open register.username
                             say "That username was just taken."
        on timeout        -> stay "Registration timed out. Try again."

    open success
}
```

**Why these words.** Each is either a Minecraft verb or a term already used in
this stack, so it needs no glossary:

| Keyword | Comes from | Reads as |
|---|---|---|
| `screen`, `open` | vanilla menus | open a screen |
| `include` | fragments | reuse a header |
| `require … else` | guard | a condition that must hold |
| `lookup … by … ->` | a table read | a cache-backed query |
| `call … with … ->` | reducer invocation | the authoritative write |
| `remember` | session state | persists across screens |
| `release` | **Internode's own limbo term** | let the player out of limbo |
| `kick`, `transfer`, `say`, `title`, `actionbar`, `sound` | server commands | do the obvious thing |
| `primary`, `quiet` | the button styles already shipped | visual weight |

`release` and `transfer` are the tell that this is *our* language: they are the
words Internode already uses internally for exactly these operations, so a
script reads like the proxy's own vocabulary rather than a generic DSL.

The text form compiles to the same IR the panel produces — they are two editors
for one program, so a document can be authored visually and reviewed as a diff.

### Extending it without changing the language

Growth happens in **registries and the manifest**, never in the grammar:

- **A new modifier** (`:smallcaps`) is a function registered against a name.
  The parser already accepts `{x|anything:args}`.
- **A new component** (`progress`) is a renderer entry plus a manifest line.
- **A new effect** (`grant_locker_item`) is a manifest entry naming a reducer.
  No compiler change, no release — the editor picks it up from the manifest.
- **A new surface** (`bossbar`) is a profile: which components, which root shape.
- **A new opcode** bumps the program version; a host that does not know it
  refuses the document and falls back, rather than half-rendering.

The grammar should be able to stay frozen for a long time while the vocabulary
grows weekly. That is the difference between a language that lasts and one that
gets forked.

## 7. Surface profiles

One document language, but a scoreboard is not a dialog. Each surface declares a
**profile**: which components it allows and how the root is interpreted.

| Surface | Root | Allowed components | Notes |
|---|---|---|---|
| `dialog` | screen graph | all | flow between screens; what `/interfaces` scripts today |
| `scoreboard` | line list | text, divider, spacer, list | ≤15 lines, per-line width limit |
| `tablist` | header/footer + column spec | text, divider, list | |
| `menu` (univue) | screen graph | all + item/slot | inventory geometry constraints |
| `message` (jingle) | single string | text only | the catalog case |
| `command` (acmd) | usage + response templates | text | help output becomes authorable |

The validator is profile-aware: putting an `input` in a scoreboard fails at
author time with a specific message, not at render time with a shrug.

## 8. Editor UX

The `/interfaces` page today is a form. The target is a three-pane workbench:

```
┌────────────┬───────────────────────────┬──────────────────┐
│ OUTLINE    │ CANVAS                    │ INSPECTOR        │
│            │                           │                  │
│ ▸ welcome  │  ┌─────────────────────┐  │ Component: text  │
│ ▸ login    │  │ Offline Identity: … │  │ Content:         │
│   ▸ header │  │ ──────────────────  │  │  Offline Identity│
│   ▸ title  │  │ Log In              │  │  : {offline_name}│
│   ▸ inputs │  │ [username        ]  │  │ Color:  #FFD166  │
│   ▸ Log In │  │ [password        ]  │  │ Visible if: …    │
│ ▸ register │  │  ( Log In )         │  │                  │
│            │  └─────────────────────┘  │                  │
├────────────┴───────────────────────────┴──────────────────┤
│ FLOW  welcome ──▶ login ──▶ (release)                     │
│            └──▶ register_username ──▶ … ──▶ success ──▶ login│
└───────────────────────────────────────────────────────────┘
```

Design principles, in priority order:

1. **Direct manipulation over JSON.** Click the divider, change its character.
   The JSON view stays available for power users and diffing, never required.
2. **The flow graph is a first-class view.** The lockout risk is a *graph*
   problem — unreachable screens, dead ends, missing entry — so show the graph.
   Unreachable nodes render greyed and disconnected; the eye catches it before
   the validator has to.
3. **Step lists are shown as logic, not JSON.** Each `run` renders as an
   ordered list of steps with its guards as diamonds, so the author sees
   "check the name → is it free? → yes: next page / no: show this error".
   The flow graph draws guard branches as separate labelled edges, which is how
   an author spots a screen reachable only when a check fails.
4. **Queries and effects are introspectable.** Hovering a query shows the table,
   index and whether it is advisory; hovering an effect shows its reducer and
   the error codes it can return, so the author can map them all.
5. **Preview with real context.** A "preview as" selector binds a real player's
   placeholders, so `{offline_name}` shows an actual name. Empty-state and
   longest-value previews too — the tablist that looks right for `Steve` and
   breaks for a 16-character name is the classic failure.
6. **Fragments are visible.** An included fragment renders inline but marked, and
   is edited once, everywhere.
7. **Effects are marked in the editor.** A step that mutates production state is
   visually distinct from one that only reads. Preview never runs effects — it
   stubs them and says so, because previewing a registration must not create an
   account.
8. **Publish is a deliberate step, and reversible.** Draft → validate → preview →
   publish, with revision history and one-click rollback (the `interface_doc_history`
   table already exists for this). Show what publish means: *takes effect for the
   next player to reach this surface*, since there is no hot reload.

## 9. Storage

Authored documents live in **harbor D1** (`interface_doc`, already built), not in
a cold table: cold requires a module republish, and the live coldmain is behind
repo HEAD with an auto-publish path that passes `--delete-data`. Documents are
read at connection time, not per tick, so cold buys nothing here.

Hosts fetch by id with a machine bearer (`INTERFACE_READ_KEY`, already
provisioned), cache in memory, and revalidate on a revision bump.

## 10. Migration path

Each phase must ship standalone value; none is a big-bang rewrite.

- **Phase 0 — IR, compiler, conformance, benchmarks.** Freeze the schema, write
  the golden fixtures, build the compiler and the Rust renderer plus its WASM
  binding. Land a benchmark suite in the same phase with a hard gate: a compiled
  scoreboard render must be within noise of the hand-written Java it replaces,
  and steady-state rendering must allocate zero times. "As fast as native" is a
  claim that decays silently unless CI fails when it stops being true. No
  product change yet.
- **Phase 1 — auth dialog.** Already the closest thing to data. Replace
  `auth_dialog.rs`'s hardcoded screens with IR fetched from harbor, keeping the
  built-in screens as the fallback. Proves the whole stack, and delivers the ask
  that started this: editable dividers and headers.
- **Phase 2 — Jingle adopts the text layer** and becomes the catalog for every
  library, replacing `messages.rs`.
- **Phase 3 — scoreboard + tablist** profiles, rendered from IR by Holster.
- **Phase 4 — Univue menus** as the `menu` profile.
- **Phase 5 — Acmd** help and responses.

## 11. Risks, honestly

- **Three renderers drift.** Mitigated by the conformance suite; unmitigated if
  the suite is allowed to rot. Treat a conformance failure as a release blocker.
- **A malformed document locks players out.** Mitigated by author-time
  validation, the capability manifest, profile rules, and per-document
  `enabled=0` fallback to built-ins. This is why the fallback path must be
  written first, not last.
- **Scope.** This touches every library. Phase 1 must be genuinely useful on its
  own, or this becomes a rewrite that never lands.
- **Performance.** Scoreboards re-render per tick. IR must be *compiled* once
  and cached, never re-parsed per render — the same constraint Jingle's plan
  already sets ("pre-compiled, not re-parsed at render time"). The failure mode
  is not a cliff but a drift: a modifier that allocates, a slot resolved by name
  "just this once", a regex compiled per call. Benchmarks in CI are the only
  thing that catches that, which is why they are Phase 0 and not Phase 5.
- **A scripting runtime on the tick thread.** Guarded by construction: reads are
  in-memory and bounded, and the only await point (`call`) runs on a task. If
  that boundary is ever softened — a "quick" blocking lookup, a synchronous
  reducer — the language becomes a source of tick stalls and the whole
  performance argument collapses.
- **Staff authoring a broken flow at 3am.** Rollback must be one click, and the
  proxy must prefer a last-known-good document over a newly-published broken one.
- **A script reaching the database is a privilege boundary.** The manifest is
  what stops an editor becoming a remote code path into production: no raw SQL,
  an allowlist of reducers, bounded steps and timeouts. If any of that is
  relaxed for convenience later, this stops being safe to expose to staff.
- **Advisory reads race authoritative writes.** Designed for above; the failure
  mode is a confusing error, not corruption — but only because the reducer
  re-checks. Never let a script's cache read be the sole gate on a mutation.

## 12. Open questions

1. **Renderer language of record** — Rust compiled to WASM + JVM binding, or
   three hand-written renderers held together by the conformance suite? The
   first is more work up front and far less drift.
2. **Does the authoring format have a text syntax at all**, or is the panel the
   only editor? A text form gives diffs, review and version control; a panel-only
   format is simpler but unreviewable.
3. **Per-gamemode overrides** — one document per surface, or a base plus
   per-mode patches?
4. **Localisation.** Out of scope here, but the catalog shape should not make it
   impossible later.
5. **Does CueScript absorb Jingle, or depend on it?** Layer 1 is Jingle's
   today. Cleanest is `cuescript` depending on `jingle` and Jingle keeping the
   template language, so the catalog stays usable on its own. The alternative —
   folding Jingle in — buys a single crate at the cost of a working library.
6. **Is the text form the source of record, or the panel?** They compile to the
   same IR, but only one can own conflict resolution when both edit a document.

---

Related: `libraries/jingle` (Layer 1, implemented), `libraries/holster`,
`workers/harbor/src/interfaces.ts`, `workers/dashvue/app/pages/interfaces.vue`,
`platforms/internode/crates/internode-core/src/limbo/auth_dialog.rs`.
