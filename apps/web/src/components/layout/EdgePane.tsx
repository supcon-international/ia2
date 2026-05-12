import {
  Activity,
  AlertCircle,
  CheckCircle2,
  Link2,
  Link2Off,
  Loader2,
  Rocket,
  Save,
} from "lucide-react"
import { useEffect, useRef, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { deployEdge, probeEdge } from "@/lib/api"
import { useRuntime } from "@/state/runtime"
import type { DeployReport } from "@/types/generated/DeployReport"
import type { Edge } from "@/types/generated/Edge"
import type { EdgeProbe } from "@/types/generated/EdgeProbe"

export function EdgePane() {
  const {
    currentEdge,
    attached,
    saveEdge,
    attachEdge,
    detachEdge,
  } = useRuntime()

  if (!currentEdge) {
    return (
      <main className="flex h-full min-h-0 min-w-0 flex-col">
        <div className="flex h-9 items-center border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
          Edge
        </div>
        <div className="grid flex-1 place-items-center p-6 text-center text-sm text-muted-foreground">
          Select an edge from the project tree, or create one with the&nbsp;
          <span className="font-mono">+</span> button next to "Edges".
        </div>
      </main>
    )
  }

  return (
    <main className="flex h-full min-h-0 min-w-0 flex-col">
      <Editor
        key={currentEdge.name}
        edge={currentEdge}
        attached={attached?.name === currentEdge.name}
        onSave={saveEdge}
        onAttach={() => attachEdge(currentEdge.name)}
        onDetach={detachEdge}
      />
    </main>
  )
}

function Editor({
  edge,
  attached,
  onSave,
  onAttach,
  onDetach,
}: {
  edge: Edge
  attached: boolean
  onSave: (e: Edge) => Promise<void>
  onAttach: () => Promise<void>
  onDetach: () => Promise<void>
}) {
  const [draft, setDraft] = useState<Edge>(edge)
  const [probe, setProbe] = useState<EdgeProbe | null>(null)
  const [probing, setProbing] = useState(false)
  const [deploying, setDeploying] = useState(false)
  const [deployLog, setDeployLog] = useState<DeployReport | null>(null)
  const [deployError, setDeployError] = useState<string | null>(null)

  useEffect(() => {
    setDraft(edge)
    setProbe(null)
    setDeployLog(null)
    setDeployError(null)
  }, [edge])

  // Auto-probe on entry and every 10s while the pane is open. Cheap (one
  // ssh + curl) and keeps the status badge fresh without user clicks.
  const probeName = edge.name
  const setProbeRef = useRef(setProbe)
  setProbeRef.current = setProbe
  useEffect(() => {
    let cancelled = false
    const tick = async () => {
      if (cancelled) return
      try {
        const result = await probeEdge(probeName)
        if (!cancelled) setProbeRef.current(result)
      } catch {
        /* ignore — backend may have moved between project loads */
      }
    }
    void tick()
    const handle = window.setInterval(tick, 10_000)
    return () => {
      cancelled = true
      window.clearInterval(handle)
    }
  }, [probeName])

  const dirty = JSON.stringify(draft) !== JSON.stringify(edge)
  const update = (patch: Partial<Edge>) => setDraft({ ...draft, ...patch })

  const probeNow = async () => {
    setProbing(true)
    try {
      setProbe(await probeEdge(edge.name))
    } catch (e) {
      setProbe({
        reachable: false,
        scan_count: null,
        uptime_secs: null,
        runtime_version: null,
        error: String(e),
      })
    } finally {
      setProbing(false)
    }
  }

  const deploy = async () => {
    if (!confirm(
      `Deploy this project to ${edge.host}?\n` +
        `The runtime will be restarted; the previous version is kept for rollback.`,
    )) {
      return
    }
    setDeploying(true)
    setDeployError(null)
    setDeployLog(null)
    try {
      const report = await deployEdge(edge.name)
      setDeployLog(report)
      // Re-probe so the user sees the new uptime + scan count climb.
      await probeNow()
    } catch (e) {
      setDeployError(String(e))
    } finally {
      setDeploying(false)
    }
  }

  return (
    <>
      <div className="flex h-9 items-center justify-between border-b border-border pl-3 pr-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span className="flex items-center gap-2 truncate normal-case tracking-normal text-foreground">
          <span className="truncate font-mono">{edge.name}</span>
          <span className="rounded bg-rose-500/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-rose-700 dark:text-rose-400">
            edge
          </span>
          {dirty && (
            <span className="rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-amber-700 dark:text-amber-400">
              modified
            </span>
          )}
        </span>
        <div className="flex items-center gap-2">
          <ReachBadge probe={probe} probing={probing} />
          <Button
            size="sm"
            variant="outline"
            onClick={() => void onSave(draft)}
            disabled={!dirty}
          >
            <Save className="mr-1.5 size-3" />
            Save
          </Button>
        </div>
      </div>

      <div className="flex-1 space-y-6 overflow-auto p-5">
        <section>
          <div className="mb-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
            Connection
          </div>
          <div className="grid max-w-2xl grid-cols-2 gap-3">
            <Field label="SSH host / alias">
              <Input
                value={draft.host}
                onChange={(e) => update({ host: e.target.value })}
                placeholder="line1.lan or production-line-1"
              />
            </Field>
            <Field label="SSH port">
              <Input
                type="number"
                value={draft.ssh_port}
                onChange={(e) =>
                  update({ ssh_port: Number(e.target.value) || 22 })
                }
              />
            </Field>
            <Field label="SSH user (optional)">
              <Input
                value={draft.ssh_user}
                placeholder="(default — from ~/.ssh/config)"
                onChange={(e) => update({ ssh_user: e.target.value })}
              />
            </Field>
            <Field label="Runtime port (loopback on edge)">
              <Input
                type="number"
                value={draft.runtime_port}
                onChange={(e) =>
                  update({ runtime_port: Number(e.target.value) || 13001 })
                }
              />
            </Field>
            <Field label="Install dir on edge">
              <Input
                value={draft.install_dir}
                onChange={(e) => update({ install_dir: e.target.value })}
              />
            </Field>
          </div>
          <p className="mt-3 max-w-2xl text-[11px] text-muted-foreground">
            What runs on this edge is the project's <span className="font-mono">tasks.toml</span>{" "}
            (every PROGRAM instance declared there, on its bound task) — not a
            single POU. Edit the Tasks pane to change the schedule.
          </p>
        </section>

        <section>
          <Label className="text-[11px] uppercase tracking-wider text-muted-foreground">
            Notes
          </Label>
          <textarea
            value={draft.notes}
            onChange={(e) => update({ notes: e.target.value })}
            placeholder="Free-form: production line 1, hardware revision, on-site contacts…"
            rows={3}
            className="mt-1.5 block w-full max-w-2xl resize-y rounded-md border border-border bg-background px-2 py-1.5 text-sm placeholder:text-muted-foreground"
          />
        </section>

        <section>
          <div className="mb-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
            Actions
          </div>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" variant="outline" onClick={probeNow} disabled={probing}>
              {probing ? (
                <Loader2 className="mr-1.5 size-3 animate-spin" />
              ) : (
                <Activity className="mr-1.5 size-3" />
              )}
              Probe now
            </Button>
            <Button
              size="sm"
              variant="default"
              onClick={deploy}
              disabled={deploying || dirty}
              title={
                dirty
                  ? "Save changes before deploying"
                  : "Bundle the project (+ runtime binary if locally built) and push to the edge"
              }
            >
              {deploying ? (
                <Loader2 className="mr-1.5 size-3 animate-spin" />
              ) : (
                <Rocket className="mr-1.5 size-3" />
              )}
              Deploy
            </Button>
            {attached ? (
              <Button
                size="sm"
                variant="outline"
                onClick={() => void onDetach()}
                className="border-emerald-500/40 text-emerald-700 dark:text-emerald-400"
              >
                <Link2Off className="mr-1.5 size-3" />
                Detach
              </Button>
            ) : (
              <Button
                size="sm"
                variant="outline"
                onClick={() => void onAttach()}
                disabled={probe?.reachable !== true}
                title={
                  probe?.reachable
                    ? "Open SSH port-forward and stream live variables from the edge runtime"
                    : "Edge unreachable — probe first"
                }
              >
                <Link2 className="mr-1.5 size-3" />
                Attach
              </Button>
            )}
          </div>
        </section>

        <section>
          <div className="mb-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
            Runtime status
          </div>
          {probe ? (
            <StatusGrid probe={probe} />
          ) : (
            <p className="text-xs text-muted-foreground">
              Probing… (or click <span className="font-mono">Probe now</span>)
            </p>
          )}
          {attached && (
            <div className="mt-3 inline-flex items-center gap-1.5 rounded-md border border-emerald-500/30 bg-emerald-500/5 px-2 py-1 text-[12px] text-emerald-800 dark:text-emerald-300">
              <Link2 className="size-3" />
              IDE is attached — Monitor and Variables panes are now showing
              live data from this edge.
            </div>
          )}
        </section>

        {(deployLog || deployError) && (
          <section>
            <div className="mb-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
              Last deploy
            </div>
            {deployError ? (
              <pre className="max-w-full overflow-auto rounded-md border border-red-500/40 bg-red-500/5 p-3 text-xs text-red-700 dark:text-red-400">
                {deployError}
              </pre>
            ) : deployLog ? (
              <div>
                <div className="mb-1 inline-flex items-center gap-1.5 text-[12px] text-emerald-800 dark:text-emerald-300">
                  <CheckCircle2 className="size-3.5" />
                  Version{" "}
                  <span className="font-mono">{deployLog.version}</span> live
                </div>
                <pre className="max-h-64 max-w-full overflow-auto rounded-md border border-border bg-muted/40 p-3 text-[11px] text-muted-foreground">
                  {deployLog.log}
                </pre>
              </div>
            ) : null}
          </section>
        )}
      </div>
    </>
  )
}

function Field({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="space-y-1.5">
      <Label className="text-[11px] uppercase tracking-wider text-muted-foreground">
        {label}
      </Label>
      {children}
    </div>
  )
}

function ReachBadge({
  probe,
  probing,
}: {
  probe: EdgeProbe | null
  probing: boolean
}) {
  if (probing) {
    return (
      <span className="inline-flex items-center gap-1 rounded-md bg-muted/50 px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
        <Loader2 className="size-3 animate-spin" />
        probing
      </span>
    )
  }
  if (!probe) {
    return (
      <span className="inline-flex items-center gap-1 rounded-md bg-muted/50 px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
        unknown
      </span>
    )
  }
  if (probe.reachable) {
    return (
      <span
        className="inline-flex items-center gap-1 rounded-md bg-emerald-500/15 px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-emerald-700 dark:text-emerald-400"
        title="Edge runtime is responding"
      >
        <CheckCircle2 className="size-3" />
        running
      </span>
    )
  }
  return (
    <span
      className="inline-flex items-center gap-1 rounded-md bg-red-500/15 px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-red-700 dark:text-red-400"
      title={probe.error ?? ""}
    >
      <AlertCircle className="size-3" />
      unreachable
    </span>
  )
}

function StatusGrid({ probe }: { probe: EdgeProbe }) {
  return (
    <dl className="grid max-w-2xl grid-cols-3 gap-3 text-sm">
      <Stat
        label="Uptime"
        value={
          probe.uptime_secs == null
            ? "—"
            : formatDuration(Number(probe.uptime_secs))
        }
      />
      <Stat
        label="Scan count"
        value={
          probe.scan_count == null ? "—" : Number(probe.scan_count).toLocaleString()
        }
      />
      <Stat label="Runtime version" value={probe.runtime_version ?? "—"} />
      {!probe.reachable && probe.error && (
        <div className="col-span-3">
          <pre className="overflow-auto rounded-md border border-red-500/40 bg-red-500/5 p-2 text-[11px] text-red-700 dark:text-red-400">
            {probe.error}
          </pre>
        </div>
      )}
    </dl>
  )
}

function Stat({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="rounded-md border border-border bg-muted/30 p-2">
      <dt className="text-[10px] uppercase tracking-wider text-muted-foreground">
        {label}
      </dt>
      <dd className="font-mono text-sm tabular-nums">{value}</dd>
    </div>
  )
}

function formatDuration(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return "—"
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  const s = Math.floor(secs % 60)
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m ${s}s`
  return `${s}s`
}
