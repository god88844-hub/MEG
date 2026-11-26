# Security Policy

Report vulnerabilities privately through [GitHub security advisories](https://github.com/god88844-hub/MEG/security/advisories/new). Include reproduction steps and the affected commit/version. Never include real API keys in reports or public issues.

## Verified Boundaries

- **Local-first & BYOK:** Local inference uses your configured server; cloud inference transmits conversation context and tool observations to the provider you select. Remote plaintext `http://` endpoints are blocked by default (only loopback `localhost`/`127.0.0.1`/`::1` allows HTTP unless `MEG_ALLOW_INSECURE_HTTP=1` is set).
- **Credential storage:** On Windows, API keys are stored in Windows Credential Manager (never written to JSON settings or session transcripts) and masked in UI inputs. Non-Windows platforms use environment variables.
- **Workspace & secret airlock:** Native file tools (`read_file`, `write_file`, `edit_file`, `grep_files`, `search_files`) confine paths to the selected workspace, block `.env*` files, and scrub detected secret patterns from observations and `@` attachments.
- **Plan mode & delegated workers:** Plan mode and `meg worker` block mutating native tools at the execution boundary. Note: MCP `readOnlyHint` annotations remain trusted server metadata, not external-process enforcement.
- **Act mode shell boundary (known limitation):** Act mode executes shell commands with the user's OS privileges. Application-level command guards and Windows Job Objects do **not** constitute a full OS kernel sandbox. Use OS/container isolation when running untrusted repositories until Phase 5 OS sandboxing ships.
- **MCP trust & supply-chain:** New or modified MCP servers require explicit start/trust, and local MCP script/binary contents are hashed into the trust fingerprint. Third-party `SKILL.md` files are screened by the DDIPE supply-chain scanner across both `read_skill` and `/skills`.
- **CI & provenance:** Cross-platform CI runs Rust tests and strict zero-warning Clippy on Windows, Linux, and macOS plus Python offline evals. Tagged releases publish `SHA256SUMS.txt` checksums verified by the installer scripts. Independent third-party security audits have not been performed.
