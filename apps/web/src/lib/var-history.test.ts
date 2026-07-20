import { describe, expect, it } from "vitest"

import {
  MAX_TIMED_HISTORY,
  pushTimedHistory,
  windowSlice,
  type TimedSample,
} from "./var-history"

function fill(
  buf: TimedSample[],
  t0: number,
  n: number,
  dt: number,
  windowS: number,
): TimedSample[] {
  for (let i = 0; i < n; i++) pushTimedHistory(buf, t0 + i * dt, i, windowS)
  return buf
}

describe("pushTimedHistory", () => {
  it("retains by age, not by count", () => {
    // 10 Hz for 60 s into a 30 s window: depth follows the window
    // (~301 samples), not a fixed cap — the old 256-sample trim held
    // ~26 s regardless of the contracted window_s.
    const buf = fill([], 1000, 600, 0.1, 30)
    expect(buf[0].t).toBeGreaterThanOrEqual(buf[buf.length - 1].t - 30)
    expect(buf.length).toBeGreaterThan(256)
    expect(buf.length).toBeLessThanOrEqual(302)
  })

  it("holds the full window at a slow snapshot rate", () => {
    // 1 Hz into a 300 s window: all 300 s stay.
    const buf = fill([], 0, 400, 1, 300)
    expect(buf.length).toBe(301)
    expect(buf[0].t).toBe(99)
  })

  it("enforces the hard cap for wild windows", () => {
    const buf = fill([], 0, MAX_TIMED_HISTORY + 500, 0.1, 86400)
    expect(buf.length).toBe(MAX_TIMED_HISTORY)
  })

  it("trims a hidden-tab gap down to what the window covers", () => {
    const buf = fill([], 0, 100, 1, 60)
    pushTimedHistory(buf, 1000, 42, 60)
    expect(buf.length).toBe(1)
    expect(buf[0]).toEqual({ t: 1000, v: 42 })
  })
})

describe("windowSlice", () => {
  it("slices a narrower per-node window off a shared buffer", () => {
    const buf = fill([], 0, 300, 1, 300)
    const view = windowSlice(buf, 60)
    expect(view[view.length - 1]).toEqual(buf[buf.length - 1])
    expect(view[0].t).toBe(buf[buf.length - 1].t - 60)
  })

  it("returns the buffer untouched when the window covers it", () => {
    const buf = fill([], 0, 50, 1, 300)
    expect(windowSlice(buf, 300)).toBe(buf)
    expect(windowSlice([], 300)).toEqual([])
  })
})
