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