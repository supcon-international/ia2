import type { ProgramInfo } from "@/types/generated/ProgramInfo"
import type { RunResponse } from "@/types/generated/RunResponse"

const API_BASE = "http://localhost:3001"

export async function fetchProgram(): Promise<ProgramInfo> {
  const res = await fetch(`${API_BASE}/api/program`)
  if (!res.ok) throw new Error(`GET /api/program → ${res.status}`)
  return res.json()
}

export async function runProgram(): Promise<RunResponse> {
  const res = await fetch(`${API_BASE}/api/run`, { method: "POST" })
  if (!res.ok) {
    const text = await res.text().catch(() => "")
    throw new Error(`POST /api/run → ${res.status} ${text}`)
  }
  return res.json()
}

export async function stopProgram(): Promise<RunResponse> {
  const res = await fetch(`${API_BASE}/api/stop`, { method: "POST" })
  if (!res.ok) throw new Error(`POST /api/stop → ${res.status}`)
  return res.json()
}

export function eventsUrl(): string {
  return `${API_BASE}/api/events`
}
