/**
 * Online-mode power colouring for the LD and FBD SVG canvases.
 *
 * Both editors colour wires and glyphs by whether they carry power in
 * the running program. The mapping was an identical helper in each; it
 * lives here once.
 */

/** Tailwind `stroke-*` class for a power state:
 *    null  → not running, static foreground stroke
 *    true  → energised, highlight stroke
 *    false → de-energised, dimmed stroke
 *  FBD flips `stroke-` to `fill-` for its pin dots via string replace. */
export function powerClass(powered: boolean | null): string {
  if (powered === null) return "stroke-foreground"
  return powered ? "stroke-highlight" : "stroke-muted-foreground/40"
}
