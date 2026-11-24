import { describe, expect, test } from "bun:test";
import {
  TOOL_ERROR_CATEGORIES,
  evaluateSubstitutionCertificate,
  isValidToolErrorCategory,
  signExecutionReceipt,
  updateMonotonicFeature,
  validateSwarmHandoff,
  verifyExecutionReceipt,
  type FeaturesManifest,
} from "../src/types";

describe("P2-01 Canonical Schemas & Contracts", () => {
  test("signs and verifies ExecutionReceipt and rejects tampering", () => {
    const receipt = signExecutionReceipt("P2-01", "abc123tree", true);
    expect(verifyExecutionReceipt(receipt, "P2-01", "abc123tree").valid).toBeTrue();
    expect(verifyExecutionReceipt({ ...receipt, treeHash: "tampered" }, "P2-01").valid).toBeFalse();
    expect(verifyExecutionReceipt(signExecutionReceipt("P2-01", "abc123tree", false)).valid).toBeFalse();
  });

  test("enforces monotonic readOnly lock on passing features.json items", () => {
    let manifest: FeaturesManifest = {
      version: 1,
      items: [{ id: "F1", description: "Auth flow", passed: false, readOnly: false }],
    };
    manifest = updateMonotonicFeature(manifest, "F1", true, "sha-ok");
    expect(manifest.items[0]?.passed).toBeTrue();
    expect(manifest.items[0]?.readOnly).toBeTrue();
    expect(() => updateMonotonicFeature(manifest, "F1", false)).toThrow("Monotonic feature");
  });

  test("validates SubstitutionCertificate, SwarmHandoff, and ToolErrorCategory", () => {
    expect(evaluateSubstitutionCertificate("n1", 5000, 1000, 500, 500).approved).toBeTrue();
    expect(evaluateSubstitutionCertificate("n2", 800, 500, 300, 200).approved).toBeFalse();

    const handoff = validateSwarmHandoff({
      handoffId: "h-1",
      role: "researcher",
      objective: "Inspect API routes",
      allowedFiles: ["src/index.ts"],
      constraints: ["read-only"],
      acceptanceCriteria: ["list all RPC methods"],
      budget: { maxTurns: 6, maxTokens: 16000 },
    });
    expect(handoff.role).toBe("researcher");

    for (const c of TOOL_ERROR_CATEGORIES) {
      expect(isValidToolErrorCategory(c)).toBeTrue();
    }
    expect(isValidToolErrorCategory("MadeUp")).toBeFalse();
  });
});