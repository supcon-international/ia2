import type { Application } from "@/types/generated/Application"
import type { ApplicationKind } from "@/types/generated/ApplicationKind"
import type { CheckDiagnostic } from "@/types/generated/CheckDiagnostic"
import type { Device } from "@/types/generated/Device"
import type { IoMap } from "@/types/generated/IoMap"
import type { ProjectInfo } from "@/types/generated/ProjectInfo"
import type { ProjectListing } from "@/types/generated/ProjectListing"
import type { ProjectTree } from "@/types/generated/ProjectTree"
import type { Protocol } from "@/types/generated/Protocol"
import type { RunResponse } from "@/types/generated/RunResponse"

async function jsonOrThrow<T>(res: Response, label: string): Promise<T> {
  if (!res.ok) {
    const text = await res.text().catch(() => "")
    throw new Error(`${label} → ${res.status} ${text}`.trim())
  }
  return res.json() as Promise<T>
}

// ---------- Project lifecycle ----------

/** Returns null when no project is open (server replies 409). */
export async function fetchProject(): Promise<ProjectTree | null> {
  const res = await fetch(`/api/project`)
  if (res.status === 409) return null
  return jsonOrThrow<ProjectTree>(res, "GET /api/project")
}

export async function fetchProjects(): Promise<ProjectListing[]> {
  return jsonOrThrow(await fetch(`/api/projects`), "GET /api/projects")
}

export async function createProject(name: string): Promise<ProjectInfo> {
  return jsonOrThrow(
    await fetch(`/api/projects`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name }),
    }),
    "POST /api/projects",
  )
}

export async function openProject(path: string): Promise<ProjectInfo> {
  return jsonOrThrow(
    await fetch(`/api/projects/open`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    }),
    "POST /api/projects/open",
  )
}

export async function closeProject(): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/projects/close`, { method: "POST" }),
    "POST /api/projects/close",
  )
}

// ---------- Applications (POUs) ----------

export async function fetchApplication(name: string): Promise<Application> {
  return jsonOrThrow(
    await fetch(`/api/applications/${encodeURIComponent(name)}`),
    `GET /api/applications/${name}`,
  )
}

export async function createApplication(
  name: string,
  kind: ApplicationKind,
): Promise<Application> {
  return jsonOrThrow(
    await fetch(`/api/applications`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name, kind }),
    }),
    "POST /api/applications",
  )
}

export async function saveApplication(
  name: string,
  source: string,
): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/applications/${encodeURIComponent(name)}`, {
      method: "PUT",
      headers: { "Content-Type": "text/plain" },
      body: source,
    }),
    `PUT /api/applications/${name}`,
  )
}

export async function deleteApplication(name: string): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/applications/${encodeURIComponent(name)}`, {
      method: "DELETE",
    }),
    `DELETE /api/applications/${name}`,
  )
}

// ---------- Devices ----------

export async function fetchDevice(name: string): Promise<Device> {
  return jsonOrThrow(
    await fetch(`/api/devices/${encodeURIComponent(name)}`),
    `GET /api/devices/${name}`,
  )
}

export async function createDevice(
  name: string,
  protocol: Protocol,
): Promise<Device> {
  return jsonOrThrow(
    await fetch(`/api/devices`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name, protocol }),
    }),
    "POST /api/devices",
  )
}

export async function updateDevice(
  name: string,
  device: Device,
): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/devices/${encodeURIComponent(name)}`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(device),
    }),
    `PUT /api/devices/${name}`,
  )
}

export async function deleteDevice(name: string): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/devices/${encodeURIComponent(name)}`, {
      method: "DELETE",
    }),
    `DELETE /api/devices/${name}`,
  )
}

// ---------- IO Mapping ----------

export async function fetchIomap(): Promise<IoMap> {
  return jsonOrThrow(await fetch(`/api/iomap`), "GET /api/iomap")
}

export async function updateIomap(iomap: IoMap): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/iomap`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(iomap),
    }),
    "PUT /api/iomap",
  )
}

// ---------- Runtime ----------

export async function checkProgram(source: string): Promise<CheckDiagnostic[]> {
  return jsonOrThrow(
    await fetch(`/api/check`, {
      method: "POST",
      headers: { "Content-Type": "text/plain" },
      body: source,
    }),
    "POST /api/check",
  )
}

export async function runProgram(app?: string): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/run`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(app ? { app } : {}),
    }),
    "POST /api/run",
  )
}

export async function stopProgram(): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/stop`, { method: "POST" }),
    "POST /api/stop",
  )
}

export function eventsUrl(): string {
  return `/api/events`
}
