import { describe, expect, test } from "bun:test";
import { OmnigraphError, OmnigraphStore } from "../src/omnigraph";

describe("P3-03 Multi-Agent Omnigraph Git-Model Memory", () => {
  test("supports @key branches, --snapshot time-travel, 409 divergent_update conflict detection & 403 worker merge block", () => {
    const og = new OmnigraphStore();
    og.put("main", "@arch.router", "v1-local-first", "reviewer");
    const snapV1 = og.currentSnapshot;

    og.createBranch("agent/a", "main");
    og.createBranch("agent/b", "main");

    // Stale expectedBaseVersion throws 409 divergent_update
    try {
      og.put("agent/a", "@arch.router", "bad-stale", "agent-a", 99);
      throw new Error("should have thrown");
    } catch (e) {
      expect(e).toBeInstanceOf(OmnigraphError);
      expect((e as OmnigraphError).status).toBe(409);
      expect((e as OmnigraphError).code).toBe("divergent_update");
    }

    // Valid branch writes
    og.put("agent/a", "@arch.router", "v2-from-a", "agent-a", 1);
    og.put("agent/b", "@arch.router", "v2-from-b", "agent-b", 1);

    // Worker cannot merge to main (HTTP 403)
    try {
      og.mergeToMain("agent/a", "worker");
      throw new Error("should have thrown 403");
    } catch (e) {
      expect((e as OmnigraphError).status).toBe(403);
    }

    // Reviewer merges agent/a cleanly
    const mergedA = og.mergeToMain("agent/a", "reviewer");
    expect(mergedA.mergedKeys).toEqual(["@arch.router"]);
    expect(og.get("main", "@arch.router")?.value).toBe("v2-from-a");

    // Merging agent/b now collides on merge_base -> 409 divergent_update (blocks lost update!)
    try {
      og.mergeToMain("agent/b", "reviewer");
      throw new Error("should have thrown 409");
    } catch (e) {
      expect((e as OmnigraphError).status).toBe(409);
      expect((e as OmnigraphError).code).toBe("divergent_update");
    }

    // --snapshot time-travel read recovers exact v1 state
    expect(og.get("main", "@arch.router", { snapshot: snapV1 })?.value).toBe("v1-local-first");
  });
});