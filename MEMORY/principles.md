# Design principles — read this first

These are the **non-negotiable** axes every design decision in this
codebase is judged on. If a proposal trades one of these for cleverness,
features, or "industry standard alignment", default to **reject**.

## 1. Simplicity is the headline feature

This is **the primary product positioning**, not a footnote. The whole
reason `controlsoftware` exists in a field already crowded with Codesys,
TwinCAT, Step 7, Studio 5000, IEC 61131-3 plugins, etc. is that all of
those have decades of accreted complexity. Engineers, students, agents,
and SDE/SRE crossovers shouldn't have to learn a 5000-page reference
manual to do anything useful. Every screen, file format, button, and
endpoint should answer "what is this for?" within 5 seconds.

**Concretely**:
- One concept per screen. Don't combine "edit POU" + "schedule POU" + "deploy" into one mega-panel.
- One canonical text representation per artefact. No proprietary binary blobs.
- Defaults that work without configuration. Empty config = sensible behaviour.
- No "advanced settings" submenus. If a setting is too advanced for the main UI, it probably isn't worth keeping.

## 2. Cognitive load — keep it low

The user (or agent) should be able to load the whole product into working
memory. If you can't draw the architecture on a single whiteboard in 60
seconds, the architecture is wrong.

**Concretely**:
- Vocabulary stays small. We have POU / Device / Edge / Task. Don't introduce "Workgroup", "Project Variant", "Compilation Profile" etc.
- One name per thing. Don't call it `app` here and `pou` there.
- Compose, don't subclass. Two simple FBs > one polymorphic super-FB with config flags.
- File on disk = file in tree = file in editor. No virtual abstraction layers.

## 3. Smooth learning curve

A first-time user should be running a "blinking variable in Monitor"
within 60 seconds of opening the IDE. A first-time agent should be able
to author a working PROGRAM by reading the API catalogue (no extra
human-oriented onboarding doc required). At every step the next action
should be obvious.

**Concretely**:
- Inline hints over modal tutorials. ("Bind a PROGRAM to a task" → the "Schedule" button next to the editor.)
- Discoverable affordances. Selected POU should make the Run button mean "run this".
- Same gesture, same effect. Run button always says what it'll do.
- Examples ship with the product. `cascade_pid`, `lorenz_attractor`, `polymer_cstr` are demo POUs; they teach by being readable.

## 4. Agent-friendly is co-equal with human-friendly

**Agents are first-class users**. We expect Claude Code, Codex, Cursor,
and future agents to drive this IDE without ever opening the GUI. This
isn't a future ambition — it's the design pivot that distinguishes us
from every existing PLC vendor's tooling, which is GUI-only.

**Concretely**:
- **API-first**: every feature reachable via REST. GUI is a thin client over the same endpoints. If a feature works in the GUI but isn't in `/api/*`, that's a bug.
- **Text-first storage**: POU sources (and future graphical languages) live in human-readable text/JSON on disk. No PLC binary project files. Grep / git diff / `cat` must work.
- **Self-describing types**: `ts-rs` exports every wire type so agents (and the IDE) can type-check requests. There is exactly one schema source of truth (the Rust struct).
- **Deterministic state**: same inputs → same outputs. No hidden mutable state in tooling that an agent can't observe.
- **Stable identifiers**: an agent that learned "POU `polymer_cstr` is in `pous/polymer_cstr.st`" yesterday must find it in the same place today.

## What this rules out

Anti-patterns to refuse — refer to these by name in code review:

- **Codesys-clone-itis**: implementing a feature because Codesys has it, when nobody asked for it and it adds three new concepts.
- **Multi-config syndrome**: every feature getting its own `.toml` / `.yaml` / "profile" — config sprawl by accumulation.
- **GUI-only features**: anything authored by mouse drag that has no REST equivalent.
- **Magic project files**: opaque binary blobs only the IDE itself can read.
- **Hidden state**: caches, locks, daemons that a user (or agent) can't observe via the API.
- **Tutorial dependency**: a feature that needs a "getting started" doc to be usable at all.

## When in doubt

Ask: "would a curious engineer who has never used a PLC understand this
in 30 seconds?" and "would an agent reading the OpenAPI schema know how
to drive this without any extra explanation?" If either answer is no,
simplify.

These principles override individual feature requests when they
conflict. They were added on 2026-05-15 after several conversations
where we caught ourselves drifting toward feature-by-feature parity with
Codesys; treat any drift in that direction as a regression on the
project's central proposition.
