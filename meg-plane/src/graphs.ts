// P5-01: Algebraic Orchestration (Maestro Order) & Code-Routed DAG:
// - Single-agent default; Early-Start independence test required before fan-out
// - 4 combinators: sequential, parallel, verify, recurse
// - Blackboard (logodds, calls, coverage, ABSTAIN)
// - Streaming pipeline() preferred over blocking parallel() barriers
// - Plain code (if/switch) routing typed node contracts
// - LIFO Saga rollback stack + partitioned parent budgets

export interface SliceDescriptor {
  id: string;
  writePaths: string[];
  dependsOn?: string[];
}

export interface EarlyStartVerdict {
  mode: "single_agent" | "fan_out";
  independent: boolean;
  conflicts: string[];
  reason: string;
}

/**
 * Single-agent default: fan-out is allowed ONLY when >=2 slices pass the
 * Early-Start independence test (zero overlapping writePaths & zero inter-slice dependencies).
 */
export function evaluateEarlyStartIndependence(slices: readonly SliceDescriptor[]): EarlyStartVerdict {
  if (slices.length <= 1) {
    return {
      mode: "single_agent",
      independent: false,
      conflicts: [],
      reason: "Single-agent default: <=1 slice does not justify multi-agent fan-out",
    };
  }
  const conflicts: string[] = [];
  const sliceIds = new Set(slices.map((s) => s.id));
  const ownerByPath = new Map<string, string>();

  for (const slice of slices) {
    for (const dep of slice.dependsOn ?? []) {
      if (sliceIds.has(dep)) {
        conflicts.push(`dependency:${slice.id}->${dep}`);
      }
    }
    for (const rawPath of slice.writePaths) {
      const norm = rawPath.replace(/\\/g, "/");
      const prev = ownerByPath.get(norm);
      if (prev && prev !== slice.id) {
        conflicts.push(`write_overlap:${norm} (${prev} vs ${slice.id})`);
      } else {
        ownerByPath.set(norm, slice.id);
      }
    }
  }

  if (conflicts.length > 0) {
    return {
      mode: "single_agent",
      independent: false,
      conflicts,
      reason: `Early-Start independence failed (${conflicts.length} conflict(s)); staying single-agent`,
    };
  }
  return {
    mode: "fan_out",
    independent: true,
    conflicts: [],
    reason: `Early-Start independence passed across ${slices.length} disjoint slices`,
  };
}

export interface BlackboardState {
  logodds: number;
  calls: number;
  coverage: number;
  decision: "PROCEED" | "REJECT" | "ABSTAIN";
  notes: string[];
}

/** Probabilistic Blackboard tracking evidence logodds, calls, coverage, and ABSTAIN threshold. */
export class Blackboard {
  logodds = 0;
  calls = 0;
  private coveredItems = new Set<string>();
  private notes: string[] = [];

  constructor(
    private totalItems: number,
    private minCoverage = 0.7,
    private abstainMargin = 0.5,
  ) {}

  recordEvidence(input: { deltaLogodds: number; coveredKeys?: string[]; note?: string }): void {
    this.calls += 1;
    this.logodds += input.deltaLogodds;
    for (const k of input.coveredKeys ?? []) this.coveredItems.add(k);
    if (input.note) this.notes.push(input.note);
  }

  get coverage(): number {
    if (this.totalItems <= 0) return 1;
    return Math.min(1, this.coveredItems.size / this.totalItems);
  }

  evaluate(): BlackboardState {
    let decision: BlackboardState["decision"];
    if (this.coverage < this.minCoverage || Math.abs(this.logodds) < this.abstainMargin) {
      decision = "ABSTAIN";
    } else {
      decision = this.logodds >= 0 ? "PROCEED" : "REJECT";
    }
    return {
      logodds: Math.round(this.logodds * 1000) / 1000,
      calls: this.calls,
      coverage: this.coverage,
      decision,
      notes: [...this.notes],
    };
  }
}

export type GraphStep<T> = (state: T) => Promise<T> | T;

/** Combinator 1: Sequential composition. */
export async function sequential<T>(initial: T, steps: readonly GraphStep<T>[]): Promise<T> {
  let cur = initial;
  for (const step of steps) {
    cur = await step(cur);
  }
  return cur;
}

/** Combinator 2: Parallel fan-out with explicit reducer (use only after Early-Start passes). */
export async function parallel<T, R>(
  items: readonly T[],
  worker: (item: T, index: number) => Promise<R> | R,
): Promise<R[]> {
  return Promise.all(items.map((item, idx) => Promise.resolve(worker(item, idx))));
}

/** Combinator 3: Verify wrapper enforcing postcondition before returning. */
export async function verify<T>(
  producer: () => Promise<T> | T,
  predicate: (value: T) => Promise<boolean> | boolean,
  onFailureMessage = "Verify combinator postcondition failed",
): Promise<T> {
  const value = await producer();
  const ok = await predicate(value);
  if (!ok) throw new Error(onFailureMessage);
  return value;
}

/** Combinator 4: Bounded Recurse combinator (terminates on `done` or `maxDepth`). */
export async function recurse<T>(
  initial: T,
  step: (state: T, depth: number) => Promise<T> | T,
  done: (state: T, depth: number) => boolean,
  maxDepth = 5,
): Promise<{ state: T; iterations: number; converged: boolean }> {
  let cur = initial;
  let depth = 0;
  while (depth < maxDepth && !done(cur, depth)) {
    depth += 1;
    cur = await step(cur, depth);
  }
  return { state: cur, iterations: depth, converged: done(cur, depth) };
}

/**
 * Streaming `pipeline()` (default over blocking `parallel()` barriers):
 * yields completed items incrementally through stages as soon as each item finishes.
 */
export async function* pipeline<T>(
  source: Iterable<T> | AsyncIterable<T>,
  stages: readonly ((item: T) => Promise<T> | T)[],
): AsyncGenerator<T> {
  for await (const raw of source) {
    let item = raw;
    for (const stage of stages) {
      item = await stage(item);
    }
    yield item;
  }
}

export type TypedNodeContract =
  | { kind: "plan"; goal: string }
  | { kind: "build"; files: string[] }
  | { kind: "verify"; passed: boolean; failedTests: string[] }
  | { kind: "blocked"; reason: string };

/** Plain code (`switch`) routing typed node contracts with zero hidden LLM routing magic. */
export function routeNodeContract(contract: TypedNodeContract): "planner" | "builder" | "verifier" | "release" | "escalate" {
  switch (contract.kind) {
    case "plan":
      return "planner";
    case "build":
      return contract.files.length > 0 ? "verifier" : "builder";
    case "verify":
      return contract.passed ? "release" : "builder";
    case "blocked":
      return "escalate";
  }
}

/** LIFO Saga rollback stack executing compensating actions in strict reverse order. */
export class SagaStack {
  private compensations: { name: string; undo: () => Promise<void> | void }[] = [];

  push(name: string, undo: () => Promise<void> | void): void {
    this.compensations.push({ name, undo });
  }

  get size(): number {
    return this.compensations.length;
  }

  async rollbackAll(): Promise<string[]> {
    const rolledBack: string[] = [];
    while (this.compensations.length > 0) {
      const entry = this.compensations.pop()!;
      await entry.undo();
      rolledBack.push(entry.name);
    }
    return rolledBack;
  }
}

export interface BudgetEnvelope {
  maxTurns: number;
  maxTokens: number;
  maxUsd: number;
}

/** Partition parent budget across N children so sum(children) <= parent strictly holds. */
export function partitionParentBudget(
  parent: BudgetEnvelope,
  weights: readonly number[],
): BudgetEnvelope[] {
  if (weights.length === 0) return [];
  const totalWeight = weights.reduce((acc, w) => acc + Math.max(0, w), 0);
  if (totalWeight <= 0) throw new Error("Budget partition requires positive weights");

  let remTurns = parent.maxTurns;
  let remTokens = parent.maxTokens;
  let remUsd = Math.round(parent.maxUsd * 10_000); // tenths of a mill for integer precision

  return weights.map((w, idx) => {
    const share = Math.max(0, w) / totalWeight;
    const isLast = idx === weights.length - 1;
    const maxTurns = isLast ? remTurns : Math.min(remTurns, Math.floor(parent.maxTurns * share));
    const maxTokens = isLast ? remTokens : Math.min(remTokens, Math.floor(parent.maxTokens * share));
    const usdScaled = isLast ? remUsd : Math.min(remUsd, Math.floor(Math.round(parent.maxUsd * 10_000) * share));
    remTurns -= maxTurns;
    remTokens -= maxTokens;
    remUsd -= usdScaled;
    return {
      maxTurns,
      maxTokens,
      maxUsd: usdScaled / 10_000,
    };
  });
}