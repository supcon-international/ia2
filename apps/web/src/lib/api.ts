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
import type { Pou } from "@/types/generated/Pou"
import type { PouLanguage } from "@/types/generated/PouLanguage"
import type { PouType } from "@/types/generated/PouType"
import type { ProjectListing } from "@/types/generated/ProjectListing"
import type { ProjectPous } from "@/types/generated/ProjectPous"
import type { ProjectTree } from "@/types/generated/ProjectTree"
import type { Protocol } from "@/types/generated/Protocol"
import type { RunResponse } from "@/types/generated/RunResponse"
import type { RuntimeStatus } from "@/types/generated/RuntimeStatus"
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

// ---------- POUs (`.st` files holding 1+ IEC declarations) ----------

export async function fetchPou(path: string): Promise<Pou> {
  return jsonOrThrow(
    await fetch(`/api/pous/${encodeURIComponent(path)}`),
    `GET /api/pous/${path}`,
  )
}

export async function createPou(
  path: string,
  type_: PouType,
  language: PouLanguage = "st",
): Promise<Pou> {
  return jsonOrThrow(
    await fetch(`/api/pous`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path, type: type_, language }),
    }),
    "POST /api/pous",
  )
}

export async function savePou(
  path: string,
  source: string,
): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/pous/${encodeURIComponent(path)}`, {
      method: "PUT",
      headers: { "Content-Type": "text/plain" },
      body: source,
    }),
    `PUT /api/pous/${path}`,
  )
}

export async function deletePou(path: string): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/pous/${encodeURIComponent(path)}`, {
      method: "DELETE",
    }),
    `DELETE /api/pous/${path}`,
  )
}

export async function createPouFolder(path: string): Promise<RunResponse> {
  return jsonOrThrow(
    await fetch(`/api/pous/folders`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    }),
    "POST /api/pous/folders",
  )
}

export async function fetchPouVariables(path: string): Promise<VariableInfo[]> {
  return jsonOrThrow(
    await fetch(`/api/pous/${encodeURIComponent(path)}/variables`),
    `GET /api/pous/${path}/variables`,
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

export async function checkProgram(
  source: string,
  language: "st" | "ld" | "fbd" = "st",
): Promise<CheckDiagnostic[]> {
  // ST source is plain text; LD / FBD source is JSON. Different
  // Content-Type plus a `?language=` query so the bridge knows what
  // shape to expect before running ironplc. Without the query the
  // server defaults to ST for back-compat with older clients.
  const url =
    language === "st" ? `/api/check` : `/api/check?language=${language}`
  const contentType =
    language === "st" ? "text/plain" : "application/json"
  return jsonOrThrow(
    await fetch(url, {
      method: "POST",
      headers: { "Content-Type": contentType },
      body: source,
    }),
    "POST /api/check",
  )
}

/**
 * Run a project.
 *
 * - No args → use the project's `tasks.toml` schedule (every PROGRAM
 *   instance declared there). The "production" path.
 * - `(program, file_path)` → ad-hoc one-shot: spawn a synthetic single-
 *   PROGRAM schedule on a default 100 ms task. When `file_path` is also
 *   set, the compile input is limited to that file alone — Monitor only
 *   sees the running PROGRAM's variables (no bleed from other POUs).
 */
export async function runProgram(
  program?: string,
  file_path?: string,
): Promise<RunResponse> {
  const body = program ? (file_path ? { program, file_path } : { program }) : {}
  return jsonOrThrow(
    await fetch(`/api/run`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
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

/** Snapshot of "what is the runtime doing right now" — same struct the
 *  agent docs / GET endpoint surface. The IDE uses this on mount and
 *  after detach to recover `running_info` (the page-reload case where
 *  no SSE `started` event will fire for an already-running program). */
export async function fetchRuntimeStatus(): Promise<RuntimeStatus> {
  return jsonOrThrow(
    await fetch(`/api/runtime/status`),
    "GET /api/runtime/status",
  )
}

/** Write a single variable on the running bridge.
 *
 *  The runtime's write primitive is `write_variable(VarIndex, i32)`,
 *  so the payload is always an i32. This helper handles the mapping
 *  from JS `number` + IEC type name:
 *
 *    - BOOL                                     → 0 or 1
 *    - SINT/INT/DINT/USINT/UINT/UDINT/BYTE/...  → truncated integer
 *    - REAL (32-bit float)                      → IEEE-754 bit pattern
 *      (interpret the bytes of the float as an i32 so the VM sees
 *      the right f32 in the slot)
 *    - LREAL / 64-bit ints                      → not supported via
 *      this endpoint yet; caller should warn or skip.
 *
 *  Throws if the runtime isn't running or the variable doesn't exist.
 */
export async function writeVariable(
  name: string,
  value: number,
  typeName: string = "",
): Promise<void> {
  const i32 = encodeForWrite(value, typeName)
  await jsonOrThrow(
    await fetch(`/api/runtime/variables/${encodeURIComponent(name)}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ value: i32 }),
    }),
    `POST /api/runtime/variables/${name}`,
  )
}

function encodeForWrite(value: number, typeName: string): number {
  const t = typeName.toUpperCase()
  if (t === "REAL") {
    // Pack f32 bits into i32. The bridge VM reads slots untyped so
    // it sees the right f32 when this VarIndex points at a REAL.
    const buf = new ArrayBuffer(4)
    new Float32Array(buf)[0] = value
    return new Int32Array(buf)[0]
  }
  // BOOL, integer family, BYTE/WORD/DWORD all pass through as integers.
  return Math.trunc(value)
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
