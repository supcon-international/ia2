import { describe, expect, it } from "vitest"

import type { SfcProgram } from "@/types/generated/SfcProgram"
import {
  addAction,
  addStep,
  addTransition,
  moveStep,
  moveTransition,
  removeAction,
  removeStep,
  removeTransition,
  renameStep,
  setActionBody,
  setActionQualifier,
  setInitialStep,
  updateTransition,
} from "./sfc-edit"

function seed(): SfcProgram {
  return {
    name: "demo",
    pou_type: "program",
    variables: [
      { name: "start", type: "BOOL", section: "input", init: null },
    ],
    initial_step: "idle",
    steps: [
      { name: "idle", actions: [] },
      {
        name: "running",
        actions: [{ qualifier: "N", body: "motor := TRUE" }],
      },
    ],
    transitions: [
      { from: "idle", to: "running", condition: "start" },
      { from: "running", to: "idle", condition: "NOT start" },
    ],
  }
}

describe("addStep", () => {
  it("appends a new step with auto-numbered name", () => {
    const { prog, name } = addStep(seed())
    // seed has 2 steps; nextStepName starts from steps.length + 1 = 3
    expect(name).toBe("step3")
    expect(prog.steps).toHaveLength(3)
  })

  it("becomes initial when adding to empty steps[]", () => {
    const empty: SfcProgram = {
      name: "p",
      pou_type: "program",
      variables: [],
      initial_step: "",
      steps: [],
      transitions: [],
    }
    const { prog, name } = addStep(empty)
    expect(prog.initial_step).toBe(name)
  })
})

describe("removeStep", () => {
  it("drops transitions that reference the removed step", () => {
    const next = removeStep(seed(), "running")
    expect(next.steps.map((s) => s.name)).toEqual(["idle"])
    expect(next.transitions).toEqual([])
  })

  it("re-picks initial_step if it was the removed one", () => {
    const next = removeStep(seed(), "idle")
    expect(next.initial_step).toBe("running")
  })

  it("doesn't touch other transitions", () => {
    const start: SfcProgram = {
      ...seed(),
      steps: [
        ...seed().steps,
        { name: "fault", actions: [] },
      ],
      transitions: [
        ...seed().transitions,
        { from: "running", to: "fault", condition: "estop" },
        { from: "fault", to: "idle", condition: "reset" },
      ],
    }
    const next = removeStep(start, "fault")
    // fault → all transitions touching fault drop
    expect(next.transitions).toEqual([
      { from: "idle", to: "running", condition: "start" },
      { from: "running", to: "idle", condition: "NOT start" },
    ])
  })
})

describe("renameStep", () => {
  it("renames the step and all referring transitions", () => {
    const next = renameStep(seed(), "running", "active")
    expect(next.steps.find((s) => s.name === "active")).toBeTruthy()
    expect(next.transitions).toEqual([
      { from: "idle", to: "active", condition: "start" },
      { from: "active", to: "idle", condition: "NOT start" },
    ])
  })

  it("rejects collision with another step", () => {
    const next = renameStep(seed(), "running", "idle")
    expect(next.steps.map((s) => s.name)).toEqual(["idle", "running"])
  })

  it("rejects names containing single quotes", () => {
    const next = renameStep(seed(), "idle", "it's bad")
    expect(next.steps[0].name).toBe("idle")
  })

  it("updates initial_step if renamed", () => {
    const next = renameStep(seed(), "idle", "home")
    expect(next.initial_step).toBe("home")
  })
})

describe("setInitialStep", () => {
  it("changes initial", () => {
    expect(setInitialStep(seed(), "running").initial_step).toBe("running")
  })

  it("rejects unknown name", () => {
    expect(setInitialStep(seed(), "ghost").initial_step).toBe("idle")
  })
})

describe("moveStep", () => {
  it("reorders", () => {
    const next = moveStep(seed(), 0, 1)
    expect(next.steps.map((s) => s.name)).toEqual(["running", "idle"])
  })
})

describe("actions", () => {
  it("addAction appends to the named step", () => {
    const next = addAction(seed(), "idle", { qualifier: "S", body: "x := 1" })
    expect(next.steps[0].actions).toEqual([
      { qualifier: "S", body: "x := 1" },
    ])
  })

  it("removeAction drops by index", () => {
    const next = removeAction(seed(), "running", 0)
    expect(next.steps[1].actions).toEqual([])
  })

  it("setActionQualifier switches N → S", () => {
    const next = setActionQualifier(seed(), "running", 0, "S")
    expect(next.steps[1].actions[0].qualifier).toBe("S")
  })

  it("setActionBody updates body, preserves qualifier", () => {
    const next = setActionBody(seed(), "running", 0, "drum := TRUE")
    expect(next.steps[1].actions[0]).toEqual({
      qualifier: "N",
      body: "drum := TRUE",
    })
  })
})

describe("transitions", () => {
  it("addTransition appends with the supplied condition", () => {
    const next = addTransition(seed(), "idle", "running", "start_btn")
    expect(next.transitions).toHaveLength(3)
    expect(next.transitions[2]).toEqual({
      from: "idle",
      to: "running",
      condition: "start_btn",
    })
  })

  it("addTransition rejects unknown endpoints", () => {
    const next = addTransition(seed(), "ghost", "running", "TRUE")
    expect(next).toEqual(seed())
  })

  it("removeTransition by index", () => {
    const next = removeTransition(seed(), 0)
    expect(next.transitions).toEqual([
      { from: "running", to: "idle", condition: "NOT start" },
    ])
  })

  it("updateTransition patches one field", () => {
    const next = updateTransition(seed(), 0, { condition: "manual_start" })
    expect(next.transitions[0]).toEqual({
      from: "idle",
      to: "running",
      condition: "manual_start",
    })
  })

  it("moveTransition reorders (priority change)", () => {
    const next = moveTransition(seed(), 0, 1)
    expect(next.transitions.map((t) => t.from)).toEqual(["running", "idle"])
  })
})
