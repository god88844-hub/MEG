import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  BiTemporalMemoryStore,
  isVolatileFastFactKey,
  lexicalSearchWorkspace,
  stairTocLeafRetrieve,
} from "../src/memory";

describe("P3-01 4-Part Memory Stack & Bi-Temporal Ledger", () => {
  test("closes bi-temporal intervals instead of overwriting, validates SHA-256 staleness, enforces HYRA 2.0 Books & blocks volatile Fast-Facts", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p301-"));
    try {
      const src = path.join(ws, "config.ts");
      fs.writeFileSync(src, "export const PORT = 3000;\n");

      const store = new BiTemporalMemoryStore(path.join(ws, ".meg", "memory.jsonl"));

      // Grok Bot Fast-Fact rule blocks volatile live state.
      for (const badKey of ["stripe.price", "user.plan_tier", "pr.ci_status", "issue.assignee", "acct.balance"]) {
        expect(isVolatileFastFactKey(badKey)).toBeTrue();
        expect(() =>
          store.recordFact({ key: badKey, value: "live", epistemic: "KNOWS" }),
        ).toThrow("FAST_FACT_BLOCKED");
      }

      // Bi-temporal interval preservation.
      const r1 = store.recordFact({
        key: "app.port",
        value: "3000",
        epistemic: "VERIFIED",
        validFrom: 1000,
        learnedAt: 1000,
        workspace: ws,
        sourceFile: "config.ts",
      });
      store.recordFact({
        key: "app.port",
        value: "8080",
        epistemic: "VERIFIED",
        validFrom: 2000,
        learnedAt: 2000,
        workspace: ws,
        sourceFile: "config.ts",
      });

      expect(store.history("app.port")).toHaveLength(2);
      expect(store.queryAsOf("app.port", 1500)?.value).toBe("3000");
      expect(store.queryAsOf("app.port", 2500)?.value).toBe("8080");

      // SHA-256 staleness invalidation when source file changes.
      expect(store.isStale(r1.id, ws)).toBeFalse();
      fs.writeFileSync(src, "export const PORT = 8080;\n");
      expect(store.isStale(r1.id, ws)).toBeTrue();

      // HYRA 2.0 3-tier Books: Observation -> Verified -> Convention [human approved].
      expect(() => store.promoteBook(r1.id, "Convention", { humanApproved: true })).toThrow("HYRA_POLICY");
      store.promoteBook(r1.id, "Verified");
      expect(() => store.promoteBook(r1.id, "Convention", { humanApproved: false })).toThrow("humanApproved");
      const conv = store.promoteBook(r1.id, "Convention", { humanApproved: true });
      expect(conv.bookTier).toBe("Convention");
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });

  test("lexical workspace search and STAIR ToC leaf retrieval work offline", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p301-lex-"));
    try {
      fs.writeFileSync(path.join(ws, "alpha.ts"), "export function computeRate() { return 42; }\n");
      const hits = lexicalSearchWorkspace(ws, "computeRate");
      expect(hits).toHaveLength(1);
      expect(hits[0]?.file).toBe("alpha.ts");

      const doc = [
        "# Manual",
        "## Networking",
        "### Retries",
        "Use exponential backoff with Retry-After header.",
        "### Timeouts",
        "Set connect timeout to 10s.",
        "## Storage",
        "### Snapshots",
        "Store bare git snapshots out of tree.",
      ].join("\n");
      const leaves = stairTocLeafRetrieve(doc, "retry backoff header", 2);
      expect(leaves[0]?.heading).toBe("Retries");
      expect(leaves[0]?.breadcrumb).toBe("Manual > Networking > Retries");
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});