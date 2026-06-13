import { describe, expect, it } from "vitest"

import { parseBlockDatasheet } from "./library-datasheet"

const FB_LAG = `(*
  FB_LAG — first-order lag filter (PT1 / first-order lag).

  ~ Standard first-order lag filter (PT1 / LAG), a staple of every major
    platform's standard library.

  Purpose: analog noise rejection, measurement smoothing.
  Algorithm (backward-Euler discretization, unconditionally stable):
    out := out + dt_s / (t_s + dt_s) * (u - out)

  Inputs:
    u     REAL  input
    t_s   REAL  filter time constant s (0 = pass-through)
    dt_s  REAL  sample period s
    reset BOOL  reset/align (level-active: out follows u)
  Outputs:
    out   REAL  filtered output
*)
FUNCTION_BLOCK FB_LAG
  VAR_INPUT
    u     : REAL;
    t_s   : REAL := 1.0;
    dt_s  : REAL := 0.1;
    reset : BOOL := FALSE;
  END_VAR
  VAR_OUTPUT
    out : REAL;
  END_VAR
  out := u;
END_FUNCTION_BLOCK
`

describe("parseBlockDatasheet", () => {
  const sheet = parseBlockDatasheet(FB_LAG)

  it("extracts the FB name and brief (after the em dash)", () => {
    expect(sheet.name).toBe("FB_LAG")
    expect(sheet.brief).toBe("first-order lag filter (PT1 / first-order lag).")
  })

  it("reads pins with type, direction and declared default", () => {
    expect(sheet.inputs.map((p) => p.name)).toEqual([
      "u",
      "t_s",
      "dt_s",
      "reset",
    ])
    const ts = sheet.inputs.find((p) => p.name === "t_s")!
    expect(ts.type).toBe("REAL")
    expect(ts.direction).toBe("input")
    expect(ts.default).toBe("1.0")
    // No `:=` in the declaration → no default.
    expect(sheet.inputs.find((p) => p.name === "u")!.default).toBeUndefined()
    expect(sheet.outputs).toEqual([
      { name: "out", type: "REAL", direction: "output", default: undefined, description: "filtered output" },
    ])
  })

  it("merges per-pin descriptions from the comment table", () => {
    expect(sheet.inputs.find((p) => p.name === "t_s")!.description).toBe(
      "filter time constant s (0 = pass-through)",
    )
    expect(sheet.inputs.find((p) => p.name === "reset")!.description).toBe(
      "reset/align (level-active: out follows u)",
    )
  })

  it("splits documentation into equivalence and labelled sections", () => {
    const eq = sheet.sections.find((s) => s.equivalence)
    expect(eq?.body).toContain("Standard first-order lag filter")
    const purpose = sheet.sections.find((s) => s.label === "Purpose")
    expect(purpose?.body).toBe("analog noise rejection, measurement smoothing.")
    const algo = sheet.sections.find(
      (s) => s.label === "Algorithm (backward-Euler discretization, unconditionally stable)",
    )
    expect(algo?.body).toContain("out := out + dt_s")
    // The Inputs:/Outputs: tables are NOT prose sections.
    expect(sheet.sections.some((s) => s.label === "Inputs")).toBe(false)
  })

  it("degrades gracefully on a block with no header comment", () => {
    const bare = `FUNCTION_BLOCK FB_X
  VAR_INPUT a : BOOL; END_VAR
  VAR_OUTPUT q : BOOL; END_VAR
END_FUNCTION_BLOCK`
    const s = parseBlockDatasheet(bare)
    expect(s.name).toBe("FB_X")
    expect(s.inputs).toEqual([
      { name: "a", type: "BOOL", direction: "input", default: undefined, description: undefined },
    ])
    expect(s.brief).toBe("")
    expect(s.sections).toEqual([])
  })
})
