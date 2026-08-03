# P4 Agent Interaction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Deliver a secure, cancellable and stream-capable desktop conversation flow over OpenAI-compatible HTTP and Codex CLI connectors.

**Architecture:** `agent-core` owns protocol invariants, deterministic run reduction and connector selection. Transport crates perform on-demand I/O, while the desktop app consumes validated state and keeps all conversation history in memory.

**Tech Stack:** Rust 1.97.1, standard MPSC channels, reqwest/rustls for HTTP, serde/serde_json, windows-rs Credential Manager APIs, Slint 1.17.1.

---

### Task 1: Freeze event and run-state semantics

**Status:** Complete (2026-07-31)

**Files:**
- Modify: `crates/agent-core/src/lib.rs`

1. Add failing tests for valid streaming reduction, request-ID mismatch, delta before start, duplicate start, terminal events and events after terminal.
2. Run `cargo test -p agent-core --offline` and confirm failure before implementation.
3. Implement `RunPhase`, `RunTranscript`, `EventSequenceError` and deterministic `apply_event` rules.
4. Rerun `cargo test -p agent-core --offline` and require all tests to pass.

### Task 2: Add connector catalog and non-secret profiles

**Status:** Complete (2026-07-31)

**Files:**
- Modify: `crates/agent-core/src/lib.rs`
- Modify: `crates/app-core/src/config.rs`
- Test: package unit tests

1. Add tests for stable connector IDs, duplicate rejection, selection, missing connectors and profile JSON migration.
2. Implement a small insertion-ordered connector catalog using `Arc<dyn AgentConnector>`.
3. Add HTTP/CLI profile configuration containing endpoint, model, executable/arguments and credential reference, never a raw secret.
4. Run `cargo test -p agent-core -p app-core --offline`.

### Task 3: Implement OpenAI-compatible streaming HTTP

**Status:** Complete (2026-07-31)

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/agent-http/Cargo.toml`
- Modify: `crates/agent-http/src/lib.rs`
- Create: `crates/agent-http/tests/openai_compatible.rs`

1. Add a local scripted HTTP server test for ordered SSE deltas, `[DONE]`, malformed JSON, disconnect, timeout, 401 and 429.
2. Add a reusable HTTP client and bounded SSE parser with explicit endpoint/model configuration.
3. Map transport outcomes to existing `ConnectorErrorCode` values and honor cancellation between reads.
4. Run `cargo test -p agent-http --offline` without external network access.

### Task 4: Implement the Codex CLI adapter

**Status:** Complete (2026-07-31)

**Files:**
- Modify: `crates/agent-cli/src/lib.rs`
- Modify: `crates/agent-cli/tests/connector.rs`

1. Add tests for executable discovery, argument construction, streaming output, cancellation and missing executable errors.
2. Add a Codex-specific constructor that uses argument arrays and stdin, with no shell interpolation or console window.
3. Preserve bounded stderr and process termination semantics.
4. Run `cargo test -p agent-cli --offline`.

### Task 5: Add Windows Credential Manager storage

**Status:** Complete (2026-07-31)

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/app-core/src/credentials.rs`
- Modify: `crates/app-core/src/lib.rs`

1. Add pure tests for credential target naming, secret redaction and validation.
2. Implement current-user Generic Credential read/write/delete through windows-rs.
3. Add an ignored isolated round-trip test that writes only a generated test target.
4. Run `cargo test -p app-core --offline` and the isolated test explicitly when permitted.

### Task 6: Integrate the desktop conversation UI

**Status:** Complete (2026-07-31)

**Files:**
- Modify: `apps/desktop/Cargo.toml`
- Modify: `apps/desktop/src/main.rs`
- Modify: `apps/desktop/ui/app-window.slint`

1. Add pure tests for one-active-run control, new request IDs, stop, retry eligibility and bounded transcript size.
2. Connect agent selection, prompt sending, incremental text, stop, retry and error states to the existing conversation tab.
3. Keep connector work off the Slint event loop and suppress stale events after cancellation or retry.
4. Run full format, tests, strict Clippy and Release build.
5. Provide manual verification steps for input, streaming, stop, retry, switching and error display; do not automate desktop interaction.
