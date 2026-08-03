# AI Model Management Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an in-app AI manager for persistent cloud, local-service and Codex CLI models, with secure credential handling and immediate conversation-list refresh.

**Architecture:** Extend versioned non-secret profiles in `app-core`, keep API keys in Windows Credential Manager, and rebuild only the connector catalog after a successful transactional save. A Slint modal owns list/editor state while Rust validates and persists all mutations.

**Tech Stack:** Rust 1.97, Slint 1.17.1, windows-rs Credential Manager, serde JSON, existing OpenAI-compatible HTTP and Codex CLI connectors.

---

### Task 1: Extend the managed profile model

**Files:**
- Modify: `crates/app-core/src/config.rs`
- Create: `crates/app-core/src/agent_profiles.rs`
- Modify: `crates/app-core/src/lib.rs`

1. Add failing tests for HTTP deployment location, version-five migration, one-time catalog initialization state and stable serialization without raw secrets.
2. Add `HttpDeployment`, the initialization marker and config version migration.
3. Add pure draft validation, duplicate-name checks, stable ID allocation and edit/delete helpers.
4. Run `cargo test -p app-core --offline`.

### Task 2: Support managed Codex connector identities

**Files:**
- Modify: `crates/agent-cli/src/lib.rs`
- Modify: `crates/agent-cli/tests/connector.rs`

1. Add a failing test proving two configured Codex models can expose different stable connector IDs and names.
2. Add a constructor that accepts a managed descriptor while retaining the existing default constructor and discovery behavior.
3. Run `cargo test -p agent-cli --offline`.

### Task 3: Add synchronous configuration acknowledgement and credential rollback

**Files:**
- Create: `apps/desktop/src/ai_management.rs`
- Modify: `apps/desktop/src/main.rs`

1. Add tests for keep/replace/clear/delete credential plans and rollback behavior through test stores.
2. Extend the existing config worker with an acknowledged immediate-save command that serializes with debounced desktop-setting saves.
3. Implement model mutation transactions without logging or returning plaintext secrets.
4. Add one-time Codex seeding before runtime initialization.
5. Run `cargo test -p desktop-assistant --offline`.

### Task 4: Make the conversation catalog reloadable

**Files:**
- Modify: `apps/desktop/src/main.rs`

1. Add tests for preserving conversation history while rebuilding profiles, selecting a valid fallback after deletion and showing an empty state.
2. Store the conversation runtime behind UI-thread interior mutability and replace only profiles/catalog after a committed mutation.
3. Remove implicit Codex injection and build every connector from managed config.
4. Refresh the Slint model and selected model after each successful change.

### Task 5: Build the AI management modal

**Files:**
- Modify: `apps/desktop/ui/app-window.slint`
- Modify: `apps/desktop/src/main.rs`

1. Add the settings-page AI management row and an internal modal list/editor.
2. Add model type controls, conditional fields, masked key entry, configuration check, validation status, delete/credential confirmations and accessible labels.
3. Register Rust callbacks for open, close, create, edit, validate, save and delete.
4. Clear secret and draft state whenever the modal closes or a save succeeds.
5. Keep mutations disabled while a conversation is active.

### Task 6: Documentation, quality gates and delivery

**Files:**
- Modify: `README.md`
- Modify: `docs/plans/2026-07-30-desktop-assistant-mvp-implementation-plan.md`
- Create: `docs/reports/2026-08-02-ai-model-management.md`

1. Replace manual JSON instructions with AI management usage and document supported local/cloud connection types.
2. Run `cargo fmt --all -- --check`.
3. Run `cargo test --workspace --offline`.
4. Run `cargo clippy --workspace --all-targets --offline -- -D warnings`.
5. Run `git diff --check`.
6. Build `cargo build --release -p desktop-assistant --offline` and replace the root `desktop-assistant.exe`.
7. Verify the root and target Release hashes match, then provide manual UI verification steps.

## Completion Status

Implemented on 2026-08-02. Model validation, version-five migration, managed Codex identities, credential rollback, reloadable conversation catalogs and the Slint management modal are complete. Automated quality gates passed; visual and interaction confirmation remains manual by prior user decision.
