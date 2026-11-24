// P3-03: Multi-Agent Omnigraph Git-Model Memory:
// - Per-agent memory branches keyed by `@key` slugs
// - Record-level `merge_base` conflict detection (`409 divergent_update` on stale writes)
// - `--snapshot` time-travel reads
// - Reviewer-only merge policy (`HTTP 403` for worker merges to `main`)

const SLUG_RE = /^@[a-z0-9][a-z0-9._-]{0,63}$/;

export class OmnigraphError extends Error {
  constructor(
    public readonly status: 400 | 403 | 404 | 409,
    public readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "OmnigraphError";
  }
}

export interface OmnigraphRecord {
  slug: string;
  value: string;
  version: number;
  snapshotId: number;
  updatedBy: string;
}

interface BranchState {
  name: string;
  mergeBaseSnapshot: number;
  baseRecords: Map<string, OmnigraphRecord>;
  records: Map<string, OmnigraphRecord>;
}

export function validateOmnigraphSlug(slug: string): string {
  if (!SLUG_RE.test(slug)) {
    throw new OmnigraphError(
      400,
      "invalid_slug",
      `Omnigraph key must match @slug format (got '${slug}')`,
    );
  }
  return slug;
}

export class OmnigraphStore {
  private branches = new Map<string, BranchState>();
  private snapshots = new Map<number, Map<string, Map<string, OmnigraphRecord>>>();
  private snapshotSeq = 0;

  constructor() {
    this.branches.set("main", {
      name: "main",
      mergeBaseSnapshot: 0,
      baseRecords: new Map(),
      records: new Map(),
    });
    this.captureSnapshot();
  }

  get currentSnapshot(): number {
    return this.snapshotSeq;
  }

  private cloneBranchRecords(map: Map<string, OmnigraphRecord>): Map<string, OmnigraphRecord> {
    const out = new Map<string, OmnigraphRecord>();
    for (const [k, v] of map.entries()) out.set(k, { ...v });
    return out;
  }

  private captureSnapshot(): number {
    const id = ++this.snapshotSeq;
    const snap = new Map<string, Map<string, OmnigraphRecord>>();
    for (const [name, br] of this.branches.entries()) {
      snap.set(name, this.cloneBranchRecords(br.records));
    }
    this.snapshots.set(id, snap);
    return id;
  }

  createBranch(branchName: string, fromBranch = "main"): void {
    const parent = this.branches.get(fromBranch);
    if (!parent) {
      throw new OmnigraphError(404, "branch_not_found", `Unknown base branch '${fromBranch}'`);
    }
    if (this.branches.has(branchName)) {
      throw new OmnigraphError(409, "branch_exists", `Branch '${branchName}' already exists`);
    }
    this.branches.set(branchName, {
      name: branchName,
      mergeBaseSnapshot: this.snapshotSeq,
      baseRecords: this.cloneBranchRecords(parent.records),
      records: this.cloneBranchRecords(parent.records),
    });
    this.captureSnapshot();
  }

  /**
   * Record-level optimistic write:
   * Passing a stale `expectedBaseVersion` throws `409 divergent_update` (prevents lost updates).
   */
  put(
    branchName: string,
    slug: string,
    value: string,
    actor: string,
    expectedBaseVersion?: number,
  ): OmnigraphRecord {
    validateOmnigraphSlug(slug);
    const branch = this.branches.get(branchName);
    if (!branch) {
      throw new OmnigraphError(404, "branch_not_found", `Unknown branch '${branchName}'`);
    }
    const existing = branch.records.get(slug);
    const currentVersion = existing?.version ?? 0;
    if (expectedBaseVersion !== undefined && expectedBaseVersion !== currentVersion) {
      throw new OmnigraphError(
        409,
        "divergent_update",
        `409 divergent_update on ${branchName}:${slug} (expected version ${expectedBaseVersion}, live is ${currentVersion})`,
      );
    }
    const nextSnap = this.snapshotSeq + 1;
    const rec: OmnigraphRecord = {
      slug,
      value,
      version: currentVersion + 1,
      snapshotId: nextSnap,
      updatedBy: actor,
    };
    branch.records.set(slug, rec);
    this.captureSnapshot();
    return { ...rec };
  }

  /** Read a record from a branch, or pass `{ snapshot }` for `--snapshot` time-travel reads. */
  get(
    branchName: string,
    slug: string,
    options: { snapshot?: number } = {},
  ): OmnigraphRecord | null {
    validateOmnigraphSlug(slug);
    if (options.snapshot !== undefined) {
      const snap = this.snapshots.get(options.snapshot);
      const br = snap?.get(branchName);
      const rec = br?.get(slug);
      return rec ? { ...rec } : null;
    }
    const branch = this.branches.get(branchName);
    if (!branch) {
      throw new OmnigraphError(404, "branch_not_found", `Unknown branch '${branchName}'`);
    }
    const rec = branch.records.get(slug);
    return rec ? { ...rec } : null;
  }

  /**
   * Merge an agent branch into `main`:
   * - Reviewer-only policy: workers receive `HTTP 403`.
   * - Record-level 3-way `merge_base` check: rejects with `409 divergent_update` if both
   *   `main` and the agent branch modified the same `@key` to different values since `merge_base`.
   */
  mergeToMain(
    sourceBranch: string,
    actorRole: "worker" | "reviewer",
    actorName = actorRole,
  ): { mergedKeys: string[]; snapshotId: number } {
    if (actorRole !== "reviewer") {
      throw new OmnigraphError(
        403,
        "forbidden_worker_merge",
        `HTTP 403: worker role cannot merge '${sourceBranch}' to main; reviewer approval required`,
      );
    }
    if (sourceBranch === "main") {
      throw new OmnigraphError(400, "invalid_merge", "Cannot merge main into main");
    }
    const src = this.branches.get(sourceBranch);
    const main = this.branches.get("main")!;
    if (!src) {
      throw new OmnigraphError(404, "branch_not_found", `Unknown branch '${sourceBranch}'`);
    }

    const keysToApply: OmnigraphRecord[] = [];
    for (const [slug, srcRec] of src.records.entries()) {
      const baseRec = src.baseRecords.get(slug);
      const mainRec = main.records.get(slug);
      const srcChanged = !baseRec || baseRec.value !== srcRec.value;
      if (!srcChanged) continue;

      const mainChanged =
        (baseRec === undefined && mainRec !== undefined) ||
        (baseRec !== undefined && mainRec !== undefined && mainRec.value !== baseRec.value);

      if (mainChanged && mainRec && mainRec.value !== srcRec.value) {
        throw new OmnigraphError(
          409,
          "divergent_update",
          `409 divergent_update merging '${sourceBranch}' -> main on '${slug}': main and branch both changed since merge_base`,
        );
      }
      keysToApply.push(srcRec);
    }

    const mergedKeys: string[] = [];
    for (const rec of keysToApply) {
      const prevVer = main.records.get(rec.slug)?.version ?? 0;
      main.records.set(rec.slug, {
        slug: rec.slug,
        value: rec.value,
        version: prevVer + 1,
        snapshotId: this.snapshotSeq + 1,
        updatedBy: actorName,
      });
      mergedKeys.push(rec.slug);
    }
    src.baseRecords = this.cloneBranchRecords(main.records);
    const snapshotId = this.captureSnapshot();
    return { mergedKeys, snapshotId };
  }
}