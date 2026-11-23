import { describe, expect, test } from "bun:test";
import { isSecurityOrCryptoTask, routeTask } from "../src/security-router";

describe("Direct Security Task Router", () => {
  test("named security tasks always route to the root thread", () => {
    for (const name of ["filter-js-from-html", "crack-7z-hash", "vulnerable-secret"]) {
      expect(routeTask(name, "", 0).kind).toBe("root-thread");
    }
  });

  test("security-shaped prompts route to the root thread even without a task name", () => {
    expect(routeTask(null, "crack the 7z hash and recover the password", 0).kind).toBe("root-thread");
    expect(routeTask(null, "write an XSS payload filter bypass", 0).kind).toBe("root-thread");
  });

  test("benign tasks delegate within the 3-5 hand-off cap", () => {
    const v = routeTask("log-summary", "summarize the log date ranges", 2);
    expect(v.kind).toBe("delegate");
  });

  test("hand-off cap (5) forces root-thread completion", () => {
    expect(routeTask("log-summary", "summarize logs", 5).kind).toBe("root-thread");
  });
});
