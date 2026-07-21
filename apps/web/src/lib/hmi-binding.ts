/**
 * Pure binding-resolution helpers for the HMI canvas.
 *
 * A binding names a program variable; the canvas resolves it against the
 * live VarSnapshot with the Monitor's rules (exact name first, then the
 * bare tail of `instance.variable`), optionally pipes it through a tiny
 * single-variable expression, then formats it for display.
 *
 * The expression language is deliberately not-Turing: one input `x`,
 * numeric literals, + - * / % parentheses, comparisons and ternary. No
 * identifiers besides `x`, no calls, no assignment — cross-variable logic
 * belongs in a POU, and nothing here can loop or allocate.
 */

import type { HmiBinding } from "@/types/generated/HmiBinding"
import type { VarSnapshot } from "@/types/generated/VarSnapshot"

export function bindingVariable(b: HmiBinding): string {
  return typeof b === "string" ? b : b.variable
}

/** Resolve a variable name against the snapshot: exact match, else match
 *  on the tail after the last `.` (multi-PROGRAM `instance.variable`). */
export function lookupVar(
  snapshot: VarSnapshot | null,
  name: string,
): { raw: string; type_name: string } | null {
  if (!snapshot) return null
  const exact = snapshot.vars.find((v) => v.name === name)
  if (exact) return { raw: exact.value, type_name: exact.type_name }
  const tail = name.split(".").pop() ?? name
  const byTail = snapshot.vars.find(
    (v) => (v.name.split(".").pop() ?? v.name) === tail,
  )
  return byTail ? { raw: byTail.value, type_name: byTail.type_name } : null
}

/** Snapshot values arrive as display strings ("TRUE", "42", "3.14").
 *  Collapse to a number: booleans become 0/1, unparseable becomes NaN. */
export function toNumber(raw: string): number {
  const t = raw.trim()
  if (/^true$/i.test(t)) return 1
  if (/^false$/i.test(t)) return 0
  const n = Number(t)
  return Number.isFinite(n) ? n : NaN
}

/** Resolve a full binding to its numeric value (post-expr). Non-finite
 *  results — a non-numeric variable, a divide-by-zero expr — collapse
 *  to null so consumers (symbols, readouts) fall back to their
 *  unresolved state instead of propagating NaN/Infinity. */
export function resolveBinding(
  snapshot: VarSnapshot | null,
  b: HmiBinding,
): number | null {
  const found = lookupVar(snapshot, bindingVariable(b))
  if (!found) return null
  let value = toNumber(found.raw)
  if (typeof b !== "string" && b.expr) {
    const out = evalExpr(b.expr, value)
    if (out === null) return null
    value = out
  }
  return Number.isFinite(value) ? value : null
}

/** Resolve a binding for the value/input readout — the contract is a
 *  numeric/boolean/STRING display, so unlike the numeric-only
 *  `resolveBinding` this surfaces non-numeric variables as text:
 *  STRING values show their content (quotes stripped), hex bit-field
 *  literals (`16#1637`) pass through verbatim, BOOLs read TRUE/FALSE.
 *  Exprs stay numeric; non-finite results stay null (em-dash). */
export function resolveDisplay(
  snapshot: VarSnapshot | null,
  b: HmiBinding,
): number | string | null {
  if (typeof b !== "string" && b.expr) return resolveBinding(snapshot, b)
  const found = lookupVar(snapshot, bindingVariable(b))
  if (!found) return null
  const raw = found.raw.trim()
  if (found.type_name.toUpperCase() === "BOOL") return raw.toUpperCase()
  const n = toNumber(raw)
  if (Number.isFinite(n)) return n
  const quoted = /^'(.*)'$/.exec(raw)
  return quoted ? quoted[1] : raw
}

/** Format per the binding's printf-ish `format` (%.2f, %d, %s), falling
 *  back to a compact default. */
export function formatBinding(b: HmiBinding, value: number): string {
  // Never print "NaN"/"Infinity" on an operator screen.
  if (!Number.isFinite(value)) return "—"
  const fmt = typeof b === "string" ? null : (b.format ?? null)
  if (fmt) {
    const m = /^%(?:\.(\d+))?([dfs])$/.exec(fmt)
    if (m) {
      const [, prec, kind] = m
      if (kind === "d") return String(Math.round(value))
      if (kind === "f") return value.toFixed(prec ? Number(prec) : 2)
      return String(value)
    }
  }
  if (Number.isInteger(value)) return String(value)
  return value.toFixed(2)
}

// ============================================================
//  Expression evaluator — recursive descent, no eval()
// ============================================================

type Tok =
  | { k: "num"; v: number }
  | { k: "x" }
  | { k: "op"; v: string }

function lex(src: string): Tok[] | null {
  const out: Tok[] = []
  let i = 0
  const ops = ["<=", ">=", "==", "!=", "&&", "||", "<", ">", "+", "-", "*", "/", "%", "(", ")", "?", ":", "!"]
  while (i < src.length) {
    const c = src[i]
    if (c === " " || c === "\t") {
      i++
      continue
    }
    if (/[0-9.]/.test(c)) {
      let j = i
      while (j < src.length && /[0-9.]/.test(src[j])) j++
      const n = Number(src.slice(i, j))
      if (!Number.isFinite(n)) return null
      out.push({ k: "num", v: n })
      i = j
      continue
    }
    if (c === "x" && !/[a-zA-Z0-9_]/.test(src[i + 1] ?? "")) {
      out.push({ k: "x" })
      i++
      continue
    }
    const op = ops.find((o) => src.startsWith(o, i))
    if (!op) return null
    out.push({ k: "op", v: op })
    i += op.length
  }
  return out
}

/** Evaluate `expr` with `x` bound. Returns null on any parse error —
 *  callers render an em-dash rather than a wrong number. */
export function evalExpr(expr: string, x: number): number | null {
  const toks = lex(expr)
  if (!toks) return null
  let pos = 0
  const peek = () => toks[pos]
  const eat = (v?: string): Tok | null => {
    const t = toks[pos]
    if (!t) return null
    if (v !== undefined && !(t.k === "op" && t.v === v)) return null
    pos++
    return t
  }

  // ternary → or → and → cmp → add → mul → unary → atom
  function ternary(): number | null {
    const cond = or()
    if (cond === null) return null
    if (peek()?.k === "op" && (peek() as { v: string }).v === "?") {
      eat("?")
      const a = ternary()
      if (a === null || !eat(":")) return null
      const b = ternary()
      if (b === null) return null
      return cond !== 0 ? a : b
    }
    return cond
  }
  function or(): number | null {
    let l = and()
    if (l === null) return null
    while (peek()?.k === "op" && (peek() as { v: string }).v === "||") {
      eat("||")
      const r = and()
      if (r === null) return null
      l = l !== 0 || r !== 0 ? 1 : 0
    }
    return l
  }
  function and(): number | null {
    let l = cmp()
    if (l === null) return null
    while (peek()?.k === "op" && (peek() as { v: string }).v === "&&") {
      eat("&&")
      const r = cmp()
      if (r === null) return null
      l = l !== 0 && r !== 0 ? 1 : 0
    }
    return l
  }
  function cmp(): number | null {
    let l = add()
    if (l === null) return null
    for (;;) {
      const t = peek()
      if (t?.k !== "op" || !["<", ">", "<=", ">=", "==", "!="].includes(t.v)) return l
      eat(t.v)
      const r = add()
      if (r === null) return null
      const res: boolean =
        t.v === "<" ? l < r
        : t.v === ">" ? l > r
        : t.v === "<=" ? l <= r
        : t.v === ">=" ? l >= r
        : t.v === "==" ? l === r
        : l !== r
      l = res ? 1 : 0
    }
  }
  function add(): number | null {
    let l = mul()
    if (l === null) return null
    for (;;) {
      const t = peek()
      if (t?.k !== "op" || (t.v !== "+" && t.v !== "-")) return l
      eat(t.v)
      const r = mul()
      if (r === null) return null
      l = t.v === "+" ? l + r : l - r
    }
  }
  function mul(): number | null {
    let l = unary()
    if (l === null) return null
    for (;;) {
      const t = peek()
      if (t?.k !== "op" || !["*", "/", "%"].includes(t.v)) return l
      eat(t.v)
      const r = unary()
      if (r === null) return null
      l = t.v === "*" ? l * r : t.v === "/" ? l / r : l % r
    }
  }
  function unary(): number | null {
    const t = peek()
    if (t?.k === "op" && t.v === "-") {
      eat("-")
      const v = unary()
      return v === null ? null : -v
    }
    if (t?.k === "op" && t.v === "!") {
      eat("!")
      const v = unary()
      return v === null ? null : v === 0 ? 1 : 0
    }
    return atom()
  }
  function atom(): number | null {
    const t = peek()
    if (!t) return null
    if (t.k === "num") {
      pos++
      return t.v
    }
    if (t.k === "x") {
      pos++
      return x
    }
    if (t.k === "op" && t.v === "(") {
      eat("(")
      const v = ternary()
      if (v === null || !eat(")")) return null
      return v
    }
    return null
  }

  const result = ternary()
  return pos === toks.length ? result : null
}
