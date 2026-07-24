/**
 * Action-write resolution for the HMI canvas. Everything that decides
 * WHAT a tap/commit will write is resolved here, against the snapshot
 * the operator is looking at, BEFORE any confirm card renders:
 *
 *   - the variable's type — refused when live data can't supply one,
 *     because an unknown type would integer-encode a REAL setpoint
 *     into denormal garbage;
 *   - the toggle direction — the inverse of the on-screen value, not
 *     of whatever snapshot happens to exist after Confirm;
 *   - the set_value clamp — applied before the dialog so it shows
 *     exactly the number that will be written.
 */

import type { HmiAction } from "@/types/generated/HmiAction"
import type { VarSnapshot } from "@/types/generated/VarSnapshot"

import { lookupVar, toNumber } from "./hmi-binding"

/** A write fully resolved at request time: `value` is exactly what the
 *  confirm (or no-confirm) path will send. */
export type ResolvedWrite = {
  variable: string
  value: number
  typeName: string
  /** Toggle only: the live value the inverse was taken from. */
  current?: "TRUE" | "FALSE"
  /** set_value/increment: present when the min/max clamp changed the
   *  outcome. */
  clamp?: { entered: number; bound: "min" | "max"; limit: number }
  /** increment only: the live value the step was applied to. */
  from?: number
}

export type ResolveResult =
  | { ok: true; write: ResolvedWrite }
  | { ok: false; reason: string }

/** Resolve a variable-writing action. Refuses — rather than guesses —
 *  when the type or toggle direction can't be resolved from live data:
 *  a wrong write to a plant is worse than no write. */
export function resolveActionWrite(
  snapshot: VarSnapshot | null,
  action: Exclude<HmiAction, { kind: "nav" }>,
  entered?: number,
): ResolveResult {
  const found = lookupVar(snapshot, action.variable)
  if (!found || !found.type_name) {
    return {
      ok: false,
      reason: snapshot
        ? `${action.variable} not in live data — action not sent`
        : "no live data — action not sent",
    }
  }
  const typeName = found.type_name
  switch (action.kind) {
    case "write":
      return {
        ok: true,
        write: { variable: action.variable, value: action.value, typeName },
      }
    case "toggle": {
      const on = /^(true|1)$/i.test(found.raw.trim())
      return {
        ok: true,
        write: {
          variable: action.variable,
          value: on ? 0 : 1,
          typeName,
          current: on ? "TRUE" : "FALSE",
        },
      }
    }
    case "pulse":
      // The 1-half; the canvas schedules the 0 with the same type.
      return {
        ok: true,
        write: { variable: action.variable, value: 1, typeName },
      }
    case "set_value": {
      if (entered == null || !Number.isFinite(entered)) {
        return { ok: false, reason: "no numeric value entered — action not sent" }
      }
      const lo = action.min ?? -Infinity
      const hi = action.max ?? Infinity
      const value = Math.min(hi, Math.max(lo, entered))
      const write: ResolvedWrite = { variable: action.variable, value, typeName }
      if (value !== entered) {
        write.clamp =
          entered < lo
            ? { entered, bound: "min", limit: lo }
            : { entered, bound: "max", limit: hi }
      }
      return { ok: true, write }
    }
    case "increment": {
      // Step from the LIVE value — no live number, no step: guessing a
      // base for a relative write is exactly the wrong-write category
      // this resolver exists to refuse.
      const from = toNumber(found.raw)
      if (!Number.isFinite(from)) {
        return {
          ok: false,
          reason: `${action.variable} has no numeric live value — action not sent`,
        }
      }
      const lo = action.min ?? -Infinity
      const hi = action.max ?? Infinity
      const wanted = from + action.step
      const value = Math.min(hi, Math.max(lo, wanted))
      const write: ResolvedWrite = { variable: action.variable, value, typeName, from }
      if (value !== wanted) {
        write.clamp =
          wanted < lo
            ? { entered: wanted, bound: "min", limit: lo }
            : { entered: wanted, bound: "max", limit: hi }
      }
      return { ok: true, write }
    }
  }
}

/** The confirm card's one-line summary — built from the RESOLVED write
 *  so the operator confirms exactly what will be sent. */
export function confirmSummary(action: HmiAction, write: ResolvedWrite): string {
  switch (action.kind) {
    case "write":
      return `Write ${write.value} → ${write.variable}`
    case "toggle":
      return `Toggle ${write.variable}: ${write.current} → ${write.value === 0 ? "FALSE" : "TRUE"}`
    case "pulse":
      return `Pulse ${write.variable} (${action.ms} ms)`
    case "set_value":
      return write.clamp
        ? `Set ${write.variable} = ${write.value} (entered ${write.clamp.entered}, ${write.clamp.bound} ${write.clamp.limit})`
        : `Set ${write.variable} = ${write.value}`
    case "increment":
      return write.clamp
        ? `Step ${write.variable}: ${write.from} → ${write.value} (${write.clamp.bound} ${write.clamp.limit})`
        : `Step ${write.variable}: ${write.from} → ${write.value}`
    case "nav":
      return ""
  }
}

/** Strip text when a no-confirm set_value entry was clamped — the write
 *  happens, but never silently different from what was typed. */
export function clampNotice(write: ResolvedWrite): string | null {
  if (!write.clamp) return null
  return `${write.variable}: entered ${write.clamp.entered}, wrote ${write.value} (${write.clamp.bound} ${write.clamp.limit})`
}

/** Parse an Input node's committed text. Returns null for anything that
 *  must not write — empty/whitespace (Number("") is 0!) and non-numbers. */
export function parseCommitText(text: string): number | null {
  const t = text.trim()
  if (t === "") return null
  const v = Number(t)
  return Number.isFinite(v) ? v : null
}
