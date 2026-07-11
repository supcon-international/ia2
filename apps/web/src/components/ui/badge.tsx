import * as React from "react"

import { cn } from "@/lib/utils"

/**
 * Small monospace uppercase pill — the recurring status-badge markup
 * (`inline-flex … rounded-md px-1.5 py-0.5 font-mono text-[10px] uppercase
 * tracking-wider`). Colour is left to the caller via `className`
 * (`bg-highlight/15 text-highlight`, `bg-destructive/15 text-destructive`,
 * …) so one primitive covers every state.
 */
export function UppercaseBadge({
  className,
  children,
  ...props
}: React.ComponentProps<"span">) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider",
        className,
      )}
      {...props}
    >
      {children}
    </span>
  )
}
