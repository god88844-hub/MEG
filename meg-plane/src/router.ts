// Model router -- Layer 2 control plane (MEG polyglot architecture, layer 2).
// Owns the provider ladder: local Ollama models are first-class and health-
// checked; cloud providers are listed HONESTLY as "not-configured" until the
// operator adds keys (the dsh/codex honesty rule -- never fake capability).

import fs from "node:fs";
import path from "node:path";

export type ProviderKind = "local" | "cloud";

export interface ModelEntry {
  id: string;
  provider: string;
  kind: ProviderKind;
  roles: string[];              // "coding" | "reasoning" | "general" | "vision"
  status: "ready" | "not-configured" | "unreachable";
  latencyMs?: number;
}

export interface StoredProviderProfile {
  id: string;
  name?: string;
  kind?: string;
  base_url: string;
  default_model?: string;
  key_env?: string | null;
  has_key?: boolean;
}

export interface RouteRequest {
  goal: string;
  role?: string;                // required capability
}

export interface RouteDecision {
  selected: ModelEntry | null;
  reason: string;
}

const PRESET_KEY_ENV: Record<string, string> = {
  openrouter: "OPENROUTER_API_KEY",
  openai: "OPENAI_API_KEY",
  anthropic: "ANTHROPIC_API_KEY",
  nvidia: "NVIDIA_API_KEY",
  google: "GEMINI_API_KEY",
  groq: "GROQ_API_KEY",
  deepseek: "DEEPSEEK_API_KEY",
  mistral: "MISTRAL_API_KEY",
  xai: "XAI_API_KEY",
  fireworks: "FIREWORKS_API_KEY",
  siliconflow: "SILICONFLOW_API_KEY",
  nebius: "NEBIUS_API_KEY",
};

function isLoopbackUrl(baseUrl: string): boolean {
  try {
    const u = new URL(baseUrl);
    return ["localhost", "127.0.0.1", "::1", "[::1]"].includes(u.hostname);
  } catch {
    return false;
  }
}

export class ModelRouter {
  private models: ModelEntry[] = [];

  register(entry: ModelEntry): void {
    this.models = this.models.filter((m) => m.id !== entry.id);
    this.models.push(entry);
  }

  fleet(): readonly ModelEntry[] {
    return this.models;
  }

  /**
   * Sync configured ProviderStore profiles into the router fleet honestly:
   * - Cloud profiles are "ready" ONLY if a default_model AND verified key exist.
   * - Local profiles keep an existing probed "ready" status; unprobed local
   *   profiles are marked "unreachable" (or "not-configured" if no model) so
   *   availability is never faked.
   */
  syncProfiles(
    profiles: readonly StoredProviderProfile[],
    env: Record<string, string | undefined> = process.env,
  ): readonly ModelEntry[] {
    for (const profile of profiles) {
      if (!profile || typeof profile.id !== "string" || typeof profile.base_url !== "string") continue;
      const local = isLoopbackUrl(profile.base_url);
      const kind: ProviderKind = local ? "local" : "cloud";
      const modelName = (profile.default_model ?? "").trim();
      const id = modelName || `${profile.id}/default`;
      const existing = this.models.find((m) => m.id === id && m.provider === profile.id);

      let status: ModelEntry["status"];
      if (local) {
        if (existing?.status === "ready") status = "ready";
        else status = modelName ? "unreachable" : "not-configured";
      } else {
        const envVar = profile.key_env ?? PRESET_KEY_ENV[profile.id.toLowerCase()];
        const hasKey = Boolean(profile.has_key || (envVar && env[envVar]?.trim()));
        status = hasKey && Boolean(modelName) ? "ready" : "not-configured";
      }

      this.register({
        id,
        provider: profile.id,
        kind,
        roles: inferRolesFor(id),
        status,
      });
    }
    return this.fleet();
  }

  /** Read `<configDir>/providers.json` (same schema as Rust ProviderStore) and sync. */
  syncFromConfigDir(
    configDir: string,
    env: Record<string, string | undefined> = process.env,
  ): readonly ModelEntry[] {
    const file = path.join(configDir, "providers.json");
    if (!fs.existsSync(file)) return this.fleet();
    if (fs.statSync(file).size > 256 * 1024) {
      throw new Error("Provider settings exceed 256 KiB");
    }
    const parsed = JSON.parse(fs.readFileSync(file, "utf8")) as {
      profiles?: StoredProviderProfile[];
    };
    if (Array.isArray(parsed.profiles)) {
      this.syncProfiles(parsed.profiles, env);
    }
    return this.fleet();
  }

  /** Route by role, preferring ready local models, then ready cloud. */
  route(req: RouteRequest): RouteDecision {
    const role = req.role ?? inferRole(req.goal);
    const ready = this.models.filter(
      (m) => m.status === "ready" && (m.roles.length === 0 || m.roles.includes(role)),
    );
    const local = ready.find((m) => m.kind === "local");
    if (local) return { selected: local, reason: `local ${role} model` };
    const cloud = ready.find((m) => m.kind === "cloud");
    if (cloud) return { selected: cloud, reason: `cloud fallback for ${role}` };
    const anyReady = this.models.find((m) => m.status === "ready");
    if (anyReady) {
      return { selected: anyReady, reason: `no ${role}-specialized model ready; using ${anyReady.id}` };
    }
    return { selected: null, reason: "no ready model in the fleet" };
  }

  /** Health probe: Ollama /api/tags for local; cloud stays not-configured
   *  unless an API key exists (never faked). */
  static async probeLocal(baseUrl = "http://localhost:11434"): Promise<ModelEntry[]> {
    const res = await fetch(`${baseUrl}/api/tags`, {
      signal: AbortSignal.timeout(4000),
    });
    if (!res.ok) throw new Error(`ollama ${res.status}`);
    const data = (await res.json()) as { models?: { name: string }[] };
    return (data.models ?? []).map((m) => ({
      id: m.name,
      provider: "ollama",
      kind: "local" as const,
      roles: inferRolesFor(m.name),
      status: "ready" as const,
    }));
  }

  static cloudLadder(): ModelEntry[] {
    // Honest by default: not configured until the operator stores keys.
    return [
      { id: "openai/gpt", provider: "openai", kind: "cloud", roles: ["reasoning"], status: "not-configured" },
      { id: "anthropic/claude", provider: "anthropic", kind: "cloud", roles: ["coding"], status: "not-configured" },
      { id: "deepseek/deepseek", provider: "deepseek", kind: "cloud", roles: ["coding"], status: "not-configured" },
    ];
  }
}

function inferRole(goal: string): string {
  const g = goal.toLowerCase();
  if (/\b(vision|image|screenshot|photo)\b/.test(g)) return "vision";
  if (/\b(prove|derive|math|reason|logic)\b/.test(g)) return "reasoning";
  return "coding";
}

function inferRolesFor(model: string): string[] {
  const m = model.toLowerCase();
  if (m.includes("coder")) return ["coding"];
  if (m.includes("moondream") || m.includes("vision") || m.includes("llava")) return ["vision"];
  if (m.includes("reason") || m.includes("r1")) return ["reasoning"];
  return ["general", "coding"];
}
// ===========================================================================
// P2-04: Universal Model Adapter, Governance Locks & Thinking Preservation
// ===========================================================================

export type ModelFamily = "openai" | "anthropic" | "google" | "openrouter" | "local" | "other";

export interface ModelCapabilities {
  tools: boolean;
  vision: boolean;
  contextWindow: number;
  reasoningLevels: ("none" | "low" | "medium" | "high")[];
  zdrNoLog: boolean;
}

export interface GovernancePolicy {
  disabledProviders?: string[];
  pinnedModel?: string;
  requireZdr?: boolean;
}

export interface NormalizedMessage {
  role: "system" | "user" | "assistant" | "tool";
  content: string;
  thinking?: string;
  sourceFamily?: ModelFamily;
}

export interface NormalizedModelClient {
  provider: string;
  family: ModelFamily;
  model: string;
  baseUrl: string;
  capabilities: ModelCapabilities;
  headers: Record<string, string>;
  reasoningDirective?: string;
}

export function detectModelFamily(provider: string, model: string): ModelFamily {
  const p = provider.toLowerCase();
  const m = model.toLowerCase();
  if (p === "anthropic" || m.includes("claude")) return "anthropic";
  if (p === "openai" || p === "codex" || m.startsWith("gpt-") || m.startsWith("o1") || m.startsWith("o3")) return "openai";
  if (p === "google" || p === "gemini" || m.includes("gemini")) return "google";
  if (p === "openrouter") return "openrouter";
  if (p === "ollama" || p === "vllm" || p === "local" || m.includes(":7b") || m.includes(":14b") || m.includes(":32b")) return "local";
  return "other";
}

export function probeModelCapabilities(
  provider: string,
  model: string,
  zdrNoLog = false,
): ModelCapabilities {
  const family = detectModelFamily(provider, model);
  const m = model.toLowerCase();
  const vision =
    m.includes("vision") ||
    m.includes("vl") ||
    m.includes("moondream") ||
    m.includes("llava") ||
    m.includes("gpt-4o") ||
    m.includes("claude-3") ||
    m.includes("claude-sonnet") ||
    m.includes("claude-opus") ||
    m.includes("gemini");
  const contextWindow =
    family === "google"
      ? 1_000_000
      : family === "anthropic"
        ? 200_000
        : family === "openai"
          ? 128_000
          : family === "local"
            ? 32_768
            : 128_000;
  const reasoningLevels: ModelCapabilities["reasoningLevels"] =
    family === "anthropic" || family === "openai" || family === "google" || m.includes("r1") || m.includes("qwen")
      ? ["low", "medium", "high"]
      : ["none"];
  return {
    tools: true,
    vision,
    contextWindow,
    reasoningLevels,
    zdrNoLog: zdrNoLog || family === "local",
  };
}

/**
 * Normalize OpenAI/Codex, Anthropic, Google, OpenRouter, and local Ollama/vLLM
 * profiles behind one `NormalizedModelClient` while enforcing:
 * - `--pinned-model` single-model lock
 * - `disabledProviders` checked AFTER `--model` pin resolution (oh-my-pi #13079)
 * - ZDR (`noLog`) header propagation when enabled
 * - `verified` reasoning directive for local/mid-tier models
 */
export function createNormalizedModelClient(
  profile: StoredProviderProfile,
  requestedModel: string | undefined,
  policy: GovernancePolicy = {},
): NormalizedModelClient {
  const candidateModel = (requestedModel ?? profile.default_model ?? "").trim();
  const effectiveModel = policy.pinnedModel?.trim() || candidateModel;
  if (policy.pinnedModel && candidateModel && candidateModel !== policy.pinnedModel) {
    throw new Error(
      `Pinned model lock active: cannot switch from '${policy.pinnedModel}' to '${candidateModel}'`,
    );
  }
  if (!effectiveModel) {
    throw new Error(`Provider '${profile.id}' has no resolved model`);
  }

  // Enforce disabledProviders AFTER pin resolution (oh-my-pi #13079).
  const inferredProvider = effectiveModel.includes("/")
    ? effectiveModel.split("/")[0]!.toLowerCase()
    : profile.id.toLowerCase();
  const disabled = new Set((policy.disabledProviders ?? []).map((d) => d.toLowerCase()));
  if (disabled.has(profile.id.toLowerCase()) || disabled.has(inferredProvider)) {
    throw new Error(
      `Provider '${profile.id}' (resolved '${inferredProvider}') is disabled by governance policy`,
    );
  }

  const family = detectModelFamily(profile.id, effectiveModel);
  const capabilities = probeModelCapabilities(profile.id, effectiveModel, Boolean(policy.requireZdr));
  if (policy.requireZdr && !capabilities.zdrNoLog) {
    throw new Error(`Provider '${profile.id}' does not satisfy ZDR noLog policy`);
  }

  const headers: Record<string, string> = {};
  if (capabilities.zdrNoLog && family === "openrouter") {
    headers["X-OpenRouter-ZDR"] = "true";
  }
  if (family === "anthropic") {
    headers["anthropic-version"] = "2023-06-01";
  }

  const isLocalOrMid =
    family === "local" ||
    /\b(7b|8b|14b|20b|24b|27b|30b|32b|35b|mini|flash|haiku)\b/i.test(effectiveModel);

  return {
    provider: profile.id,
    family,
    model: effectiveModel,
    baseUrl: profile.base_url.replace(/\/+$/, ""),
    capabilities,
    headers,
    reasoningDirective: isLocalOrMid
      ? "verified: state assumptions, verify with tool output before claiming success"
      : undefined,
  };
}

/**
 * Provider-aware Thinking Preservation:
 * - Preserves thinking tokens inside the same model family thread.
 * - Strips thinking tokens on cross-family handoff to avoid format/signature errors.
 */
export function prepareMessagesForFamily(
  messages: readonly NormalizedMessage[],
  targetFamily: ModelFamily,
): NormalizedMessage[] {
  return messages.map((msg) => {
    if (!msg.thinking) return { ...msg };
    if (msg.sourceFamily === targetFamily) {
      return { ...msg };
    }
    const { thinking: _dropped, ...rest } = msg;
    return rest;
  });
}