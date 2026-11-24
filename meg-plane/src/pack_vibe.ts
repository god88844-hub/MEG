// P6-02 Pack 1: Vibe Coders, PM, Scrum & UI/UX Wireframing (Domains 1-3).

import fs from "node:fs";
import path from "node:path";

export type GoldfishCriticRole = "Contrarian" | "Edge-Case" | "Token Economist";

export interface GoldfishCritique {
  role: GoldfishCriticRole;
  stateless: true;
  findings: string[];
}

export interface FeatureSlice {
  sliceId: string;
  title: string;
  freshContext: true;
  acceptanceCriteria: string[];
}

export interface VibeToSpecResult {
  prdFile: string;
  critiques: GoldfishCritique[];
  slices: FeatureSlice[];
}

/**
 * Vibe-to-Spec front-end:
 * vague prompt -> Elephant `PRD.md` -> 3 stateless Goldfish critics -> 1-feature slices in fresh contexts.
 */
export function runVibeToSpecPipeline(
  workspace: string,
  vaguePrompt: string,
  features: readonly string[],
): VibeToSpecResult {
  const cleanFeatures = features.map((f) => f.trim()).filter(Boolean);
  if (cleanFeatures.length === 0) {
    throw new Error("Vibe-to-Spec requires at least 1 concrete feature slice");
  }

  const critiques: GoldfishCritique[] = [
    {
      role: "Contrarian",
      stateless: true,
      findings: [`Challenge whether '${cleanFeatures[0]}' solves the root user job without extra scope.`],
    },
    {
      role: "Edge-Case",
      stateless: true,
      findings: ["Specify empty, loading, error, offline, and mobile-360px states explicitly."],
    },
    {
      role: "Token Economist",
      stateless: true,
      findings: [`Deliver ${cleanFeatures.length} isolated 1-feature slice(s) in fresh contexts to keep active context <40%.`],
    },
  ];

  const slices: FeatureSlice[] = cleanFeatures.map((title, i) => ({
    sliceId: `SLICE-${i + 1}`,
    title,
    freshContext: true,
    acceptanceCriteria: [
      `${title} works end-to-end with verified test proof`,
      `Handles empty and error states cleanly`,
    ],
  }));

  const prdFile = path.join(workspace, "PRD.md");
  const prdMd = [
    `# Elephant PRD`,
    ``,
    `## Original Request`,
    vaguePrompt.trim(),
    ``,
    `## Stateless Goldfish Critic Review`,
    ...critiques.map((c) => `- **${c.role}**: ${c.findings.join(" ")}`),
    ``,
    `## 1-Feature Fresh-Context Slices`,
    ...slices.map((s) => `- **${s.sliceId}** (${s.title}): ${s.acceptanceCriteria.join("; ")}`),
    ``,
  ].join("\n");
  fs.writeFileSync(prdFile, prdMd, "utf8");

  return { prdFile, critiques, slices };
}

export interface AskUserQuestionForm {
  id: string;
  prompt: string;
  options: { key: string; label: string; impact: string }[];
  allowCustom: boolean;
}

export function buildAskUserQuestionForm(
  id: string,
  prompt: string,
  options: readonly { key: string; label: string; impact: string }[],
): AskUserQuestionForm {
  if (options.length < 2 || options.length > 4) {
    throw new Error("AskUserQuestion form requires 2-4 structured options");
  }
  return { id, prompt: prompt.trim(), options: [...options], allowCustom: true };
}

export interface StateMachineRefactorPlan {
  command: "/refactor-state-machines";
  axes: ("UX" | "DX" | "AX")[];
  states: string[];
  eliminatedBooleanFlags: string[];
}

export function planStateMachineRefactor(
  booleanFlags: readonly string[],
  states: readonly string[],
): StateMachineRefactorPlan {
  return {
    command: "/refactor-state-machines",
    axes: ["UX", "DX", "AX"],
    states: [...states],
    eliminatedBooleanFlags: [...booleanFlags],
  };
}

export interface TasteAuditReport {
  ok: boolean;
  checkedArtifacts: ("a11y" | "smell.md" | "typeset.md")[];
  viewportsVerified: [1440, 360];
  issues: string[];
}

/** `/taste` anti-slop UI audit (`a11y`, `smell.md`, `typeset.md`, 1440px/360px responsive checks). */
export function runTasteAntiSlopAudit(input: {
  htmlOrCss: string;
  hasSmellMd: boolean;
  hasTypesetMd: boolean;
  desktop1440OverflowPx: number;
  mobile360OverflowPx: number;
}): TasteAuditReport {
  const issues: string[] = [];
  if (!input.hasSmellMd) issues.push("Missing smell.md anti-slop checklist");
  if (!input.hasTypesetMd) issues.push("Missing typeset.md typography hierarchy");
  if (/<img(?![^>]*\balt=)[^>]*>/i.test(input.htmlOrCss)) {
    issues.push("a11y: <img> missing alt attribute");
  }
  if (/outline\s*:\s*none/i.test(input.htmlOrCss) && !/:focus-visible/i.test(input.htmlOrCss)) {
    issues.push("a11y: outline:none without :focus-visible replacement");
  }
  if (/linear-gradient\([^)]*(purple|#7c3aed|#9333ea)/i.test(input.htmlOrCss)) {
    issues.push("smell: generic AI purple-gradient slop detected");
  }
  if (input.desktop1440OverflowPx > 0) {
    issues.push(`responsive: horizontal overflow (${input.desktop1440OverflowPx}px) at 1440px desktop`);
  }
  if (input.mobile360OverflowPx > 0) {
    issues.push(`responsive: horizontal overflow (${input.mobile360OverflowPx}px) at 360px mobile`);
  }
  return {
    ok: issues.length === 0,
    checkedArtifacts: ["a11y", "smell.md", "typeset.md"],
    viewportsVerified: [1440, 360],
    issues,
  };
}