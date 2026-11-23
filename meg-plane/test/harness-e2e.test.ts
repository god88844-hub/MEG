import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { loadFfi } from "../src/ffi-bridge";

class StdioRpcClient {
  private proc: ReturnType<typeof Bun.spawn>;
  private nextId = 1;
  private pending = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();
  readonly events: any[] = [];
  private closed: Promise<number>;

  constructor(extraEnv: Record<string, string> = {}) {
    const entry = path.resolve(import.meta.dir, "../src/index.ts");
    this.proc = Bun.spawn([process.execPath, "run", entry], {
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
      env: { ...process.env, ...extraEnv },
    });
    this.closed = this.proc.exited;
    void this.readLoop();
  }

  private async readLoop() {
    const decoder = new TextDecoder();
    const reader = (this.proc.stdout as ReadableStream<Uint8Array>).getReader();
    let buf = "";
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        let idx: number;
        while ((idx = buf.indexOf("\n")) !== -1) {
          const line = buf.slice(0, idx).trim();
          buf = buf.slice(idx + 1);
          if (!line) continue;
          const msg = JSON.parse(line);
          if (msg.id !== undefined && this.pending.has(msg.id)) {
            const p = this.pending.get(msg.id)!;
            this.pending.delete(msg.id);
            if (msg.error) p.reject(new Error(msg.error.message ?? "RPC error"));
            else p.resolve(msg.result);
          } else if (msg.method === "agent.event") {
            this.events.push(msg.params);
          }
        }
      }
    } finally {
      for (const p of this.pending.values()) p.reject(new Error("RPC stream closed"));
      this.pending.clear();
    }
  }

  sendRaw(obj: Record<string, unknown>) {
    (this.proc.stdin as any).write(JSON.stringify(obj) + "\n");
  }

  call(method: string, params: Record<string, unknown> = {}): { id: number; promise: Promise<any> } {
    const id = this.nextId++;
    const promise = new Promise<any>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
    this.sendRaw({ jsonrpc: "2.0", id, method, params });
    return { id, promise };
  }

  async close() {
    try {
      (this.proc.stdin as any).end();
    } catch {
      // ignore
    }
    const timer = setTimeout(() => this.proc.kill(), 2000);
    await this.closed;
    clearTimeout(timer);
  }
}

describe("Phase 6 Control-Plane & FFI E2E", () => {
  test("stdio JSON-RPC persists SessionTree across full control-plane restarts", async () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p6-rpc-"));
    const c1 = new StdioRpcClient();
    let rootId = "";
    let branchId = "";
    try {
      const ping = await c1.call("runtime.ping").promise;
      expect(ping.ready).toBeTrue();

      const created = await c1.call("session.create", { workspace: ws, sessionId: "e2e-restart" }).promise;
      expect(created.persisted).toBeTrue();

      const n1 = await c1.call("session.append", {
        workspace: ws,
        sessionId: "e2e-restart",
        parentId: null,
        kind: "user",
        content: "initial task",
      }).promise;
      rootId = n1.id;

      await c1.call("session.append", {
        workspace: ws,
        sessionId: "e2e-restart",
        parentId: rootId,
        kind: "assistant",
        content: "path A",
      }).promise;

      const n3 = await c1.call("session.append", {
        workspace: ws,
        sessionId: "e2e-restart",
        parentId: rootId,
        kind: "branch",
        content: "path B",
      }).promise;
      branchId = n3.id;
    } finally {
      await c1.close();
    }

    // Spawn a brand-new control-plane process and verify disk state reloads cleanly.
    const c2 = new StdioRpcClient();
    try {
      const list = await c2.call("session.list", { workspace: ws }).promise;
      expect(list.sessions).toContain("e2e-restart");

      const loaded = await c2.call("session.load", { workspace: ws, sessionId: "e2e-restart" }).promise;
      expect(loaded.size).toBe(3);

      const forked = await c2.call("session.forkPath", {
        workspace: ws,
        sessionId: "e2e-restart",
        nodeId: branchId,
      }).promise;
      expect(forked.map((n: any) => n.content)).toEqual(["initial task", "path B"]);
    } finally {
      await c2.close();
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });

  test("stdio JSON-RPC runtime.cancel aborts an active delegate worker cleanly", async () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p6-cancel-"));
    const client = new StdioRpcClient({ MEG_NATIVE_EXE: process.execPath });
    try {
      const started = Date.now();
      const { id, promise } = client.call("agents.run", {
        workspace: ws,
        role: "researcher",
        task: "inspect repo structure",
        _testArgs: ["-e", "setInterval(() => {}, 1000)"],
      });
      setTimeout(() => {
        client.sendRaw({ jsonrpc: "2.0", method: "runtime.cancel", params: { requestId: id } });
      }, 150);

      await expect(promise).rejects.toThrow("cancelled");
      expect(Date.now() - started).toBeLessThan(4000);
      const status = await client.call("agents.status").promise;
      expect(status.agents).toHaveLength(0);
    } finally {
      await client.close();
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });

  test("FFI bridge handles deep globs, grep, workspace index, and CRLF-preserving edits", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p6-ffi-"));
    try {
      fs.mkdirSync(path.join(ws, "a/b/c"), { recursive: true });
      fs.writeFileSync(path.join(ws, "a/b/c/mod.rs"), "pub fn deep_target() -> i32 {\r\n    1\r\n}\r\n");

      const ffi = loadFfi();
      expect(ffi.searchFiles(ws, "**/*.rs")).toContain("a/b/c/mod.rs");
      expect(ffi.grepFiles(ws, "deep_target", "**/*.rs", 10)).toContain("a/b/c/mod.rs:1:");
      expect(ffi.listDir(ws, ".")).toContain("dir  a/");

      const map = ffi.indexWorkspace(ws);
      expect(map).toContain("a/b/c/mod.rs");
      expect(map).toContain("pub fn deep_target");

      const edited = ffi.editFile(
        ws,
        "a/b/c/mod.rs",
        "pub fn deep_target() -> i32 {\n    1",
        "pub fn deep_target() -> i32 {\n    2",
        false,
      );
      expect(edited).toContain("edited");
      expect(fs.readFileSync(path.join(ws, "a/b/c/mod.rs"), "utf8")).toBe(
        "pub fn deep_target() -> i32 {\r\n    2\r\n}\r\n",
      );
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});