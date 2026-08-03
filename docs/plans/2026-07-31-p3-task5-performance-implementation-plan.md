# P3 Task 5 Performance Optimization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reduce monitoring and animation overhead while the assistant is collapsed or hidden, without delaying fresh data when the monitoring panel opens.

**Architecture:** A channel-driven three-state monitor loop owns sampling cadence and wakes immediately for the interactive state. Slint reports visibility, expansion and tab changes; background states never enqueue metric or trend UI work.

**Tech Stack:** Rust 1.97.1, `std::sync::mpsc`, Slint 1.17.1, existing Windows-native samplers.

---

### Task 1: Define monitor activity policy

**Files:**
- Modify: `apps/desktop/src/main.rs`

1. Add failing tests for UI-state mapping, 2/10/20-second intervals and UI dispatch eligibility.
2. Run `cargo test -p desktop-assistant --offline monitor_activity` and confirm the tests fail before implementation.
3. Add `MonitorActivity`, its pure mapping function and interval/dispatch methods.
4. Rerun the focused tests and confirm they pass.

### Task 2: Make the worker channel-driven

**Files:**
- Modify: `apps/desktop/src/main.rs`
- Modify: `apps/desktop/ui/app-window.slint`

1. Replace the stop-only channel with `MonitorCommand::{SetActivity, Stop}` and expose a cloneable control handle.
2. Build trends and enqueue Slint work only for the interactive state.
3. Guard the queued closure against a visibility change before applying both metrics and trends.
4. Add a Slint monitor-state callback and notify it after tab, expansion and visibility changes.
5. Run `cargo test -p desktop-assistant --offline`.

### Task 3: Reduce animation redraw work

**Files:**
- Modify: `apps/desktop/ui/app-window.slint`

1. Change the mascot timer to 100 ms when collapsed and 200 ms when expanded.
2. Keep the timer stopped while hidden.
3. Render settings-page candidate previews at a fixed first frame.
4. Compile the desktop UI through `cargo test -p desktop-assistant --offline`.

### Task 4: Verify and report

**Files:**
- Modify: `docs/plans/2026-07-31-p3-monitoring-core-implementation-plan.md`
- Modify: `docs/plans/2026-07-30-desktop-assistant-mvp-implementation-plan.md`
- Create: `docs/reports/2026-07-31-p3-monitoring-core-batch-5.md`

1. Run `cargo fmt --all`.
2. Run `cargo test --workspace --offline` and require zero failures.
3. Run `cargo clippy --workspace --all-targets --offline -- -D warnings`.
4. Run `cargo build --release -p desktop-assistant --offline`.
5. Run `git diff --check`.
6. Record the implementation, measured baseline and remaining live visual/performance verification limit.
