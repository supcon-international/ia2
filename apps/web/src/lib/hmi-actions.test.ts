import { describe, expect, it } from "vitest"

import { canHostAction } from "./hmi-actions"

describe("canHostAction", () => {
  it("allows exactly the control-surface node types", () => {
    expect(canHostAction("button")).toBe(true)
    expect(canHostAction("input")).toBe(true)
    expect(canHostAction("symbol")).toBe(true)
    expect(canHostAction("nav")).toBe(true)
  })

  it("keeps display-only node types inert", () => {
    // The regression this guards: a confirm:false write hidden behind a
    // plain text label fired on tap.
    expect(canHostAction("text")).toBe(false)
    expect(canHostAction("value")).toBe(false)
    expect(canHostAction("shape")).toBe(false)
    expect(canHostAction("trend")).toBe(false)
    expect(canHostAction("alarmbar")).toBe(false)
    expect(canHostAction("group")).toBe(false)
  })
})
