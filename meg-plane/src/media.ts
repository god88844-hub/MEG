// P6-01: CLI Multimodal Media & Paste Ingestion:
// - Drag-and-drop & clipboard (Ctrl+V) image/video ingestion
// - ffmpeg 1fps scene-change keyframe extraction command & runner
// - Deterministic OCR/layout fallback for text-only local models

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export type MediaKind = "image" | "video";

export interface IngestedMedia {
  kind: MediaKind;
  source: "drag_drop" | "clipboard";
  originalPath?: string;
  mimeType: string;
  storedRelPath: string;
  pointer: string; // @media:...
  bytes: number;
  sha256: string;
}

const IMAGE_EXTS: Record<string, string> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".webp": "image/webp",
  ".gif": "image/gif",
  ".bmp": "image/bmp",
};

const VIDEO_EXTS: Record<string, string> = {
  ".mp4": "video/mp4",
  ".mov": "video/quicktime",
  ".webm": "video/webm",
  ".mkv": "video/x-matroska",
  ".avi": "video/x-msvideo",
};

/** Parse a dragged-and-dropped file path (handles Windows quotes, file:// URIs & spaces). */
export function parseDroppedMediaPath(rawInput: string): string {
  let clean = rawInput.trim();
  if ((clean.startsWith('"') && clean.endsWith('"')) || (clean.startsWith("'") && clean.endsWith("'"))) {
    clean = clean.slice(1, -1).trim();
  }
  if (clean.startsWith("file:///")) {
    clean = decodeURIComponent(clean.slice("file:///".length));
    if (process.platform !== "win32" && !clean.startsWith("/")) clean = "/" + clean;
  }
  return clean;
}

function classifyExtension(ext: string): { kind: MediaKind; mimeType: string } | null {
  const low = ext.toLowerCase();
  if (IMAGE_EXTS[low]) return { kind: "image", mimeType: IMAGE_EXTS[low]! };
  if (VIDEO_EXTS[low]) return { kind: "video", mimeType: VIDEO_EXTS[low]! };
  return null;
}

/** Ingest a dragged-and-dropped image/video into `<workspace>/.meg/media/<hash><ext>`. */
export function ingestDroppedMedia(workspace: string, rawInput: string): IngestedMedia {
  const filePath = parseDroppedMediaPath(rawInput);
  if (!fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) {
    throw new Error(`Dropped media file does not exist: ${filePath}`);
  }
  const ext = path.extname(filePath).toLowerCase();
  const info = classifyExtension(ext);
  if (!info) {
    throw new Error(`Unsupported media extension '${ext}'`);
  }
  const buf = fs.readFileSync(filePath);
  return persistMediaBuffer(workspace, buf, ext, info.kind, info.mimeType, "drag_drop", filePath);
}

/** Ingest clipboard (`Ctrl+V`) raw bytes or data-URI image payload into `<workspace>/.meg/media/`. */
export function ingestClipboardMedia(
  workspace: string,
  payload: Uint8Array | string,
  mimeType = "image/png",
): IngestedMedia {
  let buf: Buffer;
  let resolvedMime = mimeType;
  if (typeof payload === "string") {
    const match = payload.trim().match(/^data:([^;]+);base64,(.+)$/);
    if (match) {
      resolvedMime = match[1]!;
      buf = Buffer.from(match[2]!, "base64");
    } else {
      buf = Buffer.from(payload, "base64");
    }
  } else {
    buf = Buffer.from(payload);
  }
  if (buf.length === 0) throw new Error("Clipboard media payload is empty");
  const isVideo = resolvedMime.startsWith("video/");
  const ext = resolvedMime.includes("jpeg")
    ? ".jpg"
    : resolvedMime.includes("webp")
      ? ".webp"
      : resolvedMime.includes("mp4")
        ? ".mp4"
        : ".png";
  return persistMediaBuffer(workspace, buf, ext, isVideo ? "video" : "image", resolvedMime, "clipboard");
}

function persistMediaBuffer(
  workspace: string,
  buf: Buffer,
  ext: string,
  kind: MediaKind,
  mimeType: string,
  source: "drag_drop" | "clipboard",
  originalPath?: string,
): IngestedMedia {
  const sha256 = createHash("sha256").update(buf).digest("hex");
  const short = sha256.slice(0, 16);
  const rel = `.meg/media/${short}${ext}`;
  const full = path.join(workspace, rel);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  if (!fs.existsSync(full)) {
    fs.writeFileSync(full, buf);
  }
  return {
    kind,
    source,
    originalPath,
    mimeType,
    storedRelPath: rel,
    pointer: `@media:${rel}`,
    bytes: buf.length,
    sha256,
  };
}

/** Build the deterministic `ffmpeg` 1fps + scene-change keyframe extraction argv. */
export function buildFfmpegKeyframeArgv(
  videoPath: string,
  outDir: string,
  sceneThreshold = 0.3,
): string[] {
  const pattern = path.join(outDir, "frame_%04d.png");
  return [
    "ffmpeg",
    "-hide_banner",
    "-loglevel",
    "error",
    "-i",
    videoPath,
    "-vf",
    `fps=1,select='gt(scene,${sceneThreshold})+eq(n,0)'`,
    "-vsync",
    "vfr",
    pattern,
  ];
}

export interface TextOnlyFallbackCard {
  pointer: string;
  modelSupportsVision: boolean;
  mode: "native_vision" | "deterministic_ocr_layout_fallback";
  layoutSummary: string;
  extractedText: string;
  requiresIndependentReadingCorroboration: boolean;
}

/**
 * Deterministic OCR/layout fallback for text-only local models:
 * - Vision models receive the native `@media:` pointer directly.
 * - Text-only models receive a structured layout + dual-method OCR summary card.
 */
export function buildMediaModelPayload(
  media: IngestedMedia,
  options: {
    modelSupportsVision: boolean;
    ocrLines?: readonly string[];
    layoutBoxes?: readonly { region: string; x: number; y: number; w: number; h: number }[];
  },
): TextOnlyFallbackCard {
  if (options.modelSupportsVision) {
    return {
      pointer: media.pointer,
      modelSupportsVision: true,
      mode: "native_vision",
      layoutSummary: `${media.kind} (${media.mimeType}, ${media.bytes}B)`,
      extractedText: "",
      requiresIndependentReadingCorroboration: false,
    };
  }
  const boxes = options.layoutBoxes ?? [];
  const layoutSummary =
    boxes.length > 0
      ? boxes.map((b) => `${b.region}@(${b.x},${b.y},${b.w}x${b.h})`).join("; ")
      : `unboxed ${media.kind} (${media.bytes}B)`;
  const extractedText = (options.ocrLines ?? []).join("\n").trim() || "(no text detected)";
  return {
    pointer: media.pointer,
    modelSupportsVision: false,
    mode: "deterministic_ocr_layout_fallback",
    layoutSummary,
    extractedText,
    requiresIndependentReadingCorroboration: true,
  };
}