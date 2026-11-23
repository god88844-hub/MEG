import { afterAll, describe, expect, test } from "bun:test";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { loadFfi } from "../src/ffi-bridge";

// Bridge 1 integration: real meg_ffi.dll via bun:ffi.
const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-ffi-test-"));
fs.mkdirSync(path.join(ws, "src"));
fs.writeFileSync(path.join(ws, "src/fixture.rs"), "// Offline native FFI search fixture.\n");
const ffi = loadFfi();

afterAll(() => {
  const tempRoot = path.resolve(os.tmpdir()) + path.sep;
  if (!path.resolve(ws).startsWith(tempRoot) || !path.basename(ws).startsWith("meg-ffi-test-")) throw new Error("Unsafe fixture cleanup path");
  fs.rmSync(ws, { recursive: true, force: true });
});

describe("FfiBridge (Bridge 1, real meg_ffi.dll)", () => {
  test("repeated Unicode results stay owned after native allocations are released", () => {
    const file = "ffi-owned.txt";
    const retained: string[] = [];
    for (let i = 0; i < 256; i++) {
      const expected = `payload-${i}-世界`;
      fs.writeFileSync(path.join(ws, file), expected);
      retained.push(ffi.readFileCapped(ws, file, 128));
      expect(retained[i]).toBe(expected);
      expect(ffi.truncateObservation(expected, 128)).toBe(expected);
    }
    expect(retained[0]).toBe("payload-0-世界");
  });

  test("a bundled consumer finds the DLL beside the installed application", async () => {
    const installed = path.join(ws, "standalone");
    const entry = path.join(ws, "ffi-proof.ts");
    const planeDir = path.join(installed, "meg-plane");
    fs.mkdirSync(planeDir, { recursive: true });
    fs.copyFileSync(path.resolve(import.meta.dir, "../../meg-rs/target/release/meg_ffi.dll"), path.join(installed, "meg_ffi.dll"));
    fs.writeFileSync(path.join(ws, "proof.txt"), "standalone-世界");
    fs.writeFileSync(entry, `import {loadFfi} from ${JSON.stringify(path.resolve(import.meta.dir, "../src/ffi-bridge.ts"))}; console.log(loadFfi().readFileCapped(${JSON.stringify(ws)}, "proof.txt", 128));`);
    const built = await Bun.build({ entrypoints: [entry], outdir: planeDir, target: "bun" });
    expect(built.success).toBeTrue();
    const env = { ...process.env };
    delete env.MEG_FFI_DLL;
    if (process.platform === "win32") env.PATH = `${path.dirname(process.execPath)};${env.SystemRoot}\\System32;${env.SystemRoot}`;
    const child = Bun.spawn([process.execPath, path.join(planeDir, "ffi-proof.js")], { cwd: installed, env, stdout: "pipe", stderr: "pipe" });
    const output = await new Response(child.stdout).text();
    const error = await new Response(child.stderr).text();
    expect(await child.exited, error).toBe(0);
    expect(output.trim()).toBe("standalone-世界");
  });
  test("searchFiles finds repo sources", () => {
    const hits = ffi.searchFiles(ws, "**/fixture.rs");
    expect(hits).toContain("src/fixture.rs");
  });

  test("editFile applies a unique str_replace", () => {
    fs.writeFileSync(path.join(ws, "ffi-test-tmp.txt"), "alpha beta\ngamma\n");
    const out = ffi.editFile(ws, "ffi-test-tmp.txt", "gamma", "delta", false);
    expect(out).toContain("edited");
    expect(fs.readFileSync(path.join(ws, "ffi-test-tmp.txt"), "utf-8")).toContain("delta");
  });

  test("editFile rejects non-unique without replace_all (in-band error)", () => {
    fs.writeFileSync(path.join(ws, "ffi-test-tmp.txt"), "x = 1\nx = 1\n");
    const out = ffi.editFile(ws, "ffi-test-tmp.txt", "x = 1", "x = 2", false);
    expect(out.startsWith("error:")).toBeTrue();
    expect(out).toContain("2 times");
  });

  test("readFileCapped truncates with a marker", () => {
    fs.writeFileSync(path.join(ws, "ffi-test-tmp.txt"), "z".repeat(500));
    const out = ffi.readFileCapped(ws, "ffi-test-tmp.txt", 100);
    expect(out.length).toBeLessThanOrEqual(200);
    expect(out).toContain("truncated");
  });

  test("truncateObservation preserves FATAL lines", () => {
    let body = "";
    for (let i = 0; i < 300; i++) body += `row ${i}\n`;
    body += "FATAL: database connection refused";
    const out = ffi.truncateObservation(body, 600);
    expect(out).toContain("FATAL: database connection refused");
  });

  test("parseActionVariant classifies free-model markup", () => {
    expect(ffi.parseActionVariant("<think>hmm</think>{\"final\":\"ok\"}")).toBe("final");
    expect(
      ffi.parseActionVariant(
        "<tool_call><function=read_file><parameter=path>x</parameter></function=read_file></tool_call>",
      ),
    ).toBe("tool");
    expect(ffi.parseActionVariant("no action here at all")).toBeNull();
  });
});
