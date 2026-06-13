/**
 * Parse a process-control library block's ST source into a structured
 * "datasheet" — the engineering-facing view of a FUNCTION_BLOCK:
 * interface (pins with type / default / description) + documentation
 * sections, instead of raw code.
 *
 * The library's house comment style (see library/process-control) is
 * regular enough to parse with light regex:
 *
 *   (* FB_X — one-line brief (may wrap to the blank line).
 *
 *      ~ vendor-neutral equivalence line(s).
 *      Algorithm: ...            <- free "Label: body" sections, blank-line separated
 *      Notes: ...
 *
 *      Inputs:
 *        pv      REAL  process value      <- name TYPE description, space-aligned
 *        kp      REAL  proportional gain
 *      Outputs:
 *        out     REAL  control output
 *   *)
 *   FUNCTION_BLOCK FB_X
 *     VAR_INPUT  pv : REAL;  kp : REAL := 1.0;  END_VAR
 *     VAR_OUTPUT out : REAL; END_VAR
 *
 * Pin structure (name / type / default / direction) comes from the
 * VAR_INPUT / VAR_OUTPUT declarations (the source of truth); the
 * per-pin description is merged in from the comment's Inputs:/Outputs:
 * tables by name. Documentation sections are everything else in the
 * header comment. A block whose comment is missing/irregular still
 * yields pins (from the declarations) — the doc just comes out sparse.
 */

export interface DatasheetPin {
  name: string
  /** IEC type name, e.g. REAL / BOOL. */
  type: string
  direction: "input" | "output"
  /** Right-hand side of `:=` in the declaration, when present. */
  default?: string
  /** Prose from the comment's Inputs:/Outputs: table, when present. */
  description?: string
}

export interface DatasheetSection {
  /** Leading `Label:` if the section opened with one (Algorithm, Notes…). */
  label?: string
  body: string
  /** True for the `~ …` vendor-neutral equivalence line. */
  equivalence?: boolean
}

export interface BlockDatasheet {
  /** FB type name, e.g. FB_PID. */
  name: string
  /** One-line description (the part after `FB_X —`). */
  brief: string
  inputs: DatasheetPin[]
  outputs: DatasheetPin[]
  sections: DatasheetSection[]
  /** Raw ST source, for the folded "View source" panel. */
  source: string
}

/** Pull the first `(* … *)` block that precedes `FUNCTION_BLOCK`. */
function headerComment(source: string): string {
  const fbIdx = source.search(/\bFUNCTION_BLOCK\b/)
  const head = fbIdx >= 0 ? source.slice(0, fbIdx) : source
  const m = head.match(/\(\*([\s\S]*?)\*\)/)
  return m ? m[1] : ""
}

/** Strip a uniform leading indent and the comment's own framing. */
function commentLines(comment: string): string[] {
  return comment.replace(/\r/g, "").split("\n").map((l) => l.replace(/\s+$/, ""))
}

/** Parse one `VAR_INPUT`/`VAR_OUTPUT` block body into pins. */
function parseVarBlock(
  source: string,
  keyword: "VAR_INPUT" | "VAR_OUTPUT",
  direction: "input" | "output",
): DatasheetPin[] {
  const re = new RegExp(`\\b${keyword}\\b([\\s\\S]*?)\\bEND_VAR\\b`)
  const block = source.match(re)
  if (!block) return []
  const out: DatasheetPin[] = []
  // `name : TYPE := default;` or `name : TYPE;` — one pin per line, which
  // is the library convention (no `a, b : BOOL` multi-decl in blocks).
  const lineRe = /^\s*([A-Za-z_]\w*)\s*:\s*([A-Za-z_]\w*)\s*(?::=\s*([^;]+?))?\s*;/gm
  let m: RegExpExecArray | null
  while ((m = lineRe.exec(block[1])) !== null) {
    out.push({
      name: m[1],
      type: m[2],
      direction,
      default: m[3]?.trim(),
    })
  }
  return out
}

/** Parse a comment `Inputs:`/`Outputs:` table into name→description. */
function parsePinDocs(lines: string[], startIdx: number, endIdx: number): Map<string, string> {
  const docs = new Map<string, string>()
  // Rows look like:  `    pv          REAL  process value`
  const rowRe = /^\s+([A-Za-z_]\w*)\s+[A-Za-z_]\w*\s+(.+)$/
  for (let i = startIdx + 1; i < endIdx; i++) {
    const m = lines[i].match(rowRe)
    if (m) docs.set(m[1].toLowerCase(), m[2].trim())
  }
  return docs
}

export function parseBlockDatasheet(source: string): BlockDatasheet {
  const nameM = source.match(/\bFUNCTION_BLOCK\s+([A-Za-z_]\w*)/)
  const name = nameM ? nameM[1] : "FB"

  const inputs = parseVarBlock(source, "VAR_INPUT", "input")
  const outputs = parseVarBlock(source, "VAR_OUTPUT", "output")

  const lines = commentLines(headerComment(source))

  // Locate the Inputs:/Outputs: tables so we can (a) lift per-pin docs
  // and (b) exclude those lines from the prose sections.
  const inIdx = lines.findIndex((l) => /^\s*Inputs:\s*$/.test(l))
  const outIdx = lines.findIndex((l) => /^\s*Outputs:\s*$/.test(l))
  const tableStart = inIdx >= 0 ? inIdx : outIdx >= 0 ? outIdx : lines.length

  if (inIdx >= 0) {
    const end = outIdx >= 0 ? outIdx : lines.length
    const docs = parsePinDocs(lines, inIdx, end)
    for (const p of inputs) p.description = docs.get(p.name.toLowerCase())
  }
  if (outIdx >= 0) {
    const docs = parsePinDocs(lines, outIdx, lines.length)
    for (const p of outputs) p.description = docs.get(p.name.toLowerCase())
  }

  // Prose = everything above the pin tables. First paragraph is the
  // brief (after `FB_X —`); the rest are blank-line-separated sections.
  const prose = lines.slice(0, tableStart)
  const paragraphs: string[] = []
  let cur: string[] = []
  const flush = () => {
    if (cur.length) {
      paragraphs.push(cur.join("\n").trim())
      cur = []
    }
  }
  // A section starts at a blank line, a `Label:` line, or a `~`
  // equivalence line. The library mixes blank-separated and
  // back-to-back labelled sections (fb_lag runs Purpose / Algorithm /
  // Notes with no blanks between), so blank lines alone don't delimit
  // them. `:=` excludes assignment lines in Algorithm bodies.
  const startsSection = (l: string) =>
    l.startsWith("~") ||
    (/^[A-Za-z][\w ()/+,.-]{0,80}:(\s|$)/.test(l) && !l.includes(":="))
  for (const raw of prose) {
    const t = raw.trim()
    if (t === "") {
      flush()
      continue
    }
    if (startsSection(t)) flush()
    cur.push(t)
  }
  flush()

  let brief = ""
  const sections: DatasheetSection[] = []
  paragraphs.forEach((p, i) => {
    if (i === 0) {
      // `FB_X — brief…` → keep only the brief part.
      const dash = p.indexOf("—")
      brief = (dash >= 0 ? p.slice(dash + 1) : p).replace(/\s+/g, " ").trim()
      return
    }
    if (p.startsWith("~")) {
      sections.push({ body: p.replace(/^~\s*/, "").trim(), equivalence: true })
      return
    }
    const labelM = p.match(/^([A-Za-z][\w ()/+,.-]*?):\s*([\s\S]*)$/)
    if (labelM && labelM[1].length <= 80) {
      sections.push({ label: labelM[1].trim(), body: labelM[2].trim() })
    } else {
      sections.push({ body: p })
    }
  })

  return { name, brief, inputs, outputs, sections, source }
}
