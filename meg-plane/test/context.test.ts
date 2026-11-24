import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  buildCompressedHandoff,
  compactProcessedMedia,
  compileContext,
  fetchNewestHits,
  shouldSplitLargeRead,
  summarizeLargeReadToFiveLines,
} from "../src/context";

describe("P2-06 5-Move CONTEXT.md Compiler & Media Compaction", () => {
  test("enforces Cut caps, strips HTML comments for stable KV-cache prefixHash, and anchors goal every 4 turns", () => {
    const baseInput = {
      rules: Array.from({ length: 20 }, (_, i) => `Rule ${i + 1} <!-- note -->`),
      tools: Array.from({ length: 9 }, (_, i) => ({ name: `t${9 - i}`, schema: "{}" })),
      examples: ["ex1", "ex2", "ex3"],
      agentsMd: Array.from({ length: 130 }, (_, i) => `agent line ${i}`).join("\n"),
      memoryMd: "m".repeat(3000),
      userMd: "u".repeat(2000),
      goal: "Ship feature X",
      turnNumber: 1,
    };
    const turn1 = compileContext(baseInput);
    const turn4 = compileContext({ ...baseInput, turnNumber: 4, todoMd: "- [ ] verify" });

    expect(turn1.cutReport.rulesKept).toBe(15);
    expect(turn1.cutReport.toolsKept).toBe(6);
    expect(turn1.cutReport.examplesKept).toBe(2);
    expect(turn1.cutReport.agentsLinesKept).toBe(100);
    expect(turn1.cutReport.memoryCharsKept).toBe(2200);
    expect(turn1.cutReport.userCharsKept).toBe(1375);
    expect(turn1.staticPrefix).not.toContain("<!--");
    // Static prefix hash stays identical across turns for >90% KV cache hit rate.
    expect(turn1.prefixHash).toBe(turn4.prefixHash);
    expect(turn1.tailGoalAnchored).toBeFalse();
    expect(turn4.tailGoalAnchored).toBeTrue();
    expect(turn4.dynamicTail.endsWith("GOAL: Ship feature X")).toBeTrue();
  });

  test("Fetch sorts newest-first (<=3), Compress strips prior tracebacks on fresh attempts, Split summarizes >40K reads, and compacts media to @media: pointers", () => {
    const hits = fetchNewestHits([
      { path: "old.ts", mtimeMs: 100, snippet: "a" },
      { path: "new1.ts", mtimeMs: 500, snippet: "b" },
      { path: "new2.ts", mtimeMs: 400, snippet: "c" },
      { path: "new3.ts", mtimeMs: 300, snippet: "d" },
    ]);
    expect(hits.map((h) => h.path)).toEqual(["new1.ts", "new2.ts", "new3.ts"]);

    const brief = buildCompressedHandoff({
      goal: "Fix test",
      completed: ["Edited a.ts"],
      filesTouched: ["a.ts"],
      currentAttemptErrors: ["AssertionError: expected 2"],
      priorAttemptTracebacks: ["RAW_OLD_TRACEBACK_NOISE"],
      nextStep: "Fix math",
      freshAttempt: true,
      contextWindowChars: 10_000,
    });
    expect(brief).toContain("AssertionError: expected 2");
    expect(brief).not.toContain("RAW_OLD_TRACEBACK_NOISE");
    expect(brief.length).toBeLessThanOrEqual(4_000);

    expect(shouldSplitLargeRead(45_000, 1)).toBeTrue();
    expect(shouldSplitLargeRead(1000, 25)).toBeTrue();
    const five = summarizeLargeReadToFiveLines("big.md", "a\nb\nc\nd\ne\nf");
    expect(five.split("\n")).toHaveLength(5);

    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-media-compact-"));
    try {
      const compacted = compactProcessedMedia(ws, [
        { kind: "image", mimeType: "image/png", base64Data: Buffer.from("fake-png-bytes").toString("base64") },
      ]);
      expect(compacted[0]?.base64Data).toBeUndefined();
      expect(compacted[0]?.pointer).toMatch(/^@media:\.meg\/media\/[a-f0-9]+\.png$/);
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});