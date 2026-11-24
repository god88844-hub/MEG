// P3-04: 8-Layer SKILL.md Loader & Skill++ Contrastive Index:
// - 3-level progressive disclosure: Metadata (~100 tok) -> Body (<500 lines) -> references/ assets/ scripts/
// - Active load cap: <=3 skills (<=1 universal + <=2 specific), <=1,600 tokens, >0.80 similarity dedupe
// - Tool cap post-refresh: <=6 -> 4 tools
// - Skill++ flat contrastive index (Supports | Differs | Excludes)
// - Path-scoped .meg/rules/*.md (paths: YAML frontmatter)
// - USC duplicate-skill merger
// - Data Forge manifest.yaml multi-client packager (--check CI drift validation)

import fs from "node:fs";
import path from "node:path";
import { scanSkillSupplyChain } from "./security-router";

export const SKILL_CAPS = {
  MAX_METADATA_TOKENS: 100,
  MAX_BODY_LINES: 500,
  MAX_ACTIVE_SKILLS: 3,
  MAX_UNIVERSAL_SKILLS: 1,
  MAX_SPECIFIC_SKILLS: 2,
  MAX_ACTIVE_TOKENS: 1_600,
  SIMILARITY_DEDUPE_THRESHOLD: 0.80,
  MAX_TOOLS_BASE: 6,
  MAX_TOOLS_POST_REFRESH: 4,
} as const;

export type SkillScope = "universal" | "specific";

export interface SkillMetadata {
  name: string;
  description: string;
  scope: SkillScope;
  supports: string[];
  differs: string[];
  excludes: string[];
  estimatedTokens: number;
  dirPath?: string;
}

export interface LoadedSkill {
  metadata: SkillMetadata;
  body: string;
  bodyLines: number;
  assets: {
    references: string[];
    assets: string[];
    scripts: string[];
  };
}

export function estimateTokens(text: string): number {
  return Math.ceil(text.trim().length / 4);
}

function tokenizeForSimilarity(text: string): Set<string> {
  return new Set(
    text
      .toLowerCase()
      .split(/[^a-z0-9_]+/)
      .filter((t) => t.length >= 2),
  );
}

/** Jaccard token similarity in [0, 1] for >0.80 duplicate detection & USC merging. */
export function skillSimilarity(a: string, b: string): number {
  const sa = tokenizeForSimilarity(a);
  const sb = tokenizeForSimilarity(b);
  if (sa.size === 0 && sb.size === 0) return 1;
  if (sa.size === 0 || sb.size === 0) return 0;
  let inter = 0;
  for (const tok of sa) {
    if (sb.has(tok)) inter++;
  }
  const union = sa.size + sb.size - inter;
  return union === 0 ? 0 : inter / union;
}

function parseFrontmatter(raw: string): { fm: Record<string, any>; body: string } {
  const m = raw.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
  if (!m) return { fm: {}, body: raw };
  const parsed = Bun.YAML.parse(m[1]!) as Record<string, any>;
  return { fm: parsed && typeof parsed === "object" ? parsed : {}, body: m[2] ?? "" };
}

function listSubdir(baseDir: string, sub: string): string[] {
  const target = path.join(baseDir, sub);
  if (!fs.existsSync(target) || !fs.statSync(target).isDirectory()) return [];
  return fs
    .readdirSync(target, { withFileTypes: true })
    .filter((e) => e.isFile())
    .map((e) => `${sub}/${e.name}`)
    .sort();
}

/**
 * Load a single `SKILL.md` with DDIPE supply-chain enforcement and 3-level disclosure validation.
 */
export function loadSkillFromDir(skillDir: string, options: { thirdParty?: boolean } = {}): LoadedSkill {
  const skillFile = path.join(skillDir, "SKILL.md");
  if (!fs.existsSync(skillFile)) {
    throw new Error(`SKILL.md not found in ${skillDir}`);
  }
  const raw = fs.readFileSync(skillFile, "utf8");
  if (options.thirdParty !== false) {
    const scan = scanSkillSupplyChain(raw, skillFile);
    if (!scan.safe) {
      throw new Error(
        `DDIPE_BLOCKED: refused to load poisoned skill '${skillFile}' (${scan.findings.map((f) => f.rule).join(", ")})`,
      );
    }
  }

  const { fm, body } = parseFrontmatter(raw);
  const name = String(fm.name ?? path.basename(skillDir)).trim();
  const description = String(fm.description ?? "").trim();
  const scope: SkillScope = fm.scope === "universal" ? "universal" : "specific";
  const supports = Array.isArray(fm.supports) ? fm.supports.map(String) : [];
  const differs = Array.isArray(fm.differs) ? fm.differs.map(String) : [];
  const excludes = Array.isArray(fm.excludes) ? fm.excludes.map(String) : [];

  const bodyLineArr = body.split(/\r?\n/);
  const trimmedBody =
    bodyLineArr.length > SKILL_CAPS.MAX_BODY_LINES
      ? bodyLineArr.slice(0, SKILL_CAPS.MAX_BODY_LINES).join("\n")
      : body;

  const metaLine = `${name}: ${description}`;
  const clippedDesc =
    estimateTokens(metaLine) > SKILL_CAPS.MAX_METADATA_TOKENS
      ? description.slice(0, SKILL_CAPS.MAX_METADATA_TOKENS * 4 - name.length - 4)
      : description;

  return {
    metadata: {
      name,
      description: clippedDesc,
      scope,
      supports,
      differs,
      excludes,
      estimatedTokens: estimateTokens(trimmedBody),
      dirPath: skillDir,
    },
    body: trimmedBody,
    bodyLines: Math.min(bodyLineArr.length, SKILL_CAPS.MAX_BODY_LINES),
    assets: {
      references: listSubdir(skillDir, "references"),
      assets: listSubdir(skillDir, "assets"),
      scripts: listSubdir(skillDir, "scripts"),
    },
  };
}

/**
 * Progressive-disclosure active selector:
 * - Caps active load at <=3 skills (<=1 universal + <=2 specific)
 * - Caps combined body tokens at <=1,600 tokens
 * - Deduplicates skills with >0.80 similarity
 */
export function selectActiveSkills(
  candidates: readonly LoadedSkill[],
  query: string,
): { selected: LoadedSkill[]; droppedReasons: Record<string, string> } {
  const queryTerms = tokenizeForSimilarity(query);
  const ranked = [...candidates]
    .map((skill) => {
      const hay = `${skill.metadata.name} ${skill.metadata.description} ${skill.metadata.supports.join(" ")}`.toLowerCase();
      let score = skill.metadata.scope === "universal" ? 1 : 0;
      for (const term of queryTerms) {
        if (hay.includes(term)) score += 2;
      }
      for (const ex of skill.metadata.excludes) {
        if (query.toLowerCase().includes(ex.toLowerCase())) score -= 5;
      }
      return { skill, score };
    })
    .sort((a, b) => b.score - a.score || a.skill.metadata.name.localeCompare(b.skill.metadata.name));

  const selected: LoadedSkill[] = [];
  const droppedReasons: Record<string, string> = {};
  let universalCount = 0;
  let specificCount = 0;
  let totalTokens = 0;

  for (const { skill, score } of ranked) {
    const name = skill.metadata.name;
    if (score < 0) {
      droppedReasons[name] = "excluded_by_contrastive_rule";
      continue;
    }
    if (selected.length >= SKILL_CAPS.MAX_ACTIVE_SKILLS) {
      droppedReasons[name] = "active_skill_cap_3";
      continue;
    }
    if (skill.metadata.scope === "universal" && universalCount >= SKILL_CAPS.MAX_UNIVERSAL_SKILLS) {
      droppedReasons[name] = "universal_cap_1";
      continue;
    }
    if (skill.metadata.scope === "specific" && specificCount >= SKILL_CAPS.MAX_SPECIFIC_SKILLS) {
      droppedReasons[name] = "specific_cap_2";
      continue;
    }
    const dup = selected.find(
      (s) =>
        skillSimilarity(
          `${s.metadata.description} ${s.body}`,
          `${skill.metadata.description} ${skill.body}`,
        ) > SKILL_CAPS.SIMILARITY_DEDUPE_THRESHOLD,
    );
    if (dup) {
      droppedReasons[name] = `duplicate_similarity_gt_0.80:${dup.metadata.name}`;
      continue;
    }
    if (totalTokens + skill.metadata.estimatedTokens > SKILL_CAPS.MAX_ACTIVE_TOKENS) {
      droppedReasons[name] = "token_budget_1600_exceeded";
      continue;
    }
    selected.push(skill);
    totalTokens += skill.metadata.estimatedTokens;
    if (skill.metadata.scope === "universal") universalCount++;
    else specificCount++;
  }

  return { selected, droppedReasons };
}

/** Cap active tool menu to <=6 initially and <=4 after mid-run refresh. */
export function capToolsForStage(tools: readonly string[], postRefresh: boolean): string[] {
  const limit = postRefresh ? SKILL_CAPS.MAX_TOOLS_POST_REFRESH : SKILL_CAPS.MAX_TOOLS_BASE;
  return tools.slice(0, limit);
}

/** Skill++ flat contrastive index (`Supports | Differs | Excludes`). */
export function renderSkillPlusPlusIndex(skills: readonly LoadedSkill[]): string {
  const header = "| Skill | Scope | Supports | Differs | Excludes |\n| :--- | :--- | :--- | :--- | :--- |";
  const rows = [...skills]
    .sort((a, b) => a.metadata.name.localeCompare(b.metadata.name))
    .map((s) => {
      const m = s.metadata;
      const fmt = (arr: string[]) => (arr.length ? arr.join(", ") : "-");
      return `| ${m.name} | ${m.scope} | ${fmt(m.supports)} | ${fmt(m.differs)} | ${fmt(m.excludes)} |`;
    });
  return [header, ...rows].join("\n");
}

/** USC duplicate-skill merger: collapses near-identical skills (>0.80 similarity) into one canonical skill. */
export function mergeDuplicateSkills(skills: readonly LoadedSkill[]): LoadedSkill[] {
  const merged: LoadedSkill[] = [];
  for (const cand of skills) {
    const existing = merged.find(
      (m) =>
        skillSimilarity(
          `${m.metadata.description} ${m.body}`,
          `${cand.metadata.description} ${cand.body}`,
        ) > SKILL_CAPS.SIMILARITY_DEDUPE_THRESHOLD,
    );
    if (!existing) {
      merged.push({
        ...cand,
        metadata: {
          ...cand.metadata,
          supports: [...cand.metadata.supports],
          differs: [...cand.metadata.differs],
          excludes: [...cand.metadata.excludes],
        },
      });
      continue;
    }
    // Union contrastive tags and keep the richer body (within <500 line cap).
    existing.metadata.supports = [...new Set([...existing.metadata.supports, ...cand.metadata.supports])];
    existing.metadata.differs = [...new Set([...existing.metadata.differs, ...cand.metadata.differs])];
    existing.metadata.excludes = [...new Set([...existing.metadata.excludes, ...cand.metadata.excludes])];
    if (cand.body.length > existing.body.length) {
      existing.body = cand.body;
      existing.bodyLines = cand.bodyLines;
      existing.metadata.estimatedTokens = cand.metadata.estimatedTokens;
    }
  }
  return merged;
}

export interface PathScopedRule {
  file: string;
  paths: string[];
  body: string;
}

function matchPathGlob(pattern: string, targetPath: string): boolean {
  const norm = targetPath.replace(/\\/g, "/");
  const normPat = pattern.replace(/\\/g, "/");
  let re = "^";
  for (let i = 0; i < normPat.length; i++) {
    if (normPat[i] === "*" && normPat[i + 1] === "*") {
      if (normPat[i + 2] === "/") {
        re += "(?:.*/)?";
        i += 2;
      } else {
        re += ".*";
        i += 1;
      }
    } else if (normPat[i] === "*") {
      re += "[^/]*";
    } else if ("\\.^$+?|()[]{}".includes(normPat[i]!)) {
      re += "\\" + normPat[i]!;
    } else {
      re += normPat[i]!;
    }
  }
  re += "$";
  return new RegExp(re).test(norm);
}

/** Load path-scoped `.meg/rules/*.md` rules whose `paths:` YAML globs match `activeFilePath`. */
export function loadPathScopedRules(workspace: string, activeFilePath: string): PathScopedRule[] {
  const rulesDir = path.join(workspace, ".meg", "rules");
  if (!fs.existsSync(rulesDir)) return [];
  const matched: PathScopedRule[] = [];
  for (const entry of fs.readdirSync(rulesDir, { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith(".md")) continue;
    const raw = fs.readFileSync(path.join(rulesDir, entry.name), "utf8");
    const { fm, body } = parseFrontmatter(raw);
    const paths = Array.isArray(fm.paths) ? fm.paths.map(String) : [];
    if (paths.length === 0 || paths.some((p) => matchPathGlob(p, activeFilePath))) {
      matched.push({ file: entry.name, paths, body: body.trim() });
    }
  }
  return matched;
}

export interface DataForgeManifest {
  version: number;
  clients: string[];
  skills: { name: string; path: string }[];
}

export interface DataForgePackageResult {
  ok: boolean;
  clients: string[];
  skillCount: number;
  errors: string[];
  writtenFiles: string[];
}

/**
 * Data Forge `manifest.yaml` multi-client packager (`--check` mode validates without writing).
 */
export function packageDataForgeManifest(
  workspace: string,
  options: { checkOnly?: boolean; outDir?: string } = {},
): DataForgePackageResult {
  const manifestFile = path.join(workspace, "manifest.yaml");
  if (!fs.existsSync(manifestFile)) {
    return { ok: false, clients: [], skillCount: 0, errors: ["manifest.yaml not found"], writtenFiles: [] };
  }
  const manifest = Bun.YAML.parse(fs.readFileSync(manifestFile, "utf8")) as DataForgeManifest;
  const errors: string[] = [];
  const loaded: LoadedSkill[] = [];

  if (!manifest || !Array.isArray(manifest.clients) || !Array.isArray(manifest.skills)) {
    return {
      ok: false,
      clients: [],
      skillCount: 0,
      errors: ["manifest.yaml must declare clients[] and skills[]"],
      writtenFiles: [],
    };
  }

  for (const item of manifest.skills) {
    const dir = path.resolve(workspace, item.path);
    try {
      loaded.push(loadSkillFromDir(dir, { thirdParty: true }));
    } catch (e) {
      errors.push(`${item.name}: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  const indexMd = renderSkillPlusPlusIndex(loaded);
  const outRoot = options.outDir ?? path.join(workspace, ".meg", "dist-clients");
  const writtenFiles: string[] = [];

  if (options.checkOnly) {
    for (const client of manifest.clients) {
      const target = path.join(outRoot, client, "SKILLS_INDEX.md");
      if (!fs.existsSync(target)) {
        errors.push(`--check drift: missing packaged artifact for client '${client}'`);
      } else if (fs.readFileSync(target, "utf8") !== indexMd) {
        errors.push(`--check drift: packaged index for client '${client}' is out of sync`);
      }
    }
  } else if (errors.length === 0) {
    for (const client of manifest.clients) {
      const target = path.join(outRoot, client, "SKILLS_INDEX.md");
      fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.writeFileSync(target, indexMd, "utf8");
      writtenFiles.push(target);
    }
  }

  return {
    ok: errors.length === 0,
    clients: manifest.clients,
    skillCount: loaded.length,
    errors,
    writtenFiles,
  };
}