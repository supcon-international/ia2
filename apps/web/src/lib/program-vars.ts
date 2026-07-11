/**
 * Shared pure CRUD for the inline VAR list every graphical POU carries.
 *
 * LD, FBD and SFC programs all declare their variables as an identical
 * `Array<LdVariable>` (see each generated `*Program` type). The JSON
 * round-trip (`parseProgram` / `serializeProgram`) plus the three
 * variable mutators were byte-identical across `ld-edit.ts`,
 * `fbd-edit.ts` and `sfc-edit.ts`; they live here once and each module
 * re-exports them so its public API — and the vitest suites that import
 * it — stay unchanged.
 *
 * Everything is pure / immutable, exactly like the per-language edit
 * libraries: take a program, return a fresh program, never mutate. That
 * is what keeps the JSON-on-disk round-trip (serialise every keystroke,
 * re-parse next render) honest.
 */

import type { LdVariable } from "@/types/generated/LdVariable"

/** Structural bound satisfied by `LdProgram` / `FbdProgram` /
 *  `SfcProgram` — they all carry the same inline `Array<LdVariable>`. */
export interface HasVariables {
  variables: LdVariable[]
}

/** Parse a source string into a typed program. Throws on invalid JSON;
 *  callers treat that as a parse error and fall back to the raw view.
 *  Generic in the program type — each edit module wraps it with its own
 *  concrete return type. */
export function parseProgramJson<P>(source: string): P {
  return JSON.parse(source) as P
}

/** Serialise a program back to the canonical pretty-JSON form the IDE
 *  writes on save (trailing newline included). */
export function serializeProgram<P>(prog: P): string {
  return JSON.stringify(prog, null, 2) + "\n"
}

/** Append a variable, rejecting a duplicate name rather than silently
 *  overwriting — the UI should validate first, but we defend the model. */
export function addVariable<P extends HasVariables>(prog: P, v: LdVariable): P {
  if (prog.variables.some((x) => x.name === v.name)) return prog
  return { ...prog, variables: [...prog.variables, v] } as P
}

export function removeVariable<P extends HasVariables>(prog: P, name: string): P {
  return {
    ...prog,
    variables: prog.variables.filter((v) => v.name !== name),
  } as P
}

export function updateVariable<P extends HasVariables>(
  prog: P,
  name: string,
  patch: Partial<LdVariable>,
): P {
  return {
    ...prog,
    variables: prog.variables.map((v) =>
      v.name === name ? { ...v, ...patch } : v,
    ),
  } as P
}
