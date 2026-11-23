# Security policy

The supported release is the Windows x86_64 native workstation, v0.1.x.

Report vulnerabilities privately through [GitHub security advisories](https://github.com/god88844-hub/MEG/security/advisories/new). Include reproduction steps and the affected version. Do not include real API keys in reports or public issues.

## Boundaries

- Local inference uses the model server you select. Cloud inference sends conversation context and tool observations to your selected provider.
- Windows stores provider keys in Credential Manager. Keys are masked in the editor and excluded from provider settings and session files. Environment-variable keys are also supported.
- Native file tools confine paths to the selected workspace. Plan mode and delegated workers block mutating tools.
- Act mode can execute shell commands. The harness is not an operating-system sandbox. Use operating-system isolation for untrusted tasks.
- MCP servers run commands or contact endpoints you configure. Inspect configurations before connecting and review tool permissions.
- Plugins in this release are local declarative command templates. Remote MCP OAuth and arbitrary JavaScript plugin runtimes are not implemented.

Rust and Bun tests run in CI. This release does not claim independent security certification or signed-release provenance.
