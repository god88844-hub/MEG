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