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
// ===========================================================================
// P5-02: DeepSeek Router-Worker, 4-Shape Swarm, TeamCreate Locks & Return Edges
// ===========================================================================

import { createHash as createHashP502 } from "node:crypto";
import { validateSwarmHandoff, type SwarmHandoff } from "./types";

export interface PrefillTrigger {
  id: string;
  threadId: string;
  payload: string;
}

/**
 * Persistent Bash-less Global Router (DeepSeek Router-Worker pattern):
 * - Strictly Bash-less (`bashEnabled = false`)
 * - Trigger prefill batching (`queueTrigger` -> `flushPrefillBatch`)
 * - Thread Subscription direct pass (`subscribeThread` / `publishDirect`)
 * - Full-Duplex `start()` / `stop()`
 */
export class GlobalRouter {
  readonly bashEnabled = false as const;
  private running = false;
  private prefillQueue: PrefillTrigger[] = [];
  private subscriptions = new Map<string, Set<(msg: PrefillTrigger) => void>>();

  start(): void {
    this.running = true;
  }

  stop(): void {
    this.running = false;
    this.prefillQueue = [];
  }

  get isRunning(): boolean {
    return this.running;
  }

  assertToolAllowed(toolName: string): void {
    if (toolName === "bash" || toolName === "shell") {
      throw new Error("GLOBAL_ROUTER_POLICY: persistent Global Router is strictly Bash-less");
    }
  }

  subscribeThread(threadId: string, handler: (msg: PrefillTrigger) => void): () => void {
    let set = this.subscriptions.get(threadId);
    if (!set) {
      set = new Set();
      this.subscriptions.set(threadId, set);
    }
    set.add(handler);
    return () => {
      set!.delete(handler);
    };
  }

  /** Queue trigger for prefill batching; subscribers on the thread also get direct pass immediately. */
  dispatchTrigger(trigger: PrefillTrigger): void {
    if (!this.running) throw new Error("GlobalRouter is stopped; call start() first");
    this.prefillQueue.push(trigger);
    const subs = this.subscriptions.get(trigger.threadId);
    if (subs) {
      for (const fn of subs) fn(trigger);
    }
  }

  flushPrefillBatch(maxBatch = 16): PrefillTrigger[] {
    return this.prefillQueue.splice(0, maxBatch);
  }
}

export type SwarmShape = "FAN-OUT" | "CHAIN" | "DEBATE" | "SUPERVISOR";
export const MAX_FANOUT_ITEMS_PER_WORKER = 10;
export const MAX_TEAMMATES = 5;

export interface SwarmShapePlan {
  shape: SwarmShape;
  workerCount: number;
  maxItemsPerWorker?: number;
  requireIndependentProposalsBeforeInteraction?: boolean;
  reason: string;
}

/** 4-Shape Swarm Classifier (`FAN-OUT <=10 items/worker`, `CHAIN`, `DEBATE`, `SUPERVISOR`). */
export function classifySwarmShape(input: {
  independentItems?: number;
  sequentialStages?: number;
  needsAdversarialConsensus?: boolean;
  dynamicCoordination?: boolean;
}): SwarmShapePlan {
  if (input.needsAdversarialConsensus) {
    return {
      shape: "DEBATE",
      workerCount: 2,
      requireIndependentProposalsBeforeInteraction: true,
      reason: "Adversarial review requires independent proposals before cross-agent interaction",
    };
  }
  if ((input.independentItems ?? 0) > 1) {
    const items = input.independentItems!;
    const workers = Math.min(MAX_TEAMMATES, Math.ceil(items / MAX_FANOUT_ITEMS_PER_WORKER));
    return {
      shape: "FAN-OUT",
      workerCount: workers,
      maxItemsPerWorker: Math.min(MAX_FANOUT_ITEMS_PER_WORKER, Math.ceil(items / workers)),
      reason: `Disjoint items (${items}) partitioned at <=${MAX_FANOUT_ITEMS_PER_WORKER} items/worker`,
    };
  }
  if ((input.sequentialStages ?? 0) > 1) {
    return {
      shape: "CHAIN",
      workerCount: Math.min(MAX_TEAMMATES, input.sequentialStages!),
      reason: "Ordered multi-stage dependency chain",
    };
  }
  return {
    shape: "SUPERVISOR",
    workerCount: 1,
    reason: "Dynamic coordination under single supervisor",
  };
}

/** Validate independent DEBATE proposals before allowing cross-agent critique. */
export function assertIndependentDebateProposals(
  proposals: readonly { agentId: string; proposal: string; sawPeerOutput: boolean }[],
): void {
  if (proposals.length < 2) {
    throw new Error("DEBATE requires at least 2 independent proposals");
  }
  const leaked = proposals.find((p) => p.sawPeerOutput);
  if (leaked) {
    throw new Error(
      `DEBATE_VIOLATION: agent '${leaked.agentId}' saw peer output before submitting independent proposal`,
    );
  }
}

export interface TeammateRecord {
  id: string;
  handoff: SwarmHandoff;
  ownedFiles: string[];
  active: boolean;
}

/** `TeamCreate` coordinator enforcing 1-File-1-Owner locks, <=5 teammates, and early shutdown. */
export class TeamCreateCoordinator {
  private teammates = new Map<string, TeammateRecord>();
  private fileOwners = new Map<string, string>();

  spawnTeammate(id: string, handoffInput: SwarmHandoff): TeammateRecord {
    const handoff = validateSwarmHandoff(handoffInput);
    const activeCount = [...this.teammates.values()].filter((t) => t.active).length;
    if (activeCount >= MAX_TEAMMATES) {
      throw new Error(`TEAM_LIMIT: cannot exceed ${MAX_TEAMMATES} active teammates`);
    }
    const normFiles = handoff.allowedFiles.map((f) => f.replace(/\\/g, "/"));
    for (const f of normFiles) {
      const existingOwner = this.fileOwners.get(f);
      if (existingOwner && existingOwner !== id) {
        throw new Error(`FILE_LOCK_CONFLICT: '${f}' is already owned by teammate '${existingOwner}'`);
      }
    }
    for (const f of normFiles) {
      this.fileOwners.set(f, id);
    }
    const rec: TeammateRecord = { id, handoff, ownedFiles: normFiles, active: true };
    this.teammates.set(id, rec);
    return rec;
  }

  /** Early shutdown: immediately release teammate slot and file locks as soon as slice finishes. */
  shutdownTeammate(id: string): void {
    const rec = this.teammates.get(id);
    if (!rec) return;
    rec.active = false;
    for (const f of rec.ownedFiles) {
      if (this.fileOwners.get(f) === id) this.fileOwners.delete(f);
    }
  }

  activeTeammates(): TeammateRecord[] {
    return [...this.teammates.values()].filter((t) => t.active);
  }

  ownerOf(filePath: string): string | undefined {
    return this.fileOwners.get(filePath.replace(/\\/g, "/"));
  }
}

export interface SliceEvaluation {
  unitId: string;
  files: string[];
  passed: boolean;
  failureSummary?: string;
}

export interface UnitOnlyReturnPacket {
  frozenReadOnlyFiles: string[];
  retryUnits: { unitId: string; fileFence: string[]; failureSummary: string }[];
  longLearningNotesForSplitter: string[];
}

/**
 * Unit-Only Return Edge + Long Learning Edge to Splitter:
 * - Freezes all passed slices read-only.
 * - Returns ONLY the failed unit(s) with an explicit file fence.
 * - Emits structural split lessons back to the Splitter.
 */
export function buildUnitOnlyReturnEdge(slices: readonly SliceEvaluation[]): UnitOnlyReturnPacket {
  const frozenReadOnlyFiles: string[] = [];
  const retryUnits: UnitOnlyReturnPacket["retryUnits"] = [];
  const longLearningNotesForSplitter: string[] = [];

  for (const s of slices) {
    if (s.passed) {
      frozenReadOnlyFiles.push(...s.files.map((f) => f.replace(/\\/g, "/")));
    } else {
      const fence = s.files.map((f) => f.replace(/\\/g, "/"));
      const summary = s.failureSummary ?? "verification failed";
      retryUnits.push({ unitId: s.unitId, fileFence: fence, failureSummary: summary });
      longLearningNotesForSplitter.push(
        `Splitter lesson [${s.unitId}]: failed on fence [${fence.join(", ")}] -> ${summary}`,
      );
    }
  }

  return {
    frozenReadOnlyFiles: [...new Set(frozenReadOnlyFiles)],
    retryUnits,
    longLearningNotesForSplitter,
  };
}

/** `Planfence` input-hash rebuild cache: avoids stale plans and redundant rebuilds. */
export class PlanfenceCache {
  private lastHashes = new Map<string, string>();

  static hashInputs(inputs: Record<string, string>): string {
    const keys = Object.keys(inputs).sort();
    const material = keys.map((k) => `${k}=${inputs[k]}`).join("\n");
    return createHashP502("sha256").update(material).digest("hex");
  }

  needsRebuild(planId: string, inputs: Record<string, string>): { rebuild: boolean; hash: string } {
    const hash = PlanfenceCache.hashInputs(inputs);
    const prev = this.lastHashes.get(planId);
    if (prev === hash) return { rebuild: false, hash };
    this.lastHashes.set(planId, hash);
    return { rebuild: true, hash };
  }
}

/** Loop-Until-Dry `seen` deduplication (`confirmed U rejected`). */
export class LoopUntilDrySeen {
  readonly confirmed = new Set<string>();
  readonly rejected = new Set<string>();

  hasSeen(key: string): boolean {
    return this.confirmed.has(key) || this.rejected.has(key);
  }

  markConfirmed(key: string): void {
    this.confirmed.add(key);
  }

  markRejected(key: string): void {
    this.rejected.add(key);
  }

  /** Filter incoming candidates down to only unseen items (`candidates \ (confirmed U rejected)`). */
  filterUnseen<T>(candidates: readonly T[], keyOf: (item: T) => string): T[] {
    return candidates.filter((item) => !this.hasSeen(keyOf(item)));
  }

  get isDry(): boolean {
    return this.confirmed.size + this.rejected.size > 0;
  }
}