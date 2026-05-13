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

/** Category a variable's type falls into. Drives how Monitor renders
 *  it — numeric gets a sparkline, bool a chip, bits a hex pill, time a
 *  pretty duration, text plain text, and FB instances are skipped from
 *  the trace because they're internal byte-offset bookkeeping.
 *
 *  Categorisation is by the type-name string the bridge reports
 *  (`format_variable_value` already preserves these via ironplc's
 *  debug section), so this is the one place to extend when new IEC
 *  types come online. */
export type VarCategory =
  | "bool"
  | "numeric"
  | "bits"
  | "time"
  | "text"
  | "fb"
  | "other"

const NUMERIC_TYPES = new Set([
  "REAL",
  "LREAL",
  "SINT",
  "INT",
  "DINT",
  "LINT",
  "USINT",
  "UINT",
  "UDINT",
  "ULINT",
])
const BIT_TYPES = new Set(["BYTE", "WORD", "DWORD", "LWORD"])
const TIME_TYPES = new Set([
  "TIME",
  "LTIME",
  "DATE",
  "LDATE",
  "TIME_OF_DAY",
  "TOD",
  "DATE_AND_TIME",
  "DT",
])
const TEXT_TYPES = new Set(["STRING", "WSTRING"])

export function classifyType(typeName: string): VarCategory {
  const t = typeName.toUpperCase()
  if (t === "BOOL") return "bool"
  if (NUMERIC_TYPES.has(t)) return "numeric"
  if (BIT_TYPES.has(t)) return "bits"
  if (TIME_TYPES.has(t)) return "time"
  if (TEXT_TYPES.has(t)) return "text"
  // User-defined FB instance types come through with the FB name as the
  // type (e.g. "PID", "ARRHENIUS"). They're scratch storage and have no
  // meaningful single-value display.
  if (/^[A-Z][A-Z0-9_]*$/.test(t)) return "fb"
  return "other"
}

/** Pretty-format a `TIME` / `LTIME` value the bridge serialised as
 *  `T#NNNms` into something a process operator would read. Sub-second:
 *  "750 ms". Seconds: "12.3 s". Minutes: "1m 23s". Hours similar. */
export function prettyTime(raw: string): string {
  const m = raw.match(/-?\d+(?:\.\d+)?/)
  if (!m) return raw
  const ms = parseFloat(m[0])
  if (!Number.isFinite(ms)) return raw
  const abs = Math.abs(ms)
  const sign = ms < 0 ? "-" : ""
  if (abs < 1000) return `${sign}${abs.toFixed(0)} ms`
  if (abs < 60_000) return `${sign}${(abs / 1000).toFixed(abs < 10_000 ? 2 : 1)} s`
  const mins = Math.floor(abs / 60_000)
  const secs = Math.round((abs % 60_000) / 1000)
  if (abs < 3_600_000) return `${sign}${mins}m ${secs}s`
  const hours = Math.floor(abs / 3_600_000)
  const restMin = Math.floor((abs % 3_600_000) / 60_000)
  return `${sign}${hours}h ${restMin}m`
}

/** Strip the `16#` prefix some bridge-formatted hex values carry so
 *  the row can render the digits with its own styling. */
export function stripHexPrefix(raw: string): string {
  return raw.startsWith("16#") ? raw.slice(3) : raw
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
