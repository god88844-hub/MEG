import { describe, expect, test } from "bun:test";
import {
  createNormalizedModelClient,
  prepareMessagesForFamily,
  probeModelCapabilities,
} from "../src/router";

describe("P2-04 Universal Model Adapter & Governance Locks", () => {
  test("enforces disabledProviders AFTER --model pin resolution and blocks pin drift", () => {
    const profile = {
      id: "openrouter",
      base_url: "https://openrouter.ai/api/v1",
      default_model: "openai/gpt-4o",
    };
    // Pin resolves to openai/gpt-4o; disabledProviders=["openai"] catches it AFTER pin resolution.
    expect(() =>
      createNormalizedModelClient(profile, undefined, {
        pinnedModel: "openai/gpt-4o",
        disabledProviders: ["openai"],
      }),
    ).toThrow("disabled");

    // Switching away from --pinned-model is rejected.
    expect(() =>
      createNormalizedModelClient(profile, "anthropic/claude-sonnet", {
        pinnedModel: "openai/gpt-4o",
      }),
    ).toThrow("Pinned model lock");

    // ZDR header + verified directive on local/mid models.
    const localClient = createNormalizedModelClient(
      { id: "ollama", base_url: "http://127.0.0.1:11434/v1", default_model: "qwen2.5-coder:14b" },
      undefined,
      { requireZdr: true },
    );
    expect(localClient.capabilities.zdrNoLog).toBeTrue();
    expect(localClient.reasoningDirective).toContain("verified:");
    expect(probeModelCapabilities("google", "gemini-2.5-pro").vision).toBeTrue();
  });

  test("preserves thinking tokens within same model family and strips across families", () => {
    const msgs = [
      { role: "assistant" as const, content: "step 1", thinking: "deep chain", sourceFamily: "anthropic" as const },
    ];
    expect(prepareMessagesForFamily(msgs, "anthropic")[0]?.thinking).toBe("deep chain");
    expect(prepareMessagesForFamily(msgs, "openai")[0]?.thinking).toBeUndefined();
  });
});