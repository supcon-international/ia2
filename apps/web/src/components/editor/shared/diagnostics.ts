/**
 * Shared diagnostics bucketing for the LD / FBD / SFC editors.
 *
 * All three editors used to define a near-identical `indexDiagnostics`
 * that split a flat `CheckDiagnostic[]` into two O(1) lookup maps: one
 * keyed by the language's graphical element id (rung id / block id /
 * step name) and one keyed by variable name. Only the location type and
 * the id-extraction differed. This module keeps the bucketing loop once;
 * each editor passes a small `key` classifier for its own `*_location`.
 *
 * `describeLocation` stays local to each editor — its per-kind output
 * strings are genuinely language-specific and share no reusable body.
 */

import type { CheckDiagnostic } from "@/types/generated/CheckDiagnostic"

/** Two O(1) views over a diagnostics list. `byElement` is keyed by the
 *  editor's graphical element id (rung / block / step); `byVariable` by
 *  the declared variable name. */
export interface DiagnosticIndex {
  byElement: Map<string, CheckDiagnostic[]>
  byVariable: Map<string, CheckDiagnostic[]>
}

/** Where a single diagnostic belongs. Return `{ element }` to bucket it
 *  under a graphical element, `{ variable }` for a variable, or `null`
 *  to drop it (no location, or a kind this editor doesn't surface). */
export type DiagnosticKey = { element: string } | { variable: string } | null

/** Bucket diagnostics into element-keyed and variable-keyed maps using
 *  the caller's per-language `key` classifier. */
export function indexDiagnostics(
  diags: CheckDiagnostic[],
  key: (d: CheckDiagnostic) => DiagnosticKey,
): DiagnosticIndex {
  const byElement = new Map<string, CheckDiagnostic[]>()
  const byVariable = new Map<string, CheckDiagnostic[]>()
  for (const d of diags) {
    const k = key(d)
    if (!k) continue
    if ("element" in k) push(byElement, k.element, d)
    else push(byVariable, k.variable, d)
  }
  return { byElement, byVariable }
}

function push(
  map: Map<string, CheckDiagnostic[]>,
  k: string,
  d: CheckDiagnostic,
): void {
  const list = map.get(k) ?? []
  list.push(d)
  map.set(k, list)
}
