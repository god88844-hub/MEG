import { describe, expect, test } from "bun:test";
import {
  L1AfterEditReminder,
  L2_SELF_RECHECK_ENABLED,
  aggregateSequentialCritics,
  assignColdCrossModelCritic,
  buildColdCriticPacket,
  enforceCriticToolPolicy,
} from "../src/critic";

describe("P4-04 L1 After-Edit Reminders & Cold Cross-Model L3 Critic", () => {
  test("disables L2 self-recheck and caps L1 after-edit reminders at 4/run", () => {
    expect(L2_SELF_RECHECK_ENABLED).toBeFalse();
    const l1 = new L1AfterEditReminder();
    expect(l1.emit("a.ts", "run unit test")).toContain("1/4");
    expect(l1.emit("b.ts", "run unit test")).toContain("2/4");
    expect(l1.emit("c.ts", "run unit test")).toContain("3/4");
    expect(l1.emit("d.ts", "run unit test")).toContain("4/4");
    expect(l1.emit("e.ts", "run unit test")).toBeNull();
  });

  test("assigns cross-family critic (>= writer tier), strips maker conclusion in blind mode, enforces /tmp non-mutation, and bans PROXY/SHARED MISTAKE", () => {
    const critic = assignColdCrossModelCritic(
      { id: "claude-sonnet-4", family: "anthropic", capabilityTier: 3 },
      [
        { id: "claude-opus-4", family: "anthropic", capabilityTier: 3 }, // same family -> excluded
        { id: "qwen-7b", family: "local", capabilityTier: 1 }, // lower capability -> excluded
        { id: "gpt-5", family: "openai", capabilityTier: 3 }, // valid cross-family >= tier
      ],
    );
    expect(critic.id).toBe("gpt-5");

    const packet = buildColdCriticPacket({
      contract: "Fix parser",
      gitDiff: "+return parsed;",
      testLogs: "10 pass, 0 fail",
      makerConclusion: "MAKER_BIASED_SUMMARY_HIDDEN",
      makerChatTranscript: "RAW_CHAT_NEVER_LEAKED",
      conclusionBlind: true,
      claimGuidedChanges: ["Parser returns structured AST"],
    });
    expect(packet.freshContext).toBeTrue();
    expect(packet.prompt).toContain("Claim-guided: changes");
    expect(packet.prompt).not.toContain("MAKER_BIASED_SUMMARY_HIDDEN");
    expect(packet.prompt).not.toContain("RAW_CHAT_NEVER_LEAKED");

    // Non-mutation & /tmp-only throwaway probes
    expect(enforceCriticToolPolicy("edit_file", "src/a.ts").allowed).toBeFalse();
    expect(enforceCriticToolPolicy("write_file", "src/a.ts").allowed).toBeFalse();
    expect(enforceCriticToolPolicy("write_file", "/tmp/probe.py").allowed).toBeTrue();

    // Width=2 sequential critics + PROXY/SHARED MISTAKE bans
    const banned = aggregateSequentialCritics(2, [
      { criticId: "c1", approved: true, issues: [], rationale: "Looks fine via PROXY metric" },
      { criticId: "c2", approved: true, issues: [], rationale: "SHARED MISTAKE acceptable" },
    ]);
    expect(banned.approved).toBeFalse();
    expect(banned.bannedPatternHits).toEqual(["c1:PROXY", "c2:SHARED_MISTAKE"]);

    const clean = aggregateSequentialCritics(2, [
      { criticId: "c1", approved: true, issues: [], rationale: "Verified diff against contract." },
      { criticId: "c2", approved: true, issues: [], rationale: "No defects found in logs." },
    ]);
    expect(clean.approved).toBeTrue();
    expect(clean.revisionsAllowed).toBe(1);
  });
});