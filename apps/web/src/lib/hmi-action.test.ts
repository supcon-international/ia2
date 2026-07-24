import { describe, expect, it } from "vitest"

import {
  clampNotice,
  confirmSummary,
  parseCommitText,
  resolveActionWrite,
  type ResolvedWrite,
} from "./hmi-action"
import type { HmiAction } from "@/types/generated/HmiAction"
import type { VarSnapshot } from "@/types/generated/VarSnapshot"

type WriteAction = Exclude<HmiAction, { kind: "nav" }>
type SetValue = Extract<HmiAction, { kind: "set_value" }>

const snap: VarSnapshot = {
  scan_count: 1n as unknown as bigint,
  time_us: 0n as unknown as bigint,
  vars: [
    { name: "level_sp", type_name: "REAL", value: "50" },
    { name: "valve_cmd", type_name: "BOOL", value: "TRUE" },
    { name: "feeder.motor_run", type_name: "BOOL", value: "FALSE" },
  ],
} as unknown as VarSnapshot

const setValue = (over: Partial<SetValue> = {}): SetValue => ({
  kind: "set_value",
  variable: "level_sp",
  min: 0,
  max: 100,
  confirm: true,
  ...over,
})

const okWrite = (r: ReturnType<typeof resolveActionWrite>): ResolvedWrite => {
  if (!r.ok) throw new Error(`expected ok, got: ${r.reason}`)
  return r.write
}

describe("resolveActionWrite", () => {
  it("refuses every write kind when there is no snapshot", () => {
    const actions: WriteAction[] = [
      { kind: "write", variable: "valve_cmd", value: 1, confirm: false },
      { kind: "toggle", variable: "valve_cmd", confirm: false },
      { kind: "pulse", variable: "valve_cmd", ms: 500, confirm: false },
      setValue(),
    ]
    for (const a of actions) {
      const r = resolveActionWrite(null, a, 42)
      expect(r).toEqual({ ok: false, reason: "no live data — action not sent" })
    }
  })

  it("refuses when the variable is missing from live data", () => {
    const r = resolveActionWrite(snap, setValue({ variable: "ghost" }), 42)
    expect(r).toEqual({ ok: false, reason: "ghost not in live data — action not sent" })
  })

  it("never resolves with an unknown type — a REAL setpoint gets REAL", () => {
    // typeName "" would send encodeForWrite down the integer branch and
    // turn 75 into f32-denormal garbage on the runtime side.
    const w = okWrite(resolveActionWrite(snap, setValue(), 75))
    expect(w).toMatchObject({ variable: "level_sp", value: 75, typeName: "REAL" })
    expect(w.clamp).toBeUndefined()
  })

  it("resolves the toggle direction from the snapshot, not a default", () => {
    const on = okWrite(
      resolveActionWrite(snap, { kind: "toggle", variable: "valve_cmd", confirm: true }),
    )
    expect(on).toMatchObject({ value: 0, typeName: "BOOL", current: "TRUE" })
    const off = okWrite(
      resolveActionWrite(snap, { kind: "toggle", variable: "feeder.motor_run", confirm: true }),
    )
    expect(off).toMatchObject({ value: 1, typeName: "BOOL", current: "FALSE" })
  })

  it("clamps set_value at resolve time and records which bound clipped", () => {
    const hi = okWrite(resolveActionWrite(snap, setValue(), 999))
    expect(hi.value).toBe(100)
    expect(hi.clamp).toEqual({ entered: 999, bound: "max", limit: 100 })
    const lo = okWrite(resolveActionWrite(snap, setValue({ min: 50 }), 20))
    expect(lo.value).toBe(50)
    expect(lo.clamp).toEqual({ entered: 20, bound: "min", limit: 50 })
  })

  it("refuses set_value without a usable number", () => {
    for (const entered of [undefined, NaN, Infinity]) {
      const r = resolveActionWrite(snap, setValue(), entered)
      expect(r.ok).toBe(false)
    }
  })
})

describe("resolveActionWrite: increment", () => {
  const inc = (over: Record<string, unknown> = {}) =>
    ({
      kind: "increment",
      variable: "level_sp",
      step: 5,
      min: 0,
      max: 100,
      confirm: true,
      ...over,
    }) as WriteAction

  it("steps from the live value and reports the base", () => {
    const w = okWrite(resolveActionWrite(snap, inc()))
    expect(w.value).toBe(55)
    expect(w.from).toBe(50)
    expect(w.clamp).toBeUndefined()
  })
  it("clamps at the envelope and flags the clamp", () => {
    const w = okWrite(resolveActionWrite(snap, inc({ step: 60 })))
    expect(w.value).toBe(100)
    expect(w.clamp).toEqual({ entered: 110, bound: "max", limit: 100 })
    const down = okWrite(resolveActionWrite(snap, inc({ step: -60 })))
    expect(down.value).toBe(0)
    expect(down.clamp?.bound).toBe("min")
  })
  it("refuses without a numeric live base", () => {
    const r = resolveActionWrite(null, inc())
    expect(r.ok).toBe(false)
    const ghost = resolveActionWrite(snap, inc({ variable: "ghost" }))
    expect(ghost.ok).toBe(false)
  })
  it("summarises as a from→to step", () => {
    const w = okWrite(resolveActionWrite(snap, inc()))
    expect(confirmSummary(inc(), w)).toBe("Step level_sp: 50 → 55")
  })
})

describe("confirmSummary", () => {
  it("shows the toggle target value", () => {
    const a: WriteAction = { kind: "toggle", variable: "valve_cmd", confirm: true }
    const w = okWrite(resolveActionWrite(snap, a))
    expect(confirmSummary(a, w)).toBe("Toggle valve_cmd: TRUE → FALSE")
  })

  it("shows the clamped set_value, not the raw entry", () => {
    const a = setValue()
    const w = okWrite(resolveActionWrite(snap, a, 999))
    expect(confirmSummary(a, w)).toBe("Set level_sp = 100 (entered 999, max 100)")
    const plain = okWrite(resolveActionWrite(snap, a, 75))
    expect(confirmSummary(a, plain)).toBe("Set level_sp = 75")
  })
})

describe("clampNotice", () => {
  it("flags a clamped no-confirm entry and stays quiet otherwise", () => {
    const a = setValue({ confirm: false })
    const clamped = okWrite(resolveActionWrite(snap, a, 999))
    expect(clampNotice(clamped)).toBe("level_sp: entered 999, wrote 100 (max 100)")
    const plain = okWrite(resolveActionWrite(snap, a, 75))
    expect(clampNotice(plain)).toBeNull()
  })
})

describe("parseCommitText", () => {
  it("rejects empty and whitespace-only input (Number('') === 0)", () => {
    expect(parseCommitText("")).toBeNull()
    expect(parseCommitText("   ")).toBeNull()
  })
  it("accepts real numbers including an explicit 0", () => {
    expect(parseCommitText("0")).toBe(0)
    expect(parseCommitText(" 42.5 ")).toBe(42.5)
    expect(parseCommitText("-3")).toBe(-3)
  })
  it("rejects non-numeric and non-finite entries", () => {
    expect(parseCommitText("abc")).toBeNull()
    expect(parseCommitText("1e999")).toBeNull()
  })
})
