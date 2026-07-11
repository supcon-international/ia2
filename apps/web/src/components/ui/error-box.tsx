import * as React from "react"

import { cn } from "@/lib/utils"

/**
 * Scrollable error/diagnostic box. Renders on the `destructive` token so it
 * tracks the theme rather than hardcoded red literals. Defaults to the
 * compact `p-2 text-[11px]` sizing; pass `className` to override (e.g. a
 * roomier deploy log).
 */
export function ErrorBox({
  children,
  className,
}: {
  children: React.ReactNode
  className?: string
}) {
  return (
    <pre
      className={cn(
        "overflow-auto rounded-md border border-destructive/40 bg-destructive/5 p-2 text-[11px] text-destructive",
        className,
      )}
    >
      {children}
    </pre>
  )
}
