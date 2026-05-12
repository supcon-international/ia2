import type { VarValue } from "@/types/generated/VarValue"

export const MAX_HISTORY = 256

const HEX_RE = /^16#[0-9A-F]+$/i
const TIME_RE = /^L?TIME?#-?[\d.]+/i

/** Coerce a ironplc-formatted value string into a number for charting.
 *  TRUE/FALSE → 1/0, hex/time literals strip prefixes, everything else
 *  goes through parseFloat. NaN/Infinity fall back to 0. */
export function parseVarValue(v: VarValue): number {
  const s = v.value
  if (s === "TRUE") return 1
  if (s === "FALSE") return 0
  if (HEX_RE.test(s)) {
    const n = parseInt(s.slice(3), 16)
    return Number.isFinite(n) ? n : 0
  }
  if (TIME_RE.test(s)) {
    const m = s.match(/-?[\d.]+/)
    return m ? parseFloat(m[0]) : 0
  }
  const n = parseFloat(s)
  return Number.isFinite(n) ? n : 0
}

export function isBoolType(typeName: string): boolean {
  return typeName.toUpperCase() === "BOOL"
}

/** Push `next` onto `buf` in place, trimming from the head once we hit
 *  `MAX_HISTORY`. Returns the same array for chaining / spread. */
export function pushHistory(buf: number[], next: number): number[] {
  buf.push(next)
  if (buf.length > MAX_HISTORY) {
    // Drop ~10% at a time so we're not shift()ing every tick once full.
    buf.splice(0, buf.length - MAX_HISTORY)
  }
  return buf
}

/** A small fixed palette for pinned series; cycles if more than 8. */
export const SERIES_COLORS = [
  "#0ea5e9", // sky
  "#a855f7", // violet
  "#10b981", // emerald
  "#f97316", // orange
  "#ec4899", // pink
  "#eab308", // yellow
  "#06b6d4", // cyan
  "#ef4444", // red
] as const

export function colorFor(index: number): string {
  return SERIES_COLORS[index % SERIES_COLORS.length]
}
