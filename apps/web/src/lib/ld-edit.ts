/**
 * Pure tree-transformation library for LD POU editing.
 *
 * The renderer (`LDEditor.tsx`) translates user gestures into calls
 * here; this module never reaches DOM, never touches React state.
 * Every function takes an `LdProgram` and returns a new `LdProgram`
 * (immutable). The corollary: every edit is unit-testable in isolation,
 * and undo/redo would be a stack of these snapshots.
 *
 * Path representation:
 *
 *   - `RungPath = number`                  index into `program.rungs`
 *   - `NodePath = readonly number[]`       sequence of child indices
 *     descending into a rung's `logic` tree. `[]` is the root;
 *     `[0]` is the 0th child; `[0, 1]` is the 1st child of the 0th.
 *     For `Not` nodes the only valid step is `0` (its single `arg`).
 *
 * Why a path and not an object reference: paths survive serialise →
 * mutate → deserialise round-trips (which we do every keystroke
 * because the canonical state is the JSON source). Object references
 * would dangle after each render.
 */

import type { LdCoilKind } from "@/types/generated/LdCoilKind"
import type { LdNode } from "@/types/generated/LdNode"
import type { LdProgram } from "@/types/generated/LdProgram"
import type { LdVarSection } from "@/types/generated/LdVarSection"
import type { LdVariable } from "@/types/generated/LdVariable"

export type NodePath = readonly number[]

// =================================================================
//   Tree navigation
// =================================================================

/** Return the node at `path` within `root`. Throws on out-of-range
 *  steps — callers should always derive paths from a valid render
 *  pass, so a throw here is a real bug worth surfacing loudly. */
export function getNode(root: LdNode, path: NodePath): LdNode {
  let cur: LdNode = root
  for (let i = 0; i < path.length; i++) {
    const step = path[i]
    cur = childAt(cur, step)
  }
  return cur
}

/** Return the parent of `path` along with the child index occupied by
 *  the target. `null` when `path` is empty (the root has no parent). */
export function getParent(
  root: LdNode,
  path: NodePath,
): { parent: LdNode; childIndex: number } | null {
  if (path.length === 0) return null
  const parentPath = path.slice(0, -1)
  const parent = getNode(root, parentPath)
  return { parent, childIndex: path[path.length - 1] }
}

function childAt(node: LdNode, step: number): LdNode {
  switch (node.op) {
    case "and":
    case "or":
      if (step < 0 || step >= node.args.length) {
        throw new Error(
          `LD path step ${step} out of range for ${node.op} (len=${node.args.length})`,
        )
      }
      return node.args[step]
    case "not":
      if (step !== 0) {
        throw new Error(
          `LD path step ${step} invalid for NOT — only child 0 exists`,
        )
      }
      return node.arg
    default:
      throw new Error(
        `LD path step ${step} attempted to descend into leaf ${node.op}`,
      )
  }
}

/** Replace the node at `path` with the result of `transform(old)`,
 *  returning a structurally-shared copy of `root`. */
function updateNode(
  root: LdNode,
  path: NodePath,
  transform: (n: LdNode) => LdNode,
): LdNode {
  if (path.length === 0) return transform(root)
  const [step, ...rest] = path
  switch (root.op) {
    case "and":
    case "or": {
      const args = root.args.slice()
      args[step] = updateNode(args[step], rest, transform)
      return { ...root, args }
    }
    case "not":
      if (step !== 0) throw new Error("NOT step must be 0")
      return { ...root, arg: updateNode(root.arg, rest, transform) }
    default:
      throw new Error(`cannot descend into leaf ${root.op}`)
  }
}

// =================================================================
//   Rung-level operations
// =================================================================

export function addRung(prog: LdProgram, atIndex?: number): LdProgram {
  const id = nextRungId(prog)
  // A fresh rung is a single placeholder contact wired to no coil yet —
  // the user picks a variable next. Coil-less rungs would fail the
  // transpiler's "dead-code" check, so we add a `<unbound>` coil that
  // the user is meant to immediately replace.
  const newRung = {
    id,
    label: null,
    logic: { op: "const", value: true } as LdNode,
    coils: [],
  }
  const idx = atIndex ?? prog.rungs.length
  const rungs = prog.rungs.slice()
  rungs.splice(idx, 0, newRung)
  return { ...prog, rungs }
}

export function deleteRung(prog: LdProgram, idx: number): LdProgram {
  if (idx < 0 || idx >= prog.rungs.length) return prog
  const rungs = prog.rungs.slice()
  rungs.splice(idx, 1)
  return { ...prog, rungs }
}

export function moveRung(
  prog: LdProgram,
  from: number,
  to: number,
): LdProgram {
  if (from === to) return prog
  if (from < 0 || from >= prog.rungs.length) return prog
  if (to < 0 || to >= prog.rungs.length) return prog
  const rungs = prog.rungs.slice()
  const [r] = rungs.splice(from, 1)
  rungs.splice(to, 0, r)
  return { ...prog, rungs }
}

export function setRungLabel(
  prog: LdProgram,
  rungIdx: number,
  label: string | null,
): LdProgram {
  return updateRung(prog, rungIdx, (r) => ({ ...r, label }))
}

function updateRung(
  prog: LdProgram,
  rungIdx: number,
  transform: (r: LdProgram["rungs"][number]) => LdProgram["rungs"][number],
): LdProgram {
  if (rungIdx < 0 || rungIdx >= prog.rungs.length) return prog
  const rungs = prog.rungs.slice()
  rungs[rungIdx] = transform(rungs[rungIdx])
  return { ...prog, rungs }
}

function nextRungId(prog: LdProgram): string {
  // r0, r1, ... up to one past the highest existing numbered id. If
  // the user has custom ids, we still pick a non-colliding rN.
  const used = new Set(prog.rungs.map((r) => r.id))
  let n = prog.rungs.length
  while (used.has(`r${n}`)) n++
  return `r${n}`
}

// =================================================================
//   Logic-tree operations (operate within a single rung)
// =================================================================

/** Replace the node at `path` within rung `rungIdx`. */
export function replaceNode(
  prog: LdProgram,
  rungIdx: number,
  path: NodePath,
  next: LdNode,
): LdProgram {
  return updateRung(prog, rungIdx, (r) => ({
    ...r,
    logic: updateNode(r.logic, path, () => next),
  }))
}

/** Toggle the `negated` flag on a contact. No-op if the target isn't
 *  a contact. */
export function toggleNegated(
  prog: LdProgram,
  rungIdx: number,
  path: NodePath,
): LdProgram {
  return updateRung(prog, rungIdx, (r) => ({
    ...r,
    logic: updateNode(r.logic, path, (n) =>
      n.op === "contact" ? { ...n, negated: !n.negated } : n,
    ),
  }))
}

/** Change the variable referenced by a contact. */
export function setContactVar(
  prog: LdProgram,
  rungIdx: number,
  path: NodePath,
  varName: string,
): LdProgram {
  return updateRung(prog, rungIdx, (r) => ({
    ...r,
    logic: updateNode(r.logic, path, (n) =>
      n.op === "contact" ? { ...n, var: varName } : n,
    ),
  }))
}

/**
 * Insert a new contact in series with the node at `path`. "Series"
 * means: extend the surrounding AND. If the target's parent is an AND,
 * append a sibling. Otherwise wrap the target in a fresh AND with the
 * new contact as the second sibling.
 *
 * `side`: "after" places the new contact to the right of the target,
 * "before" to the left.
 */
export function addInSeries(
  prog: LdProgram,
  rungIdx: number,
  path: NodePath,
  side: "before" | "after",
  newNode: LdNode,
): LdProgram {
  return updateRung(prog, rungIdx, (r) => {
    const root = r.logic
    if (path.length === 0) {
      // Special case: root is already an AND — just extend it instead
      // of wrapping in another AND. Keeps the tree flat for common
      // "keep adding contacts to the same series" authoring flow.
      if (root.op === "and") {
        const args = side === "after" ? [...root.args, newNode] : [newNode, ...root.args]
        return { ...r, logic: { op: "and", args } }
      }
      // Adding in series with the root: wrap root in AND.
      const args = side === "after" ? [root, newNode] : [newNode, root]
      return { ...r, logic: { op: "and", args } }
    }
    const parentPath = path.slice(0, -1)
    const childIdx = path[path.length - 1]
    const parent = getNode(root, parentPath)
    if (parent.op === "and") {
      // Append a sibling without restructuring.
      const insertAt = side === "after" ? childIdx + 1 : childIdx
      const args = [...parent.args]
      args.splice(insertAt, 0, newNode)
      const newParent: LdNode = { op: "and", args }
      return {
        ...r,
        logic: updateNode(root, parentPath, () => newParent),
      }
    }
    // Wrap the target in a fresh AND.
    return {
      ...r,
      logic: updateNode(root, path, (target) => ({
        op: "and",
        args:
          side === "after" ? [target, newNode] : [newNode, target],
      })),
    }
  })
}

/**
 * Insert a new contact in parallel with the node at `path`. "Parallel"
 * extends the surrounding OR; if the parent isn't OR, wrap.
 *
 * `side`: "after" places the new branch below, "before" above.
 */
export function addInParallel(
  prog: LdProgram,
  rungIdx: number,
  path: NodePath,
  side: "before" | "after",
  newNode: LdNode,
): LdProgram {
  return updateRung(prog, rungIdx, (r) => {
    const root = r.logic
    if (path.length === 0) {
      // Special case: root is already an OR — append rather than
      // nest another OR. Symmetric to addInSeries.
      if (root.op === "or") {
        const args = side === "after" ? [...root.args, newNode] : [newNode, ...root.args]
        return { ...r, logic: { op: "or", args } }
      }
      const args = side === "after" ? [root, newNode] : [newNode, root]
      return { ...r, logic: { op: "or", args } }
    }
    const parentPath = path.slice(0, -1)
    const childIdx = path[path.length - 1]
    const parent = getNode(root, parentPath)
    if (parent.op === "or") {
      const insertAt = side === "after" ? childIdx + 1 : childIdx
      const args = [...parent.args]
      args.splice(insertAt, 0, newNode)
      const newParent: LdNode = { op: "or", args }
      return {
        ...r,
        logic: updateNode(root, parentPath, () => newParent),
      }
    }
    return {
      ...r,
      logic: updateNode(root, path, (target) => ({
        op: "or",
        args:
          side === "after" ? [target, newNode] : [newNode, target],
      })),
    }
  })
}

/**
 * Delete the node at `path`. Removes it from its parent's child list;
 * if the parent then has exactly one remaining child, the parent
 * collapses into that child (avoids dangling 1-arg AND/OR nodes).
 *
 * Deleting the root replaces the rung's logic with `const true` —
 * we never leave a rung with no logic, because the transpiler can't
 * emit ST for an empty network.
 */
export function deleteNode(
  prog: LdProgram,
  rungIdx: number,
  path: NodePath,
): LdProgram {
  return updateRung(prog, rungIdx, (r) => {
    if (path.length === 0) {
      return { ...r, logic: { op: "const", value: true } }
    }
    const parentPath = path.slice(0, -1)
    const childIdx = path[path.length - 1]
    const parent = getNode(r.logic, parentPath)
    if (parent.op === "and" || parent.op === "or") {
      const args = parent.args.filter((_, i) => i !== childIdx)
      const collapsed: LdNode =
        args.length === 1
          ? args[0]
          : args.length === 0
            ? // Empty AND/OR shouldn't happen in practice, but collapse
              // safely to identity.
              { op: "const", value: parent.op === "and" }
            : { ...parent, args }
      return {
        ...r,
        logic: updateNode(r.logic, parentPath, () => collapsed),
      }
    }
    if (parent.op === "not") {
      // Deleting the sole child of NOT collapses the NOT to identity.
      return {
        ...r,
        logic: updateNode(r.logic, parentPath, () => ({
          op: "const",
          value: true,
        })),
      }
    }
    return r
  })
}

// =================================================================
//   Coil operations
// =================================================================

export function addCoil(
  prog: LdProgram,
  rungIdx: number,
  coilVar: string,
  kind: LdCoilKind = "standard",
): LdProgram {
  return updateRung(prog, rungIdx, (r) => ({
    ...r,
    coils: [...r.coils, { var: coilVar, kind }],
  }))
}

export function deleteCoil(
  prog: LdProgram,
  rungIdx: number,
  coilIdx: number,
): LdProgram {
  return updateRung(prog, rungIdx, (r) => ({
    ...r,
    coils: r.coils.filter((_, i) => i !== coilIdx),
  }))
}

export function setCoilVar(
  prog: LdProgram,
  rungIdx: number,
  coilIdx: number,
  varName: string,
): LdProgram {
  return updateRung(prog, rungIdx, (r) => {
    const coils = r.coils.slice()
    if (coils[coilIdx]) coils[coilIdx] = { ...coils[coilIdx], var: varName }
    return { ...r, coils }
  })
}

export function setCoilKind(
  prog: LdProgram,
  rungIdx: number,
  coilIdx: number,
  kind: LdCoilKind,
): LdProgram {
  return updateRung(prog, rungIdx, (r) => {
    const coils = r.coils.slice()
    if (coils[coilIdx]) coils[coilIdx] = { ...coils[coilIdx], kind }
    return { ...r, coils }
  })
}

// =================================================================
//   Variable operations
// =================================================================

export function addVariable(
  prog: LdProgram,
  v: LdVariable,
): LdProgram {
  // Reject duplicate names rather than silently overwriting — the
  // UI should validate this before calling, but defending the model.
  if (prog.variables.some((x) => x.name === v.name)) return prog
  return { ...prog, variables: [...prog.variables, v] }
}

export function removeVariable(prog: LdProgram, name: string): LdProgram {
  return {
    ...prog,
    variables: prog.variables.filter((v) => v.name !== name),
  }
}

export function updateVariable(
  prog: LdProgram,
  name: string,
  patch: Partial<LdVariable>,
): LdProgram {
  return {
    ...prog,
    variables: prog.variables.map((v) =>
      v.name === name ? { ...v, ...patch } : v,
    ),
  }
}

/** All BOOL variables (or any type, if `restrictType` is null). Used to
 *  populate the variable dropdown when authoring a contact/coil. */
export function variableNames(
  prog: LdProgram,
  restrictType: string | null = "BOOL",
): string[] {
  return prog.variables
    .filter((v) => restrictType === null || v.type === restrictType)
    .map((v) => v.name)
}

/** Group variables by section in canonical render order. */
export function variablesBySection(prog: LdProgram): Record<
  LdVarSection,
  LdVariable[]
> {
  const groups: Record<LdVarSection, LdVariable[]> = {
    input: [],
    output: [],
    internal: [],
  }
  for (const v of prog.variables) groups[v.section].push(v)
  return groups
}

// =================================================================
//   Construction helpers
// =================================================================

export function newContact(varName = "x", negated = false): LdNode {
  return { op: "contact", var: varName, negated }
}

export function newConst(value: boolean): LdNode {
  return { op: "const", value }
}

// =================================================================
//   JSON round-trip
// =================================================================

/** Parse a source string into a typed program. Throws on invalid
 *  JSON; the caller (LDEditor) treats that as a parse error and
 *  falls back to the JSON-text view. */
export function parseProgram(source: string): LdProgram {
  return JSON.parse(source) as LdProgram
}

/** Serialise a program back to the canonical pretty-JSON form. The
 *  IDE writes this on save; it's what lands in `pous/<slug>.ld.json`. */
export function serializeProgram(prog: LdProgram): string {
  return JSON.stringify(prog, null, 2) + "\n"
}
