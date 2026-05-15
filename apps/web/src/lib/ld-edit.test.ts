import { describe, expect, it } from "vitest"

import type { LdNode } from "@/types/generated/LdNode"
import type { LdProgram } from "@/types/generated/LdProgram"
import {
  addCoil,
  addInParallel,
  addInSeries,
  addRung,
  addVariable,
  deleteCoil,
  deleteNode,
  deleteRung,
  getNode,
  moveRung,
  newContact,
  parseProgram,
  removeVariable,
  serializeProgram,
  setCoilKind,
  setContactVar,
  toggleNegated,
  updateVariable,
} from "./ld-edit"

/** Minimal program with one rung whose logic is a single contact
 *  driving one coil. Used as the starting point for most tests. */
function seed(): LdProgram {
  return {
    name: "p",
    pou_type: "program",
    variables: [
      { name: "a", type: "BOOL", section: "input", init: null },
      { name: "out", type: "BOOL", section: "output", init: null },
    ],
    rungs: [
      {
        id: "r0",
        label: null,
        logic: { op: "contact", var: "a", negated: false },
        coils: [{ var: "out", kind: "standard" }],
      },
    ],
  }
}

describe("getNode", () => {
  it("returns root for empty path", () => {
    const p = seed()
    expect(getNode(p.rungs[0].logic, [])).toEqual({
      op: "contact",
      var: "a",
      negated: false,
    })
  })

  it("descends into AND args", () => {
    const root: LdNode = {
      op: "and",
      args: [
        { op: "contact", var: "a", negated: false },
        { op: "contact", var: "b", negated: true },
      ],
    }
    expect(getNode(root, [1])).toMatchObject({ var: "b", negated: true })
  })

  it("throws on out-of-range step", () => {
    const root: LdNode = { op: "and", args: [{ op: "contact", var: "a", negated: false }] }
    expect(() => getNode(root, [5])).toThrow(/out of range/)
  })

  it("descends into NOT's only child", () => {
    const root: LdNode = {
      op: "not",
      arg: { op: "contact", var: "a", negated: false },
    }
    expect(getNode(root, [0])).toMatchObject({ var: "a" })
  })

  it("rejects non-zero step on NOT", () => {
    const root: LdNode = {
      op: "not",
      arg: { op: "contact", var: "a", negated: false },
    }
    expect(() => getNode(root, [1])).toThrow(/only child 0/)
  })
})

describe("addInSeries", () => {
  it("wraps a leaf root in AND", () => {
    const out = addInSeries(seed(), 0, [], "after", newContact("b"))
    expect(out.rungs[0].logic).toEqual({
      op: "and",
      args: [
        { op: "contact", var: "a", negated: false },
        { op: "contact", var: "b", negated: false },
      ],
    })
  })

  it("appends to an existing AND parent (no extra wrapping)", () => {
    let p = seed()
    p = addInSeries(p, 0, [], "after", newContact("b"))
    // Now root is AND(a, b). Add `c` after `b` (path = [1]).
    p = addInSeries(p, 0, [1], "after", newContact("c"))
    const logic = p.rungs[0].logic
    expect(logic.op).toBe("and")
    if (logic.op !== "and") return
    expect(logic.args.map((n) => (n.op === "contact" ? n.var : "?"))).toEqual([
      "a",
      "b",
      "c",
    ])
  })

  it("inserts on the left when side='before'", () => {
    let p = seed()
    p = addInSeries(p, 0, [], "after", newContact("b"))
    p = addInSeries(p, 0, [0], "before", newContact("z"))
    const logic = p.rungs[0].logic
    if (logic.op !== "and") throw new Error("expected and")
    expect(logic.args.map((n) => (n.op === "contact" ? n.var : "?"))).toEqual([
      "z",
      "a",
      "b",
    ])
  })
})

describe("addInParallel", () => {
  it("wraps a leaf root in OR", () => {
    const out = addInParallel(seed(), 0, [], "after", newContact("b"))
    expect(out.rungs[0].logic).toEqual({
      op: "or",
      args: [
        { op: "contact", var: "a", negated: false },
        { op: "contact", var: "b", negated: false },
      ],
    })
  })

  it("appends to existing OR without re-wrapping", () => {
    let p = seed()
    p = addInParallel(p, 0, [], "after", newContact("b"))
    p = addInParallel(p, 0, [], "after", newContact("c"))
    const logic = p.rungs[0].logic
    if (logic.op !== "or") throw new Error("expected or")
    expect(logic.args).toHaveLength(3)
  })

  it("turns a nested branch into AND(OR(...))", () => {
    let p = seed()
    p = addInSeries(p, 0, [], "after", newContact("b")) // AND(a, b)
    p = addInParallel(p, 0, [0], "after", newContact("a2")) // AND(OR(a, a2), b)
    const logic = p.rungs[0].logic
    if (logic.op !== "and") throw new Error("expected and root")
    const first = logic.args[0]
    expect(first).toMatchObject({ op: "or" })
  })
})

describe("deleteNode", () => {
  it("collapses singleton-arg parent after deletion", () => {
    let p = seed()
    p = addInSeries(p, 0, [], "after", newContact("b"))
    // AND(a, b). Delete `b` (path [1]). Should collapse back to just `a`.
    p = deleteNode(p, 0, [1])
    expect(p.rungs[0].logic).toEqual({
      op: "contact",
      var: "a",
      negated: false,
    })
  })

  it("replaces the whole logic with const true when root is deleted", () => {
    const out = deleteNode(seed(), 0, [])
    expect(out.rungs[0].logic).toEqual({ op: "const", value: true })
  })

  it("collapses NOT to const true when its child is deleted", () => {
    const p: LdProgram = {
      ...seed(),
      rungs: [
        {
          id: "r",
          label: null,
          logic: { op: "not", arg: { op: "contact", var: "a", negated: false } },
          coils: [{ var: "out", kind: "standard" }],
        },
      ],
    }
    const out = deleteNode(p, 0, [0])
    expect(out.rungs[0].logic).toEqual({ op: "const", value: true })
  })
})

describe("toggleNegated", () => {
  it("flips the negated flag on a contact", () => {
    const out = toggleNegated(seed(), 0, [])
    expect((out.rungs[0].logic as Extract<LdNode, { op: "contact" }>).negated).toBe(true)
    const back = toggleNegated(out, 0, [])
    expect((back.rungs[0].logic as Extract<LdNode, { op: "contact" }>).negated).toBe(false)
  })

  it("is a no-op on non-contact nodes", () => {
    let p = seed()
    p = addInSeries(p, 0, [], "after", newContact("b"))
    const out = toggleNegated(p, 0, [])
    expect(out).toEqual(p)
  })
})

describe("setContactVar", () => {
  it("renames the variable a contact references", () => {
    const out = setContactVar(seed(), 0, [], "renamed")
    expect((out.rungs[0].logic as Extract<LdNode, { op: "contact" }>).var).toBe(
      "renamed",
    )
  })
})

describe("rung-level ops", () => {
  it("addRung appends with auto-generated id", () => {
    const out = addRung(seed())
    expect(out.rungs).toHaveLength(2)
    expect(out.rungs[1].id).toBe("r1")
  })

  it("addRung at index 0 prepends", () => {
    const out = addRung(seed(), 0)
    expect(out.rungs[0].id).toBe("r1")
    expect(out.rungs[1].id).toBe("r0")
  })

  it("deleteRung removes the targeted rung", () => {
    let p = addRung(seed())
    p = deleteRung(p, 0)
    expect(p.rungs).toHaveLength(1)
    expect(p.rungs[0].id).toBe("r1")
  })

  it("moveRung swaps positions", () => {
    const p = addRung(seed()) // r0, r1
    const out = moveRung(p, 0, 1) // r1, r0
    expect(out.rungs.map((r) => r.id)).toEqual(["r1", "r0"])
  })
})

describe("coil ops", () => {
  it("addCoil appends a standard coil", () => {
    const out = addCoil(seed(), 0, "second_out")
    expect(out.rungs[0].coils).toHaveLength(2)
    expect(out.rungs[0].coils[1]).toEqual({ var: "second_out", kind: "standard" })
  })

  it("deleteCoil removes by index", () => {
    let p = addCoil(seed(), 0, "second_out")
    p = deleteCoil(p, 0, 0)
    expect(p.rungs[0].coils).toEqual([{ var: "second_out", kind: "standard" }])
  })

  it("setCoilKind changes the latch type", () => {
    const out = setCoilKind(seed(), 0, 0, "set")
    expect(out.rungs[0].coils[0].kind).toBe("set")
  })
})

describe("variable ops", () => {
  it("addVariable refuses duplicates", () => {
    const out = addVariable(seed(), {
      name: "a",
      type: "BOOL",
      section: "internal",
      init: null,
    })
    expect(out.variables).toHaveLength(2) // unchanged
  })

  it("removeVariable drops by name", () => {
    const out = removeVariable(seed(), "a")
    expect(out.variables).toHaveLength(1)
    expect(out.variables[0].name).toBe("out")
  })

  it("updateVariable patches in place", () => {
    const out = updateVariable(seed(), "a", { init: "FALSE" })
    expect(out.variables.find((v) => v.name === "a")?.init).toBe("FALSE")
  })
})

describe("round-trip", () => {
  it("parseProgram(serializeProgram(p)) === p", () => {
    const p = seed()
    const back = parseProgram(serializeProgram(p))
    expect(back).toEqual(p)
  })
})

// =================================================================
//   evaluateNode — online-mode evaluator
// =================================================================
import { evaluateNode } from "./ld-edit"

describe("evaluateNode", () => {
  it("contact conducts when var is true and not negated", () => {
    expect(
      evaluateNode({ op: "contact", var: "a", negated: false }, { a: true }),
    ).toBe(true)
    expect(
      evaluateNode({ op: "contact", var: "a", negated: false }, { a: false }),
    ).toBe(false)
  })

  it("negated contact inverts", () => {
    expect(
      evaluateNode({ op: "contact", var: "a", negated: true }, { a: true }),
    ).toBe(false)
    expect(
      evaluateNode({ op: "contact", var: "a", negated: true }, { a: false }),
    ).toBe(true)
  })

  it("missing variable reads as false", () => {
    expect(
      evaluateNode({ op: "contact", var: "missing", negated: false }, {}),
    ).toBe(false)
    expect(
      evaluateNode({ op: "contact", var: "missing", negated: true }, {}),
    ).toBe(true)
  })

  it("AND requires all children", () => {
    const tree: LdNode = {
      op: "and",
      args: [
        { op: "contact", var: "a", negated: false },
        { op: "contact", var: "b", negated: false },
      ],
    }
    expect(evaluateNode(tree, { a: true, b: true })).toBe(true)
    expect(evaluateNode(tree, { a: true, b: false })).toBe(false)
    expect(evaluateNode(tree, { a: false, b: true })).toBe(false)
  })

  it("OR fires when any child fires", () => {
    const tree: LdNode = {
      op: "or",
      args: [
        { op: "contact", var: "a", negated: false },
        { op: "contact", var: "b", negated: false },
      ],
    }
    expect(evaluateNode(tree, { a: false, b: false })).toBe(false)
    expect(evaluateNode(tree, { a: true, b: false })).toBe(true)
    expect(evaluateNode(tree, { a: false, b: true })).toBe(true)
  })

  it("evaluates the seal-in pattern correctly", () => {
    // network: start OR (motor_run AND NOT stop)
    const tree: LdNode = {
      op: "or",
      args: [
        { op: "contact", var: "start", negated: false },
        {
          op: "and",
          args: [
            { op: "contact", var: "motor_run", negated: false },
            { op: "contact", var: "stop", negated: true },
          ],
        },
      ],
    }
    // not yet started
    expect(evaluateNode(tree, {})).toBe(false)
    // press start
    expect(evaluateNode(tree, { start: true })).toBe(true)
    // start released, motor running, stop not pressed -> sealed in
    expect(evaluateNode(tree, { motor_run: true })).toBe(true)
    // stop pressed releases
    expect(evaluateNode(tree, { motor_run: true, stop: true })).toBe(false)
  })

  it("empty AND/OR collapse to identity", () => {
    expect(evaluateNode({ op: "and", args: [] }, {})).toBe(true)
    expect(evaluateNode({ op: "or", args: [] }, {})).toBe(false)
  })

  it("treats omitted `negated` field as false (serde-default round-trip)", () => {
    // Real .ld.json files often omit `negated: false`, relying on
    // serde's #[default]. On the TS side this comes through as
    // undefined, and a naïve `var !== undefined` would always be
    // true. Regression test for that exact bug.
    const node = { op: "contact", var: "x" } as unknown as LdNode
    expect(evaluateNode(node, { x: false })).toBe(false)
    expect(evaluateNode(node, { x: true })).toBe(true)
    // explicit-false should match the omitted case
    expect(
      evaluateNode(
        { op: "contact", var: "x", negated: false } as LdNode,
        { x: false },
      ),
    ).toBe(false)
  })
})
