# MEG

A native terminal agent harness for local models and cloud APIs, built with Rust and a Bun control plane. No MEG account or login service is required.

MEG v0.1.0 includes a workspace browser, streaming chat with a minimal thinking animation, multiline editing, persistent sessions, provider/model selection, skills, declarative plugins, MCP tools, and read-only delegated workers.

## Windows release

Download `MEG-native-release.zip` from [Releases](https://github.com/god88844-hub/MEG/releases). Install [Bun](https://bun.sh), extract the entire archive into one directory, and run `meg.exe`. Keep `meg_ffi.dll` and `meg-plane` beside the executable.

The supported native target is Windows x86_64. Other operating systems are not certified by this release's CI.

## First run

1. Run `meg.exe` and open a folder, or use Ctrl+N to create a workspace.
2. Open `/provider`. Connect Ollama, LM Studio, llama.cpp, vLLM, or a cloud provider with your own API key. Paste keys into the masked API-key field.
3. Use `/model` to select a model reported by the connected server. Local model files must be registered with a running model server before they appear.
4. Describe a task. `/plan` selects read-only work, `/code` selects Act mode, and `/stop` cancels active work.

The default endpoint is local Ollama at `http://127.0.0.1:11434/v1`. Model names and cloud credentials are never invented. Cloud connections use the provider APIs you configure; no browser-account automation or personal gateway is bundled.

Type `/` or `/help` for commands. `/settings`, `/sessions`, `/workspace`, `/skills`, `/plugins`, `/mcp`, `/agent`, and `/stats` expose the product features. See [usage](docs/native-harness.md) and [security](SECURITY.md).

## Build and test

Requirements: Bun 1.4.4 or later, Rust's `stable-x86_64-pc-windows-gnu` toolchain, and a WinLibs/MinGW GNU linker on PATH. Windows CI sets up that linker with MSYS2.

```powershell
cd meg-rs
cargo test --locked --workspace
cargo build --locked --release --workspace
cd ../meg-plane
bun install --frozen-lockfile --ignore-scripts
bun test --timeout 20000
bun build src/index.ts --target=bun --outfile=dist/plane.js
cd ..
./meg-rs/target/release/meg.exe
```

The Bun FFI tests use the real release DLL built by the preceding Cargo command. Python is not required to build or run this product.

For headless execution:

```powershell
meg.exe run -C C:\projects\example "inspect this workspace"
```

Act mode and MCP tools can execute powerful operations. Read the security boundaries before running untrusted work. Provider compatibility depends on the selected model and endpoint. Remote MCP OAuth and arbitrary JavaScript plugin runtimes are outside v0.1.0 support.

## License

MIT. See [LICENSE](LICENSE).
