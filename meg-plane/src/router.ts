// Model router — Layer 2 control plane (MEG polyglot architecture, layer 2).
// Owns the provider ladder: local Ollama models are first-class and health-
// checked; cloud providers are listed HONESTLY as "not configured" until the
// operator adds keys (the dsh/codex honesty rule — never fake capability).

export type ProviderKind = "local" | "cloud";

export interface ModelEntry {
  id: string;
  provider: string;
  kind: ProviderKind;
  roles: string[];              // "coding" | "reasoning" | "general" | "vision"
  status: "ready" | "not-configured" | "unreachable";
  latencyMs?: number;
}

export interface RouteRequest {
  goal: string;
  role?: string;                // required capability
}

export interface RouteDecision {
  selected: ModelEntry | null;
  reason: string;
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
