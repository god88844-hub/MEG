// Bridge 1 -- Bun FFI into the meg-rs cdylib (in-process, zero fork).
// The hot path (search/grep/list/edit/read/truncate/parse/index) calls Rust
// directly in ~us; strings cross as UTF-8 pointers and are released via meg_free.

import { dlopen, FFIType, CString } from "bun:ffi";
import path from "node:path";
import fs from "node:fs";

export interface FfiLib {
  searchFiles(workspace: string, pattern: string): string;
  grepFiles(workspace: string, pattern: string, glob?: string, maxResults?: number): string;
  listDir(workspace: string, relPath?: string): string;
  readFileCapped(workspace: string, path: string, cap: number): string;
  editFile(workspace: string, path: string, oldS: string, newS: string, replaceAll: boolean): string;
  truncateObservation(text: string, limit: number): string;
  parseActionVariant(raw: string): string | null;
  indexWorkspace(root: string): string;
}

let lib: FfiLib | null = null;

/** Installed bundles keep the library beside the executable or control plane. */
function dllPath(): string {
  const override = process.env.MEG_FFI_DLL;
  if (override) {
    const resolved = path.resolve(override);
    if (!fs.existsSync(resolved)) throw new Error("MEG_FFI_DLL does not name an existing library");
    return resolved;
  }
  const candidates = [
    path.resolve(import.meta.dir, "meg_ffi.dll"),
    path.resolve(import.meta.dir, "../meg_ffi.dll"),
    path.resolve(import.meta.dir, "../../meg-rs/target/release/meg_ffi.dll"),
    path.resolve(import.meta.dir, "../../../meg-rs/target/release/meg_ffi.dll"),
  ];
  for (const c of candidates) {
    if (fs.existsSync(c)) return c;
  }
  throw new Error(
    `meg_ffi.dll not found — build it: (meg-rs) cargo build -p meg-ffi --release`,
  );
}

export function loadFfi(): FfiLib {
  if (lib) return lib;
  const { symbols } = dlopen(dllPath(), {
    meg_search_files: { args: [FFIType.cstring, FFIType.cstring], returns: FFIType.ptr },
    meg_grep_files: {
      args: [FFIType.cstring, FFIType.cstring, FFIType.cstring, FFIType.u32],
      returns: FFIType.ptr,
    },
    meg_list_dir: { args: [FFIType.cstring, FFIType.cstring], returns: FFIType.ptr },
    meg_read_file: { args: [FFIType.cstring, FFIType.cstring, FFIType.u32], returns: FFIType.ptr },
    meg_edit_file: {
      args: [FFIType.cstring, FFIType.cstring, FFIType.cstring, FFIType.cstring, FFIType.bool],
      returns: FFIType.ptr,
    },
    meg_truncate_observation: { args: [FFIType.cstring, FFIType.u32], returns: FFIType.ptr },
    meg_parse_action_variant: { args: [FFIType.cstring], returns: FFIType.ptr },
    meg_index_workspace: { args: [FFIType.cstring], returns: FFIType.ptr },
    meg_free: { args: [FFIType.ptr], returns: FFIType.void },
  });
  const ownedString = (value: unknown): string => {
    const pointer = value as number;
    if (!pointer) throw new Error("Native bridge returned no string");
    try { return new CString(pointer).toString(); }
    finally { symbols.meg_free(pointer); }
  };
  lib = {
    // Bun FFI contract: cstring args must be JS strings (auto null-terminated).
    searchFiles: (ws, pattern) => ownedString(symbols.meg_search_files(ws, pattern)),
    grepFiles: (ws, pattern, glob = "", maxResults = 50) =>
      ownedString(symbols.meg_grep_files(ws, pattern, glob, maxResults)),
    listDir: (ws, relPath = ".") => ownedString(symbols.meg_list_dir(ws, relPath)),
    readFileCapped: (ws, p, cap) => ownedString(symbols.meg_read_file(ws, p, cap)),
    editFile: (ws, p, oldS, newS, all) =>
      ownedString(symbols.meg_edit_file(ws, p, oldS, newS, all)),
    truncateObservation: (text, limit) => ownedString(symbols.meg_truncate_observation(text, limit)),
    parseActionVariant: (raw) => {
      const p: number = symbols.meg_parse_action_variant(raw) as unknown as number;
      if (!p) return null; // null pointer = no parseable action
      return ownedString(p);
    },
    indexWorkspace: (root) => ownedString(symbols.meg_index_workspace(root)),
  };
  return lib;
}