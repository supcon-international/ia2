import type { ProgramInfo } from "@/types/generated/ProgramInfo"
import type { RunResponse } from "@/types/generated/RunResponse"

// Paths are relative — vite dev proxies /api and /health to the backend, and
// in production the server can be reverse-proxied behind the same origin.

export async function fetchProgram(): Promise<ProgramInfo> {
  const res = await fetch(`/api/program`)
  if (!res.ok) throw new Error(`GET /api/program → ${res.status}`)
  return res.json()
}

export async function runProgram(source: string): Promise<RunResponse> {
  const res = await fetch(`/api/run`, {
    method: "POST",
    headers: { "Content-Type": "text/plain" },
    body: source,
  })
  if (!res.ok) {
    const text = await res.text().catch(() => "")
    throw new Error(`POST /api/run → ${res.status} ${text}`)
  }
  return res.json()
}

export async function stopProgram(): Promise<RunResponse> {
  const res = await fetch(`/api/stop`, { method: "POST" })
  if (!res.ok) throw new Error(`POST /api/stop → ${res.status}`)
  return res.json()
}

export function eventsUrl(): string {
  return `/api/events`
}
