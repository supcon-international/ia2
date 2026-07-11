/**
 * Shared top-of-pane header for the LD / FBD / SFC editors.
 *
 * The three were near-identical: the POU name in mono, a language
 * badge, a PROGRAM/FUNCTION_BLOCK badge, and a one-line element count.
 * They differed only in the language literal and the count noun, so
 * those come in as props (`language`, `summary`). `children` carries
 * any language-specific trailer — e.g. SFC's live "→ activeStep" badge.
 */

import type { LdPouType } from "@/types/generated/LdPouType"

export function EditorHeader({
  name,
  language,
  pouType,
  summary,
  children,
}: {
  name: string
  /** Short language tag rendered in the badge (`ld` / `fbd` / `sfc`). */
  language: string
  pouType: LdPouType
  /** The element-count line, e.g. "3 rungs · 2 vars". */
  summary: React.ReactNode
  /** Optional language-specific trailer (SFC's active-step badge). */
  children?: React.ReactNode
}) {
  return (
    <div className="border-b border-border bg-muted/30 px-3 py-1.5 text-[11px] uppercase tracking-wider text-muted-foreground">
      <span className="font-mono normal-case tracking-normal text-foreground">
        {name}
      </span>
      <span className="ml-2 rounded border border-border bg-muted/50 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
        {language}
      </span>
      <span className="ml-2 rounded border border-border bg-muted/50 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
        {pouType === "function_block" ? "fb" : "prg"}
      </span>
      <span className="ml-3">{summary}</span>
      {children}
    </div>
  )
}
