import { describe, expect, it } from "vitest"

import {
  evalExpr,
  formatBinding,
  lookupVar,
  resolveBinding,
  resolveDisplay,
  toNumber,
} from "./hmi-binding"
import type { VarSnapshot } from "@/types/generated/VarSnapshot"

const snap: VarSnapshot = {
  scan_count: 1n as unknown as bigint,
  time_us: 0n as unknown as bigint,
  vars: [
    { name: "level_pct", type_name: "REAL", value: "42.5" },
    { name: "pump_run", type_name: "BOOL", value: "TRUE" },
    { name: "feeder.speed_rpm", type_name: "INT", value: "1500" },
    { name: "msg", type_name: "STRING", value: "'hello'" },
    { name: "status_w", type_name: "WORD", value: "16#1637" },
  ],
} as unknown as VarSnapshot

describe("lookupVar", () => {
  it("matches exact names and instance tails", () => {
    expect(lookupVar(snap, "level_pct")?.raw).toBe("42.5")
    expect(lookupVar(snap, "speed_rpm")?.raw).toBe("1500")
    expect(lookupVar(snap, "feeder.speed_rpm")?.raw).toBe("1500")
    expect(lookupVar(snap, "ghost")).toBeNull()
    expect(lookupVar(null, "level_pct")).toBeNull()
  })
})

describe("toNumber", () => {
  it("collapses booleans and numerics", () => {
    expect(toNumber("TRUE")).toBe(1)
    expect(toNumber("false")).toBe(0)
    expect(toNumber(" 3.5 ")).toBe(3.5)
    expect(Number.isNaN(toNumber("banana"))).toBe(true)
  })
})

describe("evalExpr", () => {
  it("does arithmetic with precedence", () => {
    expect(evalExpr("x / 100", 250)).toBe(2.5)
    expect(evalExpr("1 + x * 2", 3)).toBe(7)
    expect(evalExpr("(1 + x) * 2", 3)).toBe(8)
    expect(evalExpr("-x + 1", 4)).toBe(-3)
  })
  it("does comparisons, logic and ternary", () => {
    expect(evalExpr("x > 50", 60)).toBe(1)
    expect(evalExpr("x > 50", 40)).toBe(0)
    expect(evalExpr("x >= 10 && x <= 20", 15)).toBe(1)
    expect(evalExpr("x < 0 || x > 100", 50)).toBe(0)
    expect(evalExpr("x > 50 ? 1 : 0", 99)).toBe(1)
    expect(evalExpr("!x", 0)).toBe(1)
  })
  it("rejects everything outside the language", () => {
    expect(evalExpr("y + 1", 0)).toBeNull()
    expect(evalExpr("x(", 0)).toBeNull()
    expect(evalExpr("alert", 0)).toBeNull()
    expect(evalExpr("x; x", 0)).toBeNull()
    expect(evalExpr("", 0)).toBeNull()
  })
})

describe("resolveBinding + formatBinding", () => {
  it("resolves bare-string bindings", () => {
    expect(resolveBinding(snap, "level_pct")).toBe(42.5)
    expect(resolveBinding(snap, "pump_run")).toBe(1)
  })
  it("applies expr and format from spec bindings", () => {
    const b = { variable: "level_pct", expr: "x / 100", format: "%.3f" }
    expect(resolveBinding(snap, b)).toBe(0.425)
    expect(formatBinding(b, 0.425)).toBe("0.425")
  })
  it("formats %d and defaults sanely", () => {
    expect(formatBinding("v", 3)).toBe("3")
    expect(formatBinding("v", 3.14159)).toBe("3.14")
    expect(
      formatBinding({ variable: "v", expr: null, format: "%d" }, 3.7),
    ).toBe("4")
  })
  it("collapses non-finite results to null / em-dash, never NaN text", () => {
    // Divide-by-zero expr, expr over a STRING, plain STRING/WORD vars.
    expect(resolveBinding(snap, { variable: "level_pct", expr: "x / 0", format: null })).toBeNull()
    expect(resolveBinding(snap, { variable: "msg", expr: "x + 1", format: null })).toBeNull()
    expect(resolveBinding(snap, "msg")).toBeNull()
    expect(formatBinding("v", Infinity)).toBe("—")
    expect(formatBinding({ variable: "v", expr: null, format: "%.1f" }, NaN)).toBe("—")
  })
})

describe("resolveDisplay", () => {
  it("keeps numerics numeric", () => {
    expect(resolveDisplay(snap, "level_pct")).toBe(42.5)
    expect(
      resolveDisplay(snap, { variable: "level_pct", expr: "x / 100", format: null }),
    ).toBe(0.425)
  })
  it("surfaces STRING text and hex literals instead of NaN", () => {
    expect(resolveDisplay(snap, "msg")).toBe("hello")
    expect(resolveDisplay(snap, "status_w")).toBe("16#1637")
  })
  it("reads BOOLs as TRUE/FALSE", () => {
    expect(resolveDisplay(snap, "pump_run")).toBe("TRUE")
  })
  it("stays null for unresolvable or non-finite values", () => {
    expect(resolveDisplay(snap, "ghost")).toBeNull()
    expect(resolveDisplay(null, "level_pct")).toBeNull()
    expect(
      resolveDisplay(snap, { variable: "level_pct", expr: "x / 0", format: null }),
    ).toBeNull()
  })
})
