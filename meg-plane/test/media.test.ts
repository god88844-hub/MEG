import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  buildFfmpegKeyframeArgv,
  buildMediaModelPayload,
  ingestClipboardMedia,
  ingestDroppedMedia,
  parseDroppedMediaPath,
} from "../src/media";

describe("P6-01 CLI Multimodal Media & Paste Ingestion", () => {
  test("ingests drag-and-drop & clipboard media into @media: pointers, builds 1fps ffmpeg keyframe argv, and emits OCR/layout fallback for text-only models", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p601-"));
    try {
      const srcImg = path.join(ws, "ui shot.png");
      fs.writeFileSync(srcImg, Buffer.from("fake-png-content"));

      expect(parseDroppedMediaPath(`"${srcImg}"`)).toBe(srcImg);
      const dropped = ingestDroppedMedia(ws, `"${srcImg}"`);
      expect(dropped.kind).toBe("image");
      expect(dropped.pointer).toMatch(/^@media:\.meg\/media\/[a-f0-9]+\.png$/);

      const pasted = ingestClipboardMedia(
        ws,
        `data:image/png;base64,${Buffer.from("clipboard-png").toString("base64")}`,
      );
      expect(pasted.source).toBe("clipboard");
      expect(fs.existsSync(path.join(ws, pasted.storedRelPath))).toBeTrue();

      const argv = buildFfmpegKeyframeArgv("demo.mp4", path.join(ws, "frames"), 0.35);
      expect(argv[0]).toBe("ffmpeg");
      expect(argv.join(" ")).toContain("fps=1,select='gt(scene,0.35)+eq(n,0)'");

      const textFallback = buildMediaModelPayload(pasted, {
        modelSupportsVision: false,
        ocrLines: ["Total: ₹1,250"],
        layoutBoxes: [{ region: "header", x: 0, y: 0, w: 1440, h: 64 }],
      });
      expect(textFallback.mode).toBe("deterministic_ocr_layout_fallback");
      expect(textFallback.extractedText).toContain("1,250");
      expect(textFallback.requiresIndependentReadingCorroboration).toBeTrue();

      const visionPayload = buildMediaModelPayload(pasted, { modelSupportsVision: true });
      expect(visionPayload.mode).toBe("native_vision");
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});