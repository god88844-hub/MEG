// The TS plane schedules delegates; the Rust worker owns inference and tools.
// Ensures Windows process-tree cleanup on abort/timeout/shutdown and emits
// structured lifecycle & progress events.

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import fs from "node:fs";
import { routeTask } from "./security-router";

export type AgentEvent = {
  id: string;
  role: string;
  status: string;
  phase: "starting" | "working" | "done";
  elapsed: number;
  activeCount: number;
  usage?: unknown;
};

type Task = { child: ChildProcess; role: string; workspace: string; started: number };

/** Kill a child process and its full descendant tree (Windows taskkill /T /F). */
export function killProcessTree(child: ChildProcess): void {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const pid = child.pid;
  if (process.platform === "win32" && typeof pid === "number" && pid > 0) {
    try {
      spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], {
        windowsHide: true,
        stdio: "ignore",
        timeout: 3000,
      });
    } catch {
      // Fall through to child.kill()
    }
  }
  try {
    child.kill("SIGKILL");
  } catch {
    try {
      child.kill();
    } catch {
      // Already exited
    }
  }
}

export class AgentManager {
  private active = new Map<string, Task>();
  private handoffs = new Map<string, number>();

  constructor(private limit = 4, private heartbeatMs = 1500) {}

  async run(params: any, signal: AbortSignal, notify: (event: AgentEvent) => void) {
    const role = params.role ?? "researcher";
    if (!["planner", "researcher", "reviewer"].includes(role)) {
      throw new Error("Delegate role must be planner, researcher or reviewer; delegated tools are read-only");
    }
    if (typeof params.task !== "string" || !params.task.trim() || params.task.length > 32_000) {
      throw new Error("Delegate needs a task (up to 32,000 characters)");
    }
    const handoffId = String(params.worker?.handoff_id ?? randomUUID());
    const count = this.handoffs.get(handoffId) ?? 0;
    const route = routeTask(null, params.task, count);
    if (route.kind === "root-thread") throw new Error(route.reason);
    this.handoffs.set(handoffId, count + 1);
    if (this.handoffs.size > 1024) this.handoffs.delete(this.handoffs.keys().next().value!);
    if (this.active.size >= this.limit) {
      throw new Error(`Delegate limit reached (${this.limit}); wait for an active worker`);
    }
    const executable = process.env.MEG_NATIVE_EXE;
    if (!executable || !fs.existsSync(executable)) {
      throw new Error("Native MEG worker executable is unavailable");
    }
    if (signal.aborted) throw new Error("Operation cancelled");

    const id = `${role}-${randomUUID().slice(0, 8)}`;
    const started = Date.now();
    const spawnArgs = Array.isArray(params._testArgs) ? params._testArgs : ["worker"];
    const child = spawn(executable, spawnArgs, {
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
      env: { ...process.env, MEG_PLANE_DISABLED: "1" },
    });
    child.stdin!.on("error", () => {});
    this.active.set(id, { child, role, workspace: params.workspace, started });

    const report = (
      status: string,
      phase: AgentEvent["phase"],
      usage?: unknown,
    ) => {
      const ev: AgentEvent = {
        id,
        role,
        status,
        phase,
        elapsed: (Date.now() - started) / 1000,
        activeCount: this.active.size,
      };
      if (usage !== undefined) ev.usage = usage;
      notify(ev);
    };

    report("running", "starting");
    let size = 0;
    const chunks: Buffer[] = [];
    child.stdout!.on("data", (chunk: Buffer) => {
      size += chunk.length;
      if (size > 2 * 1024 * 1024) killProcessTree(child);
      else chunks.push(chunk);
    });
    child.stderr!.on("data", () => {});

    const stop = () => killProcessTree(child);
    signal.addEventListener("abort", stop, { once: true });
    const timeoutMs = typeof params.timeoutMs === "number" && params.timeoutMs > 0
      ? Math.min(params.timeoutMs, 300_000)
      : 300_000;
    const timeout = setTimeout(stop, timeoutMs);
    const heartbeat = this.heartbeatMs > 0
      ? setInterval(() => report("running", "working"), this.heartbeatMs)
      : null;

    try {
      const result = new Promise<void>((resolve, reject) => {
        child.once("error", () => reject(new Error("Could not start native MEG worker")));
        child.once("close", (code) =>
          code === 0
            ? resolve()
            : reject(new Error(signal.aborted ? "Delegate cancelled" : "Delegate failed or timed out")),
        );
      });
      child.stdin!.end(
        JSON.stringify({
          ...params.worker,
          workspace: params.workspace,
          goal: `Act as a ${role}. This delegated run is read-only. Return useful findings and evidence.\nTask: ${params.task}`,
          mode: "plan",
        }),
      );
      await result;
      const payload = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      if (typeof payload.answer !== "string") {
        throw new Error(typeof payload.error === "string" ? payload.error : "Native worker returned no answer");
      }
      report("completed", "done", payload.usage);
      return { ...payload, id, role, message: `${role}: ${payload.answer}` };
    } catch (error) {
      report(signal.aborted ? "cancelled" : "failed", "done");
      throw error;
    } finally {
      if (heartbeat) clearInterval(heartbeat);
      clearTimeout(timeout);
      signal.removeEventListener("abort", stop);
      this.active.delete(id);
    }
  }

  cancelAll() {
    for (const task of this.active.values()) killProcessTree(task.child);
  }

  status() {
    return [...this.active].map(([id, t]) => ({
      id,
      role: t.role,
      workspace: t.workspace,
      status: "running",
      elapsed: (Date.now() - t.started) / 1000,
    }));
  }
}