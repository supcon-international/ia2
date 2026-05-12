import type { Application } from "@/types/generated/Application"
import type { ApplicationKind } from "@/types/generated/ApplicationKind"
import type { AttachInfo } from "@/types/generated/AttachInfo"
import type { AttachmentStatus } from "@/types/generated/AttachmentStatus"
import type { CheckDiagnostic } from "@/types/generated/CheckDiagnostic"
import type { DemoSlaveSnapshot } from "@/types/generated/DemoSlaveSnapshot"
import type { DeployReport } from "@/types/generated/DeployReport"
import type { Device } from "@/types/generated/Device"
import type { Edge } from "@/types/generated/Edge"
import type { EdgeProbe } from "@/types/generated/EdgeProbe"
import type { IoMap } from "@/types/generated/IoMap"
import type { ProjectInfo } from "@/types/generated/ProjectInfo"
import type { MigrationResponse } from "@/types/generated/MigrationResponse"
import type { ProjectListing } from "@/types/generated/ProjectListing"
import type { ProjectPous } from "@/types/generated/ProjectPous"
import type { ProjectTree } from "@/types/generated/ProjectTree"
import type { Protocol } from "@/types/generated/Protocol"
import type { RunResponse } from "@/types/generated/RunResponse"
import type { Tasks } from "@/types/generated/Tasks"
import type { VariableInfo } from "@/types/generated/VariableInfo"

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

export async function createApplicationFolder(
  path: string,
): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/applications/folders`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    }),
    "POST /api/applications/folders",
  )
}

export async function fetchApplicationVariables(
  name: string,
): Promise<VariableInfo[]> {
  return jsonOrThrow(
    await fetch(`/api/applications/${encodeURIComponent(name)}/variables`),
    `GET /api/applications/${name}/variables`,
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

export async function createDeviceFolder(path: string): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/devices/folders`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    }),
    "POST /api/devices/folders",
  )
}

// ---------- Tasks (project-level scheduling) ----------

export async function fetchTasks(): Promise<Tasks> {
  return jsonOrThrow(await fetch(`/api/tasks`), "GET /api/tasks")
}

export async function updateTasks(tasks: Tasks): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/tasks`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(tasks),
    }),
    "PUT /api/tasks",
  )
}

export async function migrateTasks(): Promise<MigrationResponse> {
  return jsonOrThrow(
    await fetch(`/api/project/migrate-tasks`, { method: "POST" }),
    "POST /api/project/migrate-tasks",
  )
}

export async function fetchProjectPous(): Promise<ProjectPous> {
  return jsonOrThrow(
    await fetch(`/api/project/pous`),
    "GET /api/project/pous",
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

export async function runProgram(): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/run`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{}",
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

export async function fetchDemoSlaveSnapshot(): Promise<DemoSlaveSnapshot> {
  return jsonOrThrow(await fetch(`/api/_demo/slave`), "GET /api/_demo/slave")
}

// ---------- Edges ----------

export async function fetchEdge(name: string): Promise<Edge> {
  return jsonOrThrow(
    await fetch(`/api/edges/${encodeURIComponent(name)}`),
    `GET /api/edges/${name}`,
  )
}

export async function createEdge(name: string, host: string): Promise<Edge> {
  return jsonOrThrow(
    await fetch(`/api/edges`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name, host }),
    }),
    "POST /api/edges",
  )
}

export async function updateEdge(name: string, edge: Edge): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/edges/${encodeURIComponent(name)}`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(edge),
    }),
    `PUT /api/edges/${name}`,
  )
}

export async function deleteEdge(name: string): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/edges/${encodeURIComponent(name)}`, { method: "DELETE" }),
    `DELETE /api/edges/${name}`,
  )
}

export async function createEdgeFolder(path: string): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/edges/folders`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    }),
    "POST /api/edges/folders",
  )
}

export async function probeEdge(name: string): Promise<EdgeProbe> {
  return jsonOrThrow(
    await fetch(`/api/edges/${encodeURIComponent(name)}/probe`),
    `GET /api/edges/${name}/probe`,
  )
}

export async function deployEdge(name: string): Promise<DeployReport> {
  return jsonOrThrow(
    await fetch(`/api/edges/${encodeURIComponent(name)}/deploy`, {
      method: "POST",
    }),
    `POST /api/edges/${name}/deploy`,
  )
}

export async function attachEdge(name: string): Promise<AttachInfo> {
  return jsonOrThrow(
    await fetch(`/api/edges/${encodeURIComponent(name)}/attach`, {
      method: "POST",
    }),
    `POST /api/edges/${name}/attach`,
  )
}

export async function detachEdge(name: string): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/edges/${encodeURIComponent(name)}/detach`, {
      method: "POST",
    }),
    `POST /api/edges/${name}/detach`,
  )
}

export async function fetchAttachment(name: string): Promise<AttachmentStatus> {
  return jsonOrThrow(
    await fetch(`/api/edges/${encodeURIComponent(name)}/attachment`),
    `GET /api/edges/${name}/attachment`,
  )
}
