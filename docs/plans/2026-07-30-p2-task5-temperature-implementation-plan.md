# P2 Task 5 and Temperature Monitoring Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Complete per-user startup and display recovery while adding a truthful, low-overhead ACPI temperature metric.

**Architecture:** Keep geometry policy and temperature conversion testable outside platform APIs. Use Win32 adapters for current-user startup and monitor work areas, and add the temperature counter to the existing `SystemSampler` thread without another resident process.

**Tech Stack:** Rust 1.97.1, Slint 1.17.1, windows-rs 0.62.2, Windows Registry, Win32 monitor APIs, PDH.

---

### Task 1: ACPI temperature metric

**Files:**
- Modify: `crates/system-monitor/src/lib.rs`
- Modify: `apps/desktop/src/main.rs`
- Modify: `apps/desktop/ui/app-window.slint`

1. Add failing tests for Kelvin conversion, invalid samples, hottest-zone selection and UI text.
2. Add `temperature_celsius` to `MetricSnapshot` and a Windows PDH temperature sampler.
3. Bind the metric to the temperature card with an honest `ACPI 热区` source label.
4. Run `cargo test -p system-monitor -p desktop-assistant --offline`.

### Task 2: Current-user startup

**Files:**
- Modify: `Cargo.toml`
- Create: `apps/desktop/src/startup.rs`
- Modify: `apps/desktop/src/main.rs`
- Modify: `apps/desktop/ui/app-window.slint`

1. Add tests for the quoted startup command and path matching.
2. Implement read/write/delete operations under the current user's Run key.
3. Add synchronized settings and tray toggles; update UI only after a successful registry operation.
4. Verify enable, restart-state detection and disable without administrator rights.

### Task 3: Multi-display work-area recovery

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/app-core/src/lib.rs`
- Create: `apps/desktop/src/display.rs`
- Modify: `apps/desktop/src/main.rs`

1. Add failing tests for nearest-screen selection and a removed saved monitor.
2. Enumerate Win32 monitor work areas and move geometry policy into `app-core`.
3. Clamp at startup and on move, resize and scale-factor events; persist the corrected position.
4. Verify normal, off-screen and simulated removed-monitor positions at 150% DPI.

### Task 4: Quality gate and progress

**Files:**
- Modify: `README.md`
- Modify: `docs/plans/2026-07-30-desktop-assistant-mvp-implementation-plan.md`
- Modify: `docs/plans/2026-07-30-p2-desktop-shell-implementation-plan.md`
- Create: `docs/reports/2026-07-30-p2-desktop-shell-batch-4.md`

1. Run formatting, workspace tests, strict Clippy, release build and `git diff --check` offline.
2. Perform the Windows interaction and temperature checklist.
3. Record resource measurements, limitations and verified progress.
4. Commit the completed batch.
