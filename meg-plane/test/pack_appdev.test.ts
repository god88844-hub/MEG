import { describe, expect, test } from "bun:test";
import path from "node:path";
import { loadSkillFromDir } from "../src/skills";
import {
  buildDapLaunchConfig,
  runLspDiagnosticsOnEdit,
  runPercentCheckHarness,
  scrubAccessibilityTreePii,
  verifyBombadilUiFuzzTrace,
  verifyBootstrapOrdering,
} from "../src/pack_appdev";

describe("P6-03 Pack 2 -- Web, Full-Stack, Backend, Mobile, Desktop, DB, Enterprise & QA", () => {
  test("SKILL.md loads cleanly, LSP/DAP/Bombadil/bootstrap/agent-desktop PII scrub & xlsx PercentCheckHarness work", async () => {
    const skill = loadSkillFromDir(path.resolve(import.meta.dir, "../skills/pack-appdev"));
    expect(skill.metadata.name).toBe("pack-appdev");

    const lsp = await runLspDiagnosticsOnEdit("src/app.ts", () => []);
    expect(lsp.within50msBudget).toBeTrue();
    expect(lsp.clean).toBeTrue();

    expect(buildDapLaunchConfig("debugpy", "app.py", [12]).command).toContain("debugpy");
    expect(buildDapLaunchConfig("dlv", "main.go", [5]).command[0]).toBe("dlv");
    expect(buildDapLaunchConfig("lldb", "bin", [1]).command[0]).toBe("lldb-vscode");

    expect(
      verifyBombadilUiFuzzTrace([
        { action: "click", target: "#submit", consoleErrors: [], uncaughtExceptions: [] },
      ]).passed,
    ).toBeTrue();
    expect(
      verifyBombadilUiFuzzTrace([
        { action: "click", target: "#crash", consoleErrors: ["TypeError"], uncaughtExceptions: [] },
      ]).passed,
    ).toBeFalse();

    expect(verifyBootstrapOrdering(["load_secrets", "db_migrate", "app_start", "healthcheck"]).ok).toBeTrue();
    expect(verifyBootstrapOrdering(["app_start", "db_migrate", "load_secrets", "healthcheck"]).ok).toBeFalse();

    const scrubbed = scrubAccessibilityTreePii({
      role: "window",
      name: "Profile for alice@example.com",
      value: "SSN 123-45-6789",
      children: [{ role: "textbox", name: "Phone", value: "+91-9876543210" }],
    });
    expect(scrubbed.name).toContain("<PII_EMAIL>");
    expect(scrubbed.value).toContain("<PII_SSN>");
    expect(scrubbed.children?.[0]?.value).toContain("<PII_PHONE>");

    const pct = runPercentCheckHarness([
      { cell: "B7", value: 0.185, numberFormat: "General", expectPercent: true },
      { cell: "B8", value: 0.25, numberFormat: "0.0%", expectPercent: true },
    ]);
    expect(pct.ok).toBeFalse();
    expect(pct.audits[0]?.normalizedPercent).toBe(18.5);
    expect(pct.audits[0]?.convertedFromFraction).toBeTrue();
    expect(pct.audits[1]?.normalizedPercent).toBe(25);
  });
});