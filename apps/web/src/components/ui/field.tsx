import * as React from "react"

import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"

/**
 * Label-over-child form field. The canonical version of a wrapper that
 * had drifted into three near-identical copies (DevicePane, EdgePane,
 * TasksPane). `className` lands on the outer wrapper so callers can size
 * the field (e.g. `w-44`) inside a flex/grid row.
 */
export function Field({
  label,
  className,
  children,
}: {
  label: string
  className?: string
  children: React.ReactNode
}) {
  return (
    <div className={cn("space-y-1.5", className)}>
      <Label className="text-[11px] uppercase tracking-wider text-muted-foreground">
        {label}
      </Label>
      {children}
    </div>
  )
}
