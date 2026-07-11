/**
 * Pure tree-transformation library for SFC POU editing.
 *
 * Same role `ld-edit.ts` and `fbd-edit.ts` play for their editors:
 * every gesture in the SFC canvas translates to one or more calls
 * here, producing a fresh `SfcProgram` value. No DOM, no React.
 */

import type { SfcAction } from "@/types/generated/SfcAction"
import type { SfcProgram } from "@/types/generated/SfcProgram"
import type { SfcQualifier } from "@/types/generated/SfcQualifier"
import type { SfcStep } from "@/types/generated/SfcStep"
import type { SfcTransition } from "@/types/generated/SfcTransition"

// =================================================================
//   JSON round-trip + shared variable CRUD
// =================================================================

// `serializeProgram` and the three variable mutators are byte-identical
// across the graphical editors — they live in `program-vars` now and
// re-export here so this module's public API is unchanged.
export {
  addVariable,
  removeVariable,
  serializeProgram,
  updateVariable,
} from "./program-vars"
import { parseProgramJson } from "./program-vars"

export function parseProgram(source: string): SfcProgram {
  return parseProgramJson<SfcProgram>(source)
}

// =================================================================
//   Steps
// =================================================================

/** Append a new step with auto-numbered name. The first step also
 *  becomes the program's `initial_step` if there was none. */
export function addStep(prog: SfcProgram): { prog: SfcProgram; name: string } {
  const name = nextStepName(prog)
  const step: SfcStep = { name, actions: [] }
  const next: SfcProgram = {
    ...prog,
    steps: [...prog.steps, step],
    initial_step: prog.initial_step || name,
  }
  return { prog: next, name }
}

/**
 * Remove a step by name. Any transition pointing AT the removed step
 * (as from-end or to-end) is dropped — otherwise the program would
 * reference a non-existent step and the transpiler would refuse it.
 *
 * If the removed step was the initial step, the first remaining step
 * (or empty string) becomes the new initial.
 */
export function removeStep(prog: SfcProgram, name: string): SfcProgram {
  const steps = prog.steps.filter((s) => s.name !== name)
  const transitions = prog.transitions.filter(
    (t) => t.from !== name && t.to !== name,
  )
  const initial_step =
    prog.initial_step === name ? steps[0]?.name ?? "" : prog.initial_step
  return { ...prog, steps, transitions, initial_step }
}

/** Rename a step. Updates every transition that references it.
 *  Refuses if the new name collides with another step or is empty /
 *  contains a single quote (would break the lowered STRING literal). */
export function renameStep(
  prog: SfcProgram,
  oldName: string,
  newName: string,
): SfcProgram {
  const trimmed = newName.trim()
  if (!trimmed || trimmed.includes("'")) return prog
  if (trimmed === oldName) return prog
  if (prog.steps.some((s) => s.name === trimmed)) return prog
  const steps = prog.steps.map((s) =>
    s.name === oldName ? { ...s, name: trimmed } : s,
  )
  const transitions = prog.transitions.map((t) => ({
    ...t,
    from: t.from === oldName ? trimmed : t.from,
    to: t.to === oldName ? trimmed : t.to,
  }))
  const initial_step =
    prog.initial_step === oldName ? trimmed : prog.initial_step
  return { ...prog, steps, transitions, initial_step }
}

/** Mark a step as the program's entry point. No-op if the name
 *  doesn't refer to a declared step. */
export function setInitialStep(prog: SfcProgram, name: string): SfcProgram {
  if (!prog.steps.some((s) => s.name === name)) return prog
  return { ...prog, initial_step: name }
}

/** Reorder steps (move from `from` index to `to` index). Pure UI —
 *  doesn't affect execution semantics, but matches authoring intent
 *  in the rendered vertical flow. */
export function moveStep(
  prog: SfcProgram,
  from: number,
  to: number,
): SfcProgram {
  if (from === to) return prog
  if (from < 0 || from >= prog.steps.length) return prog
  if (to < 0 || to >= prog.steps.length) return prog
  const steps = prog.steps.slice()
  const [s] = steps.splice(from, 1)
  steps.splice(to, 0, s)
  return { ...prog, steps }
}

function nextStepName(prog: SfcProgram): string {
  const used = new Set(prog.steps.map((s) => s.name))
  let n = prog.steps.length + 1
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const candidate = `step${n}`
    if (!used.has(candidate)) return candidate
    n += 1
  }
}

// =================================================================
//   Actions (on a single step)
// =================================================================

/** Append an action to a step. */
export function addAction(
  prog: SfcProgram,
  stepName: string,
  action: SfcAction = { qualifier: "N", body: "" },
): SfcProgram {
  return updateStep(prog, stepName, (s) => ({
    ...s,
    actions: [...s.actions, action],
  }))
}

export function removeAction(
  prog: SfcProgram,
  stepName: string,
  index: number,
): SfcProgram {
  return updateStep(prog, stepName, (s) => ({
    ...s,
    actions: s.actions.filter((_, i) => i !== index),
  }))
}

export function updateAction(
  prog: SfcProgram,
  stepName: string,
  index: number,
  patch: Partial<SfcAction>,
): SfcProgram {
  return updateStep(prog, stepName, (s) => ({
    ...s,
    actions: s.actions.map((a, i) => (i === index ? { ...a, ...patch } : a)),
  }))
}

export function setActionQualifier(
  prog: SfcProgram,
  stepName: string,
  index: number,
  qualifier: SfcQualifier,
): SfcProgram {
  return updateAction(prog, stepName, index, { qualifier })
}

export function setActionBody(
  prog: SfcProgram,
  stepName: string,
  index: number,
  body: string,
): SfcProgram {
  return updateAction(prog, stepName, index, { body })
}

function updateStep(
  prog: SfcProgram,
  stepName: string,
  transform: (s: SfcStep) => SfcStep,
): SfcProgram {
  if (!prog.steps.some((s) => s.name === stepName)) return prog
  return {
    ...prog,
    steps: prog.steps.map((s) => (s.name === stepName ? transform(s) : s)),
  }
}

// =================================================================
//   Transitions
// =================================================================

/** Append a transition. Refuses if either endpoint doesn't exist as
 *  a declared step. The condition may be empty initially — the user
 *  is expected to fill it in via the detail bar; the transpiler will
 *  reject it on save until they do. */
export function addTransition(
  prog: SfcProgram,
  from: string,
  to: string,
  condition: string = "TRUE",
): SfcProgram {
  const knownSteps = new Set(prog.steps.map((s) => s.name))
  if (!knownSteps.has(from) || !knownSteps.has(to)) return prog
  const t: SfcTransition = { from, to, condition }
  return { ...prog, transitions: [...prog.transitions, t] }
}

export function removeTransition(prog: SfcProgram, index: number): SfcProgram {
  return {
    ...prog,
    transitions: prog.transitions.filter((_, i) => i !== index),
  }
}

export function updateTransition(
  prog: SfcProgram,
  index: number,
  patch: Partial<SfcTransition>,
): SfcProgram {
  return {
    ...prog,
    transitions: prog.transitions.map((t, i) =>
      i === index ? { ...t, ...patch } : t,
    ),
  }
}

/** Reorder transitions. The transpiler emits an IF/ELSIF cascade
 *  where author order = priority — first matching condition wins.
 *  So this is semantically meaningful, not just cosmetic. */
export function moveTransition(
  prog: SfcProgram,
  from: number,
  to: number,
): SfcProgram {
  if (from === to) return prog
  if (from < 0 || from >= prog.transitions.length) return prog
  if (to < 0 || to >= prog.transitions.length) return prog
  const transitions = prog.transitions.slice()
  const [t] = transitions.splice(from, 1)
  transitions.splice(to, 0, t)
  return { ...prog, transitions }
}

// Variable CRUD (addVariable / removeVariable / updateVariable) is
// shared — see the re-export from `program-vars` above.
