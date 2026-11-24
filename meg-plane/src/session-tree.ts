// Session tree (Pi pattern, sweep 1): every turn/branch is a node with a
// parent -- fork from any node without losing history, with optional atomic
// persistence under `<workspace>/.meg/sessions/<sessionId>.json`.

import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";

export interface SessionNode {
  id: string;
  parentId: string | null;
  kind: "user" | "assistant" | "tool" | "branch";
  content: string;
  createdAt: number;
}

interface PersistedSessionTree {
  version: 1;
  sessionId: string;
  heads: string[];
  nodes: SessionNode[];
}

const SESSION_ID_RE = /^[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}$/;

let counter = 0;
const nextId = () => `n${(++counter).toString(36)}-${Date.now().toString(36)}`;

export function validateSessionId(sessionId: string): string {
  if (typeof sessionId !== "string" || !SESSION_ID_RE.test(sessionId)) {
    throw new Error("Invalid session identifier");
  }
  return sessionId;
}

export function sessionsDir(workspace: string): string {
  const root = fs.realpathSync(workspace);
  return path.join(root, ".meg", "sessions");
}

export function sessionFilePath(workspace: string, sessionId: string): string {
  validateSessionId(sessionId);
  const dir = sessionsDir(workspace);
  const full = path.join(dir, `${sessionId}.json`);
  const rel = path.relative(dir, full);
  if (rel.startsWith("..") || path.isAbsolute(rel)) {
    throw new Error("Session path escapes workspace");
  }
  return full;
}

function atomicWriteJson(file: string, value: unknown): void {
  const dir = path.dirname(file);
  fs.mkdirSync(dir, { recursive: true });
  const tmp = `${file}.${randomUUID()}.tmp`;
  try {
    fs.writeFileSync(tmp, JSON.stringify(value, null, 2), { flag: "wx" });
    fs.renameSync(tmp, file);
  } finally {
    if (fs.existsSync(tmp)) fs.unlinkSync(tmp);
  }
}

export class SessionTree {
  private nodes = new Map<string, SessionNode>();
  private heads: string[] = []; // root-level node ids
  readonly sessionId: string;
  private readonly storageFile: string | null;

  constructor(options?: { sessionId?: string; storageFile?: string }) {
    this.sessionId = options?.sessionId ? validateSessionId(options.sessionId) : `s-${Date.now().toString(36)}`;
    this.storageFile = options?.storageFile ?? null;
  }

  /** Open or create a persistent session tree under `<workspace>/.meg/sessions/`. */
  static open(workspace: string, sessionId: string): SessionTree {
    const file = sessionFilePath(workspace, sessionId);
    const tree = new SessionTree({ sessionId, storageFile: file });
    if (fs.existsSync(file)) {
      tree.loadFromDisk();
    } else {
      tree.save();
    }
    return tree;
  }

  /** List persisted session IDs in `<workspace>/.meg/sessions/`. */
  static list(workspace: string): string[] {
    const dir = sessionsDir(workspace);
    if (!fs.existsSync(dir)) return [];
    return fs
      .readdirSync(dir, { withFileTypes: true })
      .filter((e) => e.isFile() && e.name.endsWith(".json"))
      .map((e) => e.name.slice(0, -5))
      .filter((id) => SESSION_ID_RE.test(id))
      .sort();
  }

  private loadFromDisk(): void {
    if (!this.storageFile || !fs.existsSync(this.storageFile)) return;
    if (fs.statSync(this.storageFile).size > 4 * 1024 * 1024) {
      throw new Error("Persisted session tree exceeds 4 MiB");
    }
    const parsed = JSON.parse(fs.readFileSync(this.storageFile, "utf8")) as PersistedSessionTree;
    if (parsed?.version !== 1 || !Array.isArray(parsed.nodes)) {
      throw new Error("Invalid persisted session tree");
    }
    this.nodes.clear();
    this.heads = [];
    for (const node of parsed.nodes) {
      if (!node || typeof node.id !== "string" || typeof node.content !== "string") continue;
      this.nodes.set(node.id, node);
      if (node.parentId === null) this.heads.push(node.id);
    }
  }

  save(): void {
    if (!this.storageFile) return;
    const payload: PersistedSessionTree = {
      version: 1,
      sessionId: this.sessionId,
      heads: [...this.heads],
      nodes: [...this.nodes.values()],
    };
    atomicWriteJson(this.storageFile, payload);
  }

  append(parentId: string | null, kind: SessionNode["kind"], content: string): SessionNode {
    if (parentId !== null && !this.nodes.has(parentId)) {
      throw new Error(`unknown parent node: ${parentId}`);
    }
    const node: SessionNode = { id: nextId(), parentId, kind, content, createdAt: Date.now() };
    this.nodes.set(node.id, node);
    if (parentId === null) this.heads.push(node.id);
    this.save();
    return node;
  }

  /** Fork: replay the path to `nodeId`, then return it as the new tail. */
  pathTo(nodeId: string): SessionNode[] {
    const pathNodes: SessionNode[] = [];
    let cur: string | null = nodeId;
    const seen = new Set<string>();
    while (cur && !seen.has(cur)) {
      seen.add(cur);
      const node = this.nodes.get(cur);
      if (!node) break;
      pathNodes.unshift(node);
      cur = node.parentId;
    }
    return pathNodes;
  }

  children(nodeId: string): SessionNode[] {
    return [...this.nodes.values()].filter((n) => n.parentId === nodeId);
  }

  /** Fork from any node: returns the replayed path; new nodes branch from it. */
  forkFrom(nodeId: string): SessionNode[] {
    return this.pathTo(nodeId);
  }

  allNodes(): SessionNode[] {
    return [...this.nodes.values()];
  }

  get size(): number {
    return this.nodes.size;
  }
}
// ===========================================================================
// P2-02: Deterministic Task State Machine, 3-Zone Workspace, 6-Field progress.md & Replay
// ===========================================================================

import { type ExecutionReceipt, verifyExecutionReceipt } from "./types";

export type TaskState =
  | "CREATED"
  | "SCOPED"
  | "PLANNED"
  | "EXECUTING"
  | "VERIFYING"
  | "VERIFIED"
  | "NEEDS_WORK"
  | "BLOCKED"
  | "FAILED"
  | "CANCELLED";

const ALLOWED_TRANSITIONS: Record<TaskState, readonly TaskState[]> = {
  CREATED: ["SCOPED", "CANCELLED", "FAILED"],
  SCOPED: ["PLANNED", "BLOCKED", "CANCELLED", "FAILED"],
  PLANNED: ["EXECUTING", "BLOCKED", "CANCELLED", "FAILED"],
  EXECUTING: ["VERIFYING", "BLOCKED", "CANCELLED", "FAILED"],
  VERIFYING: ["VERIFIED", "NEEDS_WORK", "BLOCKED", "FAILED", "CANCELLED"],
  NEEDS_WORK: ["EXECUTING", "PLANNED", "BLOCKED", "FAILED", "CANCELLED"],
  BLOCKED: ["SCOPED", "PLANNED", "EXECUTING", "CANCELLED", "FAILED"],
  VERIFIED: [],
  FAILED: [],
  CANCELLED: [],
};

export interface TaskStateEvent {
  seq: number;
  taskId: string;
  from: TaskState | null;
  to: TaskState;
  reason?: string;
  receipt?: ExecutionReceipt;
  timestamp: number;
}

export class TaskStateMachine {
  readonly taskId: string;
  private current: TaskState = "CREATED";
  private events: TaskStateEvent[] = [];
  private receipt: ExecutionReceipt | null = null;
  private readonly ledgerFile: string | null;

  constructor(taskId: string, ledgerFile: string | null = null, skipInitialWrite = false) {
    this.taskId = validateSessionId(taskId);
    this.ledgerFile = ledgerFile;
    if (!skipInitialWrite) {
      this.recordEvent(null, "CREATED", "task created");
    }
  }

  get state(): TaskState {
    return this.current;
  }

  get verifiedReceipt(): ExecutionReceipt | null {
    return this.receipt;
  }

  history(): readonly TaskStateEvent[] {
    return this.events;
  }

  private recordEvent(
    from: TaskState | null,
    to: TaskState,
    reason?: string,
    receipt?: ExecutionReceipt,
  ): TaskStateEvent {
    const ev: TaskStateEvent = {
      seq: this.events.length + 1,
      taskId: this.taskId,
      from,
      to,
      reason,
      receipt,
      timestamp: Date.now(),
    };
    this.events.push(ev);
    if (this.ledgerFile) {
      fs.mkdirSync(path.dirname(this.ledgerFile), { recursive: true });
      fs.appendFileSync(this.ledgerFile, JSON.stringify(ev) + "\n", "utf8");
    }
    return ev;
  }

  transition(
    to: TaskState,
    options?: { reason?: string; receipt?: ExecutionReceipt; expectedTreeHash?: string },
  ): TaskStateEvent {
    const allowed = ALLOWED_TRANSITIONS[this.current];
    if (!allowed.includes(to)) {
      throw new Error(`Invalid task state transition: ${this.current} -> ${to}`);
    }
    if (to === "VERIFIED") {
      const check = verifyExecutionReceipt(options?.receipt, this.taskId, options?.expectedTreeHash);
      if (!check.valid) {
        throw new Error(`Cannot commit VERIFIED without machine-verified receipt: ${check.reason}`);
      }
      this.receipt = options!.receipt!;
    }
    const from = this.current;
    this.current = to;
    return this.recordEvent(from, to, options?.reason, options?.receipt);
  }

  /** Crash/restart replay: reconstruct state machine deterministically from append-only event ledger. */
  static replayFromLedger(taskId: string, ledgerFile: string): TaskStateMachine {
    if (!fs.existsSync(ledgerFile)) {
      throw new Error(`Task event ledger not found: ${ledgerFile}`);
    }
    const sm = new TaskStateMachine(taskId, ledgerFile, true);
    const lines = fs.readFileSync(ledgerFile, "utf8").split(/\r?\n/).filter((l) => l.trim().length > 0);
    for (const line of lines) {
      const ev = JSON.parse(line) as TaskStateEvent;
      if (ev.taskId !== sm.taskId) continue;
      if (ev.from === null && ev.to === "CREATED") {
        sm.current = "CREATED";
        sm.events.push(ev);
        continue;
      }
      const allowed = ALLOWED_TRANSITIONS[sm.current];
      if (!allowed.includes(ev.to)) {
        throw new Error(`Corrupt ledger replay transition: ${sm.current} -> ${ev.to}`);
      }
      if (ev.to === "VERIFIED") {
        const check = verifyExecutionReceipt(ev.receipt, sm.taskId);
        if (!check.valid) {
          throw new Error(`Corrupt ledger VERIFIED event: ${check.reason}`);
        }
        sm.receipt = ev.receipt!;
      }
      sm.current = ev.to;
      sm.events.push(ev);
    }
    return sm;
  }
}

export interface ThreeZonePaths {
  root: string;
  sources: string;
  drafts: string;
  published: string;
  progressFile: string;
}

/** Initialize 3-zone workspace (`sources/`, `drafts/`, `published/`) + 6-field `progress.md`. */
export function initThreeZoneWorkspace(workspace: string, initialTask = ""): ThreeZonePaths {
  const root = fs.realpathSync(workspace);
  const sources = path.join(root, "sources");
  const drafts = path.join(root, "drafts");
  const published = path.join(root, "published");
  fs.mkdirSync(sources, { recursive: true });
  fs.mkdirSync(drafts, { recursive: true });
  fs.mkdirSync(published, { recursive: true });
  const progressFile = path.join(root, "progress.md");
  if (!fs.existsSync(progressFile)) {
    writeProgressMd(progressFile, {
      task: initialTask || "(unassigned)",
      outputs: [],
      completed: [],
      decisions: [],
      openIssues: [],
      nextAction: "Scope task and lock contract",
    });
  }
  return { root, sources, drafts, published, progressFile };
}

export interface ProgressSnapshot {
  task: string;
  outputs: string[];
  completed: string[];
  decisions: string[];
  openIssues: string[];
  nextAction: string;
}

export function formatProgressMd(p: ProgressSnapshot): string {
  const list = (items: string[]) => (items.length ? items.map((i) => `- ${i}`).join("\n") : "- (none)");
  return [
    `# Progress`,
    ``,
    `## Task`,
    p.task,
    ``,
    `## Outputs`,
    list(p.outputs),
    ``,
    `## Completed`,
    list(p.completed),
    ``,
    `## Decisions`,
    list(p.decisions),
    ``,
    `## Open issues`,
    list(p.openIssues),
    ``,
    `## Next action`,
    p.nextAction,
    ``,
  ].join("\n");
}

export function writeProgressMd(file: string, p: ProgressSnapshot): void {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.${randomUUID()}.tmp`;
  try {
    fs.writeFileSync(tmp, formatProgressMd(p), { flag: "wx" });
    fs.renameSync(tmp, file);
  } finally {
    if (fs.existsSync(tmp)) fs.unlinkSync(tmp);
  }
}

export function parseProgressMd(raw: string): ProgressSnapshot {
  const section = (heading: string): string => {
    const re = new RegExp(`^## ${heading}\\r?\\n([\\s\\S]*?)(?=^## |\\Z)`, "m");
    const m = raw.match(re);
    return (m?.[1] ?? "").trim();
  };
  const parseList = (text: string): string[] =>
    text
      .split(/\r?\n/)
      .map((l) => l.replace(/^\s*-\s*/, "").trim())
      .filter((l) => l.length > 0 && l !== "(none)");
  return {
    task: section("Task"),
    outputs: parseList(section("Outputs")),
    completed: parseList(section("Completed")),
    decisions: parseList(section("Decisions")),
    openIssues: parseList(section("Open issues")),
    nextAction: section("Next action"),
  };
}