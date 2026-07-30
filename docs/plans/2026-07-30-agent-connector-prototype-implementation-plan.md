# Agent Connector Prototype Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build and verify one runtime-independent agent contract, a scripted HTTP stream simulator, and a real child-process CLI streaming prototype.

**Architecture:** `agent-core` owns transport-neutral requests, capabilities, events, errors, cancellation and validation. `agent-http` and `agent-cli` implement the same object-safe trait while keeping simulation and process details outside the desktop UI.

**Tech Stack:** Rust 1.97 standard library, MPSC channels, atomics, `std::process::Command`, existing Cargo workspace.

---

### Task 1: Core Connector Contract

**Files:**
- Create: `crates/agent-core/Cargo.toml`
- Create: `crates/agent-core/src/lib.rs`
- Modify: `Cargo.toml`

**Steps:**
1. Add failing tests for request validation, unsupported attachments, event order primitives, error retry semantics and idempotent cancellation.
2. Run `cargo test -p agent-core --offline` and confirm the missing contract fails to compile.
3. Implement descriptors, capabilities, request/message types, event/error enums, cancellation token, `AgentRun` and object-safe `AgentConnector`.
4. Run `cargo test -p agent-core --offline`; all core tests must pass.

### Task 2: Scripted HTTP Connector

**Files:**
- Create: `crates/agent-http/Cargo.toml`
- Create: `crates/agent-http/src/lib.rs`
- Create: `crates/agent-http/examples/mock_stream.rs`
- Modify: `Cargo.toml`

**Steps:**
1. Add failing tests for ordered chunks, 401, 429, disconnect-after-chunks and cancellation.
2. Run `cargo test -p agent-http --offline` and confirm failure before implementation.
3. Implement `MockHttpConnector` with a scripted scenario and a short interruptible delay.
4. Run the package tests and `cargo run -p agent-http --example mock_stream --offline`; expect `Started`, text deltas and `Completed`.

### Task 3: Child-Process CLI Connector

**Files:**
- Create: `crates/agent-cli/Cargo.toml`
- Create: `crates/agent-cli/src/lib.rs`
- Create: `crates/agent-cli/src/bin/agent-cli-fixture.rs`
- Create: `crates/agent-cli/tests/connector.rs`
- Create: `crates/agent-cli/examples/cli_stream.rs`
- Modify: `Cargo.toml`

**Steps:**
1. Add failing integration tests for line streaming, literal shell metacharacters, exit code mapping and cancellation.
2. Run `cargo test -p agent-cli --offline` and confirm the connector is absent.
3. Implement shell-free `Command` startup, stdin prompt delivery, stdout/stderr readers, process polling and kill-on-cancel.
4. Run package tests and the example against the fixture binary; expect one unified event stream and no console shell.

### Task 4: Workspace Verification And Documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/plans/2026-07-30-desktop-assistant-mvp-implementation-plan.md`
- Create: `docs/reports/2026-07-30-p1-agent-connectors.md`

**Steps:**
1. Run `cargo fmt --all -- --check`.
2. Run `cargo test --workspace --offline`.
3. Run `cargo clippy --workspace --all-targets --offline -- -D warnings`.
4. Record event sequences, test counts, process behavior, limitations and updated P1 progress.
5. Run `git diff --check`, commit the connector milestone, and confirm the worktree is clean.

