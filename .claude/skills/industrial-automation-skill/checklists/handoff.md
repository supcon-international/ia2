# Handoff checklist

Run before you tell the user "done". Catches the things that look fine in your terminal but leave the IDE in a confusing or unsafe state.

## Correctness

- [ ] **`cs project check ~/Documents/IA2/<name>` passes.** A clean compile of the whole project, not just the one POU you edited. Edits to shared variables or iomap can break a different POU.
- [ ] **Every iomap entry resolves.** If you added bindings, confirm each `device`/`channel`/`variable` exists (`cs device get`, `cs symbols`). Unresolved bindings warn-skip silently at run time — they won't error, they just won't do anything.
- [ ] **`tasks.programs` has exactly one entry** if the user will `cs run` the schedule. Two+ → it'll error; either fix it or tell them to use `cs run --program NAME`.

## Runtime state

- [ ] **No leftover forces.** `cs runtime status --json` → `forces` array empty. If you forced anything during debugging, `cs runtime unforce` it. A stuck force is invisible and infuriating.
- [ ] **Program is in the state the user expects.** If they asked you to leave it running, confirm `cs runtime status` shows `running: true` and the right project/program. If they asked you to set it up but not run it, `cs stop`.
- [ ] **No accidental cross-project stop.** If you ran a program in project B while the user was watching project A's program, you stopped theirs. Tell them.

## Session hygiene

- [ ] **Your agent session is closed.** If you used `cs agent run`, it auto-closed on exit — fine. If you used `cs agent enter`, confirm you called `cs agent leave` (or that the work is genuinely ongoing and you mean to leave it open). A stuck overlay reads as "the agent is still working" when it isn't.

## What to tell the user

A good handoff message includes:
- **What changed** — which POUs/devices/iomap/tasks, in which project.
- **State now** — running or stopped; if running, what they should see in the Monitor pane.
- **What's pinned/forced**, if anything (should be nothing — see above).
- **Anything you couldn't do** and why (e.g. "couldn't test the real RTU device — no serial adapter; it's configured and will connect when plugged in").
- **The next obvious step**, if there is one (e.g. "deploy to field_pi with `cs deploy field_pi` once you've cross-compiled the runtime").

## If you hit a wall

Don't paper over it. If `cs project check` won't pass, or a device won't connect, or you're unsure which project the user means — say so, show the exact error (`cs check --explain` output, the server log line), and ask. The system surfaces precise diagnostics; relay them rather than guessing.
