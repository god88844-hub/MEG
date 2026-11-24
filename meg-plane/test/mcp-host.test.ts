import { describe, expect, test } from "bun:test";
import {
  McpTransportError,
  McpVerdictError,
  ParallelWebResearchConnector,
  buildEphemeralUvxMcpConfig,
  callMcpWithPerCallerTimeout,
  classifyMcpError,
  mintScopedActionToken,
  routeCliOrMcp,
} from "../src/mcp-host";

describe("P3-06 CLI > MCP Enforcer & Hardened MCP Host", () => {
  test("routes gh, supabase, stripe, vercel, git, curl to native CLIs and restricts default MCP to chrome-devtools, Context7, sentry", () => {
    for (const cli of ["gh", "supabase", "stripe", "vercel", "git", "curl", "gh-mcp"]) {
      const r = routeCliOrMcp(cli);
      expect(r.route).toBe("cli");
    }
    for (const allowed of ["chrome-devtools", "Context7", "sentry"]) {
      const r = routeCliOrMcp(allowed);
      expect(r.route).toBe("mcp");
    }
    expect(routeCliOrMcp("random-unapproved-mcp").route).toBe("denied");
    expect(routeCliOrMcp("custom-lab", { extraAllowedMcp: ["custom-lab"] }).route).toBe("mcp");
  });

  test("splits verdict vs transport errors, enforces per-caller timeout, mints scoped action tokens & builds zero-context uvx configs", async () => {
    expect(classifyMcpError(new Error("ECONNREFUSED 127.0.0.1"))).toBeInstanceOf(McpTransportError);
    expect(classifyMcpError(new Error("Schema mismatch: missing field id"))).toBeInstanceOf(McpVerdictError);

    await expect(
      callMcpWithPerCallerTimeout(
        () => new Promise((resolve) => setTimeout(() => resolve("late"), 500)),
        30,
      ),
    ).rejects.toBeInstanceOf(McpTransportError);

    const tok = mintScopedActionToken("seed-123", "Context7", "query-docs", 60);
    expect(tok.token.startsWith("meg_act_")).toBeTrue();
    const uvx = buildEphemeralUvxMcpConfig("mcp-server-docs", ["--readonly"], tok.token);
    expect(uvx.command).toBe("uvx");
    expect(uvx.env?.MEG_ZERO_TURN_CONTEXT).toBe("1");
    expect(uvx.env?.MEG_SCOPED_ACTION_TOKEN).toBe(tok.token);
    expect(uvx.env?.OPENAI_API_KEY).toBeUndefined();
  });

  test("ParallelWebResearchConnector fetches multiple URLs concurrently and isolates single-URL failures", async () => {
    const connector = new ParallelWebResearchConnector(async (url) => {
      if (url.includes("fail")) throw new Error("ECONNREFUSED");
      return { title: `Title for ${url}`, text: `Body of ${url}` };
    }, 1000);

    const results = await connector.fetchParallel(["[https://a.test](https://a.test)", "[https://fail.test](https://fail.test)", "[https://b.test](https://b.test)"]);
    expect(results).toHaveLength(3);
    expect(results[0]?.ok).toBeTrue();
    expect(results[1]?.ok).toBeFalse();
    expect(results[2]?.ok).toBeTrue();
  });
});