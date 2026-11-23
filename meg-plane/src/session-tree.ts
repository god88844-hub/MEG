// Session tree (Pi pattern, sweep 1): every turn/branch is a node with a
// parent — fork from any node without losing history.

export interface SessionNode {
  id: string;
  parentId: string | null;
  kind: "user" | "assistant" | "tool" | "branch";
  content: string;
  createdAt: number;
}

let counter = 0;
const nextId = () => `n${(++counter).toString(36)}-${Date.now().toString(36)}`;

export class SessionTree {
  private nodes = new Map<string, SessionNode>();
  private heads: string[] = []; // root-level node ids

  append(parentId: string | null, kind: SessionNode["kind"], content: string): SessionNode {
    const node: SessionNode = { id: nextId(), parentId, kind, content, createdAt: Date.now() };
    this.nodes.set(node.id, node);
    if (parentId === null) this.heads.push(node.id);
    return node;
  }

  /** Fork: replay the path to `nodeId`, then return it as the new tail. */
  pathTo(nodeId: string): SessionNode[] {
    const path: SessionNode[] = [];
    let cur: string | null = nodeId;
    while (cur) {
      const node = this.nodes.get(cur);
      if (!node) break;
      path.unshift(node);
      cur = node.parentId;
    }
    return path;
  }

  children(nodeId: string): SessionNode[] {
    return [...this.nodes.values()].filter((n) => n.parentId === nodeId);
  }

  /** Fork from any node: returns the replayed path; new nodes branch from it. */
  forkFrom(nodeId: string): SessionNode[] {
    return this.pathTo(nodeId);
  }

  get size(): number {
    return this.nodes.size;
  }
}
