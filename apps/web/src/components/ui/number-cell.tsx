import * as React from "react"

import { Input } from "@/components/ui/input"

/**
 * Numeric `<Input>` that parses and clamps in one place, handing the caller
 * a ready number. Replaces the ~dozen
 * `Math.max(min, Math.min(max, Number(e.target.value) || fallback))`
 * closures in the device editors. Non-numeric input falls back to
 * `fallback` (default 0); `min`/`max` clamp when supplied.
 */
export function NumberCell({
  value,
  onChange,
  min,
  max,
  fallback = 0,
  className,
  ...props
}: {
  value: number
  onChange: (value: number) => void
  min?: number
  max?: number
  fallback?: number
  className?: string
} & Omit<
  React.ComponentProps<"input">,
  "value" | "onChange" | "min" | "max" | "type"
>) {
  return (
    <Input
      type="number"
      min={min}
      max={max}
      value={value}
      onChange={(e) => {
        let n = Number(e.target.value)
        if (!Number.isFinite(n)) n = fallback
        if (min != null) n = Math.max(min, n)
        if (max != null) n = Math.min(max, n)
        onChange(n)
      }}
      className={className}
      {...props}
    />
  )
}
