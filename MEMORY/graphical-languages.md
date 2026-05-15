# Adding LD / FBD / CFC / SFC support — design plan

**Status**: design only, not implemented. Read `MEMORY/principles.md`
first — every decision below is downstream of those principles.

## Where we stand today

From a code audit on 2026-05-15:

| Layer | State |
|---|---|
| `PouLanguage` enum (`crates/project/src/types.rs`) | All 5 variants present (`St / Ld / Fbd / Il / Sfc`) — schema is ready, semantics are not. |
| `crates/project/src/store.rs::create_pou_file` | **Actively rejects** non-ST with `StoreError::UnsupportedLanguage`. |
| Disk format | Hardcoded `.st` extension. POU language not persisted; only source text written. |
| ironplc parser (`vendor/ironplc/compiler/parser/`) | **ST only**. No FBD / LD / IL textual parser. |
| ironplc PLCopen XML module (`vendor/ironplc/compiler/sources/src/xml/`) | Parses XML structure of FBD / LD / SFC bodies. **Only SFC** is transformed to DSL with steps/transitions/actions. FBD/LD/IL bodies silently transform to `FunctionBlockBodyKind::Empty`. ironplc has **zero codegen** for graphical bodies. |
| Bridge (`crates/ironplc-bridge/src/lib.rs::compile`) | Strictly `&str` → `Container`. No language dispatch. |
| Server routes (`/api/pous/...`) | `Content-Type: text/plain` for source, `Pou { source: String, ... }` response. No language-aware shape. |
| Frontend editor | `<STEditor>` (Monaco) unconditionally. No branching on `currentPou.declarations[0].language`. |
| New-POU dialog | Hardcodes ST. |

The infrastructure has the **schema slot** but is functionally empty
below it.

## The architectural question

**At which level do we generate executable code from a graphical
program?** Three candidate pipelines:

```
A) JSON → transpile to ST string → ironplc parse → codegen → bytecode
B) JSON → emit PLCopen XML → ironplc XML module → DSL → codegen
C) JSON → directly synthesise ironplc DSL nodes → codegen
```

|   | Pros | Cons |
|---|---|---|
| **A. transpile-to-ST** | Reuses entire existing ironplc pipeline. We own one transpiler per language. Diff-friendly intermediate (the generated ST is greppable / debug-printable). Agents can paste the transpiled ST into ChatGPT to debug. | Some constructs (esp. SFC) inflate when expressed in ST. Source-map mid-error needs care. |
| **B. transpile-to-XML** | Aligns with industry's PLCopen interchange format. Future Codesys import/export "free". | ironplc's XML module **doesn't emit code** for FBD / LD / IL today. We'd have to contribute upstream codegen for all four. Doubles the dependency surface. XML is verbose, ugly, not agent-friendly. |
| **C. direct DSL emit** | Theoretically the tightest. | Brittle to ironplc internals. No human-readable intermediate. Hard to debug. Hard to test. |

**Choose A — transpile to ST.** This is the simplicity-first answer. It
matches what most commercial PLCs actually do internally (LD/FBD are
historically syntactic sugar over IL/ST). We don't touch ironplc. The
intermediate ST is observable; if a graphical program misbehaves we can
read the ST it generated and reason about it like any ST program.

We can offer "Export to PLCopen XML" later as a separate flow for
Codesys interop, **never as the storage format**.

## Storage format

Canonical on-disk representation is **JSON**, not XML, not binary, not
S-expressions. JSON because:

- Agents read/write it natively. No "I need to teach Claude how to write
  XML for FBD" tutorial.
- `git diff` is meaningful.
- Schema enforced via `ts-rs` (single source of truth across Rust ↔ TS).
- Round-trip clean: load → render → save = byte-identical (modulo
  reformatting).

**File extensions** — encode language in the filename so file managers,
CI, grep, and the store all stay simple:

```
pous/cascade_pid.st                       (ST, today's convention)
pous/motor_start.ld.json                  (Ladder Diagram)
pous/temp_control.fbd.json                (Function Block Diagram)
pous/free_layout.cfc.json                 (CFC — FBD with free positions)
pous/batch_sequence.sfc.json              (Sequential Function Chart)
```

Rationale for `.<lang>.json` (not `.ld` alone, not `.json` alone): the
double-extension keeps both pieces of metadata observable. A human
sees "this is a ladder diagram, encoded as JSON" without opening the
file. Editors that don't know about `.ld` still get JSON syntax
highlighting.

Single `pous/` directory — **don't split into `pous_st/`,
`pous_ld/`** etc. The project tree groups POUs by purpose, not by
encoding. Mixing languages in one folder is fine and follows the
"language is an implementation detail of a POU" principle.

## Per-language design sketch

### LD (Ladder Diagram) — ship first

Boolean relay logic. The simplest graphical language and the one closest
to historical PLC tradition. Validates the entire JSON → render →
transpile → ironplc pipeline.

**Schema** (TypeScript-ish, real shape goes in Rust + ts-rs):
```jsonc
{
  "language": "ld",
  "variables": [{ "name": "start_btn", "type": "BOOL", "section": "input" }, ...],
  "rungs": [
    {
      "id": "r0",
      "label": "motor seal-in",
      // Series + parallel composition of contacts; same shape recursively.
      "logic": {
        "op": "and",
        "args": [
          { "op": "contact", "var": "start_btn" },
          { "op": "or", "args": [
            { "op": "contact", "var": "stop_btn", "negated": true },
            { "op": "contact", "var": "motor_run" }   // seal-in feedback
          ]}
        ]
      },
      "coil": { "var": "motor_run", "kind": "standard" }
    }
  ]
}
```

**Transpile to ST**: each rung becomes one assignment.
```st
motor_run := start_btn AND (NOT stop_btn OR motor_run);
```
Latch coils (`set` / `reset` kinds) emit IF statements. Timer / counter
ladder elements become FB instantiations of the IEC standard library.

**Render**: SVG, ~600 LOC. Vertical rails left/right, rungs are
horizontal rows, contacts and coils render at fixed widths. Auto-layout
from the recursive logic tree (left-to-right for AND, top-to-bottom for
OR). **No free positioning** — LD has a strict canonical layout.

### FBD (Function Block Diagram)

Blocks + wires. Each block is an instance of a FUNCTION_BLOCK or
FUNCTION (defined in ST or in another FBD POU). Wires are name-based
references in JSON.

**Schema**:
```jsonc
{
  "language": "fbd",
  "variables": [...],
  "blocks": [
    {
      "id": "pid1",
      "fb_type": "pid_controller",  // resolves to FB declared elsewhere
      "inputs": {
        "setpoint": { "ref": "T_sp" },        // variable reference
        "pv": { "ref": "T_r" },
        "kp": { "const": "0.5" }              // literal
      }
    },
    {
      "id": "out1",
      "fb_type": "scale_0_100",
      "inputs": { "raw": { "block": "pid1", "port": "output" } }  // wire
    }
  ],
  "outputs": { "valve_pct": { "block": "out1", "port": "scaled" } }
}
```

**Transpile to ST**: dependency-sort blocks, then emit FB instance
declarations + a call sequence. Wires become assignments to temporary
variables. Cycles (forbidden in FBD without explicit feedback marker)
fail at transpile time with a useful error.

**Render**: use `@xyflow/react` (react-flow) — a battle-tested library
that already handles drag, zoom, pan, connection-by-port. Auto-layout
via `dagre` when positions aren't specified. Positions, when the user
drags them, **persist in the JSON** (the only deviation from "data
only" — but it's optional metadata, layout is regenerable from scratch).

### CFC (Continuous Function Chart)

Reuse the FBD schema and renderer. The only difference is **CFC always
persists positions** and supports more layout flexibility (feedback
without strict sort order, decorative free-floating annotation
elements). Same transpiler.

**Don't ship CFC as a separate language at all initially.** Just allow
FBD to optionally persist positions and call it "FBD" until someone
specifically asks for Codesys-style CFC semantics (free-form blocks
without execution-order constraints). That decision is far enough in
the future that we shouldn't prepay the complexity.

### SFC (Sequential Function Chart)

State-machine: steps, transitions, actions. ironplc's XML module
already parses SFC bodies into DSL — but the **codegen** is missing
upstream. For the transpile-to-ST plan, that doesn't matter; we lower
SFC to a `CASE current_step OF` block in ST.

**Schema**:
```jsonc
{
  "language": "sfc",
  "variables": [{ "name": "step", "type": "DINT", "section": "internal" }, ...],
  "initial_step": "idle",
  "steps": [
    {
      "name": "idle",
      "actions": []
    },
    {
      "name": "filling",
      "actions": [
        { "qualifier": "N",      "action": "open_inlet" },     // N = while active
        { "qualifier": "S",      "action": "start_timer" },    // S = set on entry
        { "qualifier": "P1",     "action": "log_entry" }       // P1 = pulse on entry
      ]
    },
    { "name": "draining", "actions": [{ "qualifier": "N", "action": "open_drain" }] }
  ],
  "transitions": [
    { "from": "idle",     "to": "filling",  "condition": "start_btn" },
    { "from": "filling",  "to": "draining", "condition": "tank_full" },
    { "from": "draining", "to": "idle",     "condition": "tank_empty" }
  ]
}
```

**Transpile to ST**: encode step as an enum/DINT, emit `CASE` for
action execution and a transition-evaluation block. Action qualifiers
(N/S/R/P/P1/P0 etc.) lower to IF-edge / IF-level patterns.

**Render**: vertical flow of step boxes with transition bars between.
Step names are short identifiers, action lists hang off the right side.
Use SVG, not react-flow — SFC has a strict canonical top-to-bottom
layout that doesn't need a generic graph editor.

### IL (Instruction List) — **skip**

Deprecated by IEC 61131-3 Ed 3. Almost no real-world authorship today.
Don't budget a single hour of design time for it. If a user asks, point
them at ST and explain ST replaces IL with a real expression syntax.

## Phased rollout

| Phase | Scope | ~ Cost | Demo target |
|---|---|---|---|
| **1. LD** | Schema + transpiler + read-only SVG render + write path. | 1–2 weeks | "Author a motor seal-in circuit by editing JSON, see it render as a ladder, Run it, watch the coil flip in Monitor." |
| **2. LD editing** | Drag/drop add contacts; rung edit. | 1 week | Click-to-add contact, change variable from a dropdown. |
| **3. FBD** | Schema + dependency sort + transpiler + react-flow renderer with auto-layout (dagre). | 2–3 weeks | "Define a cascade controller graphically; the FB instances we already have (pid, lp_filter, arrhenius) appear as draggable blocks." |
| **4. FBD position persistence (= CFC)** | Save dragged positions to JSON. Nothing else changes. | 2 days | Same demo as Phase 3, but rearranging blocks survives reload. |
| **5. SFC** | Schema + transpiler + vertical-flow renderer. | 2 weeks | Batch process: idle → filling → draining → idle. Watch `current_step` change in Monitor. |

Each phase **ships independently**. Don't gate Phase 3 on Phase 2 being
"polished". A read-only renderer with JSON-edit-only authoring is a
complete, useful product per phase — it preserves the principle that
agents are first-class authors (they write JSON directly and don't need
the drag UI).

## Cross-language reuse

A FUNCTION_BLOCK declared in any language (ST, FBD, LD, SFC) must be
callable from any other language. This works "for free" with the
transpile-to-ST plan because every POU ends up as ST before reaching
ironplc, and ironplc's symbol resolution is then per-FB identifier, not
per-source-language.

The only subtle bit: when an FBD POU imports `pid_controller`, the
transpiler needs to know its input/output port names. Resolve this by
reading the project tree for the target POU's `declarations[].name` +
`variables` (which the bridge already extracts via
`extract_pou_declarations`).

## Standard function block library — owned by ironplc

The IEC 61131-3 standard function blocks (TON / TOF / TP / SR / RS /
CTU / CTD / CTUD / R_TRIG / F_TRIG) are **not our code**. They are
implemented natively in ironplc's VM as Rust intrinsics:

- ADR: `vendor/ironplc/specs/adrs/0003-plc-standard-function-blocks-as-intrinsics.md`
- Native impls: `vendor/ironplc/compiler/vm/src/intrinsic.rs`
  (`ton`, `tof`, `tp`, `sr`, `rs`, `ctu`, `ctd`, `ctud`, `r_trig`, `f_trig`)
- VM dispatch: `vendor/ironplc/compiler/vm/src/vm.rs` matches on
  `opcode::fb_type::{TON, TOF, ...}` and routes FB_CALL to the
  corresponding Rust function.
- Type signatures (pin names / types) live in
  `vendor/ironplc/compiler/analyzer/src/stdlib.rs`.

**The library is language-agnostic.** ST calls it via `myT(IN := x, PT := T#1s)`,
LD calls it by transpiling a graphical block to that same ST call, FBD
will do the same. We never write `TON`'s timing logic — we only write
the **per-language front-end that emits the call**.

When adding new graphical front-end support for the standard FB set:

1. Front-end node: a graphical element with `(instance_name, fb_type,
   input bindings, output pin selection)`.
2. Transpiler: emit `instance_name : fb_type;` in VAR; emit
   `instance_name(IN := ..., ...);` as a statement before the rung's
   coil assignment; substitute the node's position in any boolean
   expression with `instance_name.output_pin`.
3. Editor metadata: hard-coded table of `{fb_type → [inputs], [outputs]}`
   (mirroring `stdlib.rs`) so the UI knows which pins to render.

The metadata table **duplicates** what's in ironplc — a deliberate
tradeoff: the UI needs synchronous pin info without round-tripping
through the compiler. If the standard FB set ever grows, both sides
need updating; the duplication is small enough (~10 FBs × ~4 pins
each) to maintain by hand.

## Source map for diagnostics

ironplc emits errors keyed to line/col in the ST it received. After
transpilation, that line/col points into our generated ST, which the
user never wrote.

**Mitigation**: each language's transpiler emits a side-channel
"source map" — a `Vec<(generated_line_range, graphical_element_id)>`.
When ironplc reports an error, the bridge maps it back to the rung /
block / step in the originating JSON file and the IDE highlights that
element instead of an editor line.

This is the **one piece of net-new infrastructure** required across all
graphical languages. Plan for it in Phase 1 (when implementing LD) so
the pattern is set before FBD lands.

## What we are NOT doing

(Cross-reference `principles.md` § "What this rules out")

- **No PLCopen XML as canonical storage.** Optional export only.
- **No drag-only authoring.** The JSON is the source; the GUI is one of two ways to edit it.
- **No custom graph editor framework.** Use react-flow for FBD/CFC.
- **No bypass of ST in the pipeline.** Don't be tempted to "optimise"
  by emitting DSL or bytecode directly — we lose the human-readable
  intermediate that makes debugging tractable.
- **No language-specific subdirectories.** All POUs live under `pous/`.
- **No "advanced PLCopen features" upfront.** Skip action blocks,
  resource declarations, multi-resource configurations, etc. Until
  the basic LD / FBD / SFC trio is solid.

## Risks worth flagging

1. **Transpiler bug surface**. Each language is a separate compiler.
   Mitigation: aggressive property-based testing — generate random
   well-typed LD/FBD/SFC trees, transpile, ensure ironplc accepts; OR
   round-trip a corpus of textbook examples through transpile → execute.

2. **Editor complexity**. react-flow + dagre adds frontend weight.
   Mitigation: ship read-only renderer first, defer drag/drop authoring
   to a later phase. Agents can use the IDE without drag.

3. **Variable scoping**. Each POU file needs its VAR / VAR_INPUT /
   VAR_OUTPUT declarations. For graphical POUs, put them in a
   `variables: [{name, type, section, init?}, ...]` block in the JSON
   — a separate "interface" tab in the UI maps directly to it.

4. **Drift in ironplc's PLCopen XML module**. We don't use it for
   storage, but if we ever offer XML export, the upstream stub status
   for FBD/LD codegen means our export-then-import round-trip won't
   close. Document this in any export feature.

5. **Layout flickering on auto-layout**. dagre / elk re-layouts on
   every block add can be jarring. Mitigation: only auto-layout on
   first render or explicit "Tidy" button; preserve positions otherwise.

## Memory anchor

When picking up this work in the future, the **single most important
decision** to remember is: **transpile-to-ST, not transpile-to-XML, not
direct-to-DSL**. Every other choice — JSON storage, react-flow,
phased rollout — follows from that one axis. If a future contributor
proposes routing graphical languages through PLCopen XML or directly
into ironplc's DSL, push back hard and re-read the trade-off table in
"The architectural question" above.

Written 2026-05-15 alongside `principles.md`.
