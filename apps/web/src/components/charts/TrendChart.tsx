type Series = {
  name: string
  values: number[]
  color: string
  binary: boolean
}

type Props = {
  series: Series[]
  height?: number
}

/**
 * Multi-line chart for "pinned" variables — each series autoscales
 * independently between 0..1 so totally different magnitudes coexist
 * cleanly without a real Y axis. The legend on top maps colour → name.
 */
export function TrendChart({ series, height = 110 }: Props) {
  if (series.length === 0) {
    return (
      <div
        className="flex items-center justify-center text-[11px] text-muted-foreground"
        style={{ height }}
      >
        Pin a variable below to chart it.
      </div>
    )
  }

  const width = 700 // viewBox; SVG scales to container
  const padTop = 4
  const padBottom = 4
  const usableH = height - padTop - padBottom

  const renderSeries = (s: Series) => {
    if (s.values.length < 2) return null
    let min: number
    let max: number
    if (s.binary) {
      min = 0
      max = 1
    } else {
      min = s.values[0]
      max = s.values[0]
      for (const v of s.values) {
        if (v < min) min = v
        if (v > max) max = v
      }
      if (max === min) max = min + 1
    }
    const range = max - min
    const n = s.values.length
    const stepX = width / Math.max(1, n - 1)
    const toY = (v: number) =>
      padTop + usableH - ((v - min) / range) * usableH

    const points: string[] = []
    if (s.binary) {
      for (let i = 0; i < n; i++) {
        const x = i * stepX
        const y = toY(s.values[i])
        if (i === 0) {
          points.push(`${x},${y}`)
        } else {
          const prevY = toY(s.values[i - 1])
          points.push(`${x},${prevY}`)
          points.push(`${x},${y}`)
        }
      }
    } else {
      for (let i = 0; i < n; i++) {
        points.push(`${(i * stepX).toFixed(1)},${toY(s.values[i]).toFixed(1)}`)
      }
    }
    return (
      <polyline
        key={s.name}
        points={points.join(" ")}
        fill="none"
        stroke={s.color}
        strokeWidth={1.25}
        strokeLinejoin={s.binary ? "miter" : "round"}
        strokeLinecap="round"
        opacity={0.95}
      />
    )
  }

  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 px-1 text-[10px]">
        {series.map((s) => (
          <div key={s.name} className="flex items-center gap-1.5">
            <span
              className="inline-block size-2 rounded-full"
              style={{ background: s.color }}
            />
            <span className="font-mono text-foreground">{s.name}</span>
          </div>
        ))}
      </div>
      <svg
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
        className="block w-full"
        style={{ height }}
      >
        <line
          x1={0}
          y1={height / 2}
          x2={width}
          y2={height / 2}
          stroke="currentColor"
          strokeOpacity={0.08}
          strokeDasharray="2 4"
        />
        {series.map(renderSeries)}
      </svg>
    </div>
  )
}
