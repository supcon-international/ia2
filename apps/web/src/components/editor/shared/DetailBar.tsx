/**
 * Canonical detail-bar primitives shared by the LD and SFC editors.
 *
 * Both editors had defined `DetailLabel` / `Separator` / `ActionBtn`
 * under the same names but with drifted rendering. This is the single
 * reconciled set; the look follows the SFC/FBD family, which was the
 * majority:
 *
 *   - `DetailLabel` is the rounded muted pill (SFC's, and FBD's inline
 *     `block bN` label). LD's was a bare uppercase label.
 *   - `Separator` is the vertical hairline (SFC's and FBD's). LD's was
 *     a `·` dot.
 *   - `ActionBtn` is the bordered h-7 button (SFC's, matching FBD's
 *     inline detail buttons). LD's was a compact uppercase text button.
 *     It gains a `destructive` variant — styled like SFC's `DangerBtn`
 *     and FBD's inline delete — so LD's delete action keeps its look.
 */

import { cn } from "@/lib/utils"

export function DetailLabel({ children }: { children: React.ReactNode }) {
  return (
    <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] uppercase text-muted-foreground">
      {children}
    </span>
  )
}

export function Separator() {
  return <span className="mx-1 h-4 w-px bg-border" />
}

export function ActionBtn({
  onClick,
  title,
  disabled = false,
  destructive = false,
  children,
}: {
  onClick: () => void
  title?: string
  disabled?: boolean
  destructive?: boolean
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={cn(
        "flex h-7 items-center gap-1 rounded border px-2 text-[11px] disabled:cursor-not-allowed disabled:opacity-50",
        destructive
          ? "border-destructive/40 bg-destructive/5 text-destructive hover:bg-destructive/15"
          : "border-input hover:bg-accent/30",
      )}
    >
      {children}
    </button>
  )
}
