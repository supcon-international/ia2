/**
 * Which node types may carry actions. Mirrors `hosts_actions` in
 * crates/project/src/hmi.rs (validate_hmi flags the rest as errors) —
 * the canvas must not fire gestures the validator forbids, or a write
 * could hide behind an inert-looking label.
 */

import type { HmiNode } from "@/types/generated/HmiNode"

export const ACTION_HOST_TYPES: ReadonlySet<HmiNode["type"]> = new Set([
  "button",
  "input",
  "symbol",
  "nav",
])

export function canHostAction(type: HmiNode["type"]): boolean {
  return ACTION_HOST_TYPES.has(type)
}
