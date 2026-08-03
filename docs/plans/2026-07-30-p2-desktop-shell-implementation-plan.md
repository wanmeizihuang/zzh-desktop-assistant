# P2 Desktop Shell Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Status:** Closed with documented follow-up validation (2026-07-31)

**Goal:** Build a persistent, controllable Windows desktop shell around ZZH Desktop Assistant with one lifecycle state shared by the window and tray.

**Architecture:** Keep lifecycle and behavior policy in `app-core`, with Slint as a view and `apps/desktop` as the Windows adapter. Persist only non-sensitive desktop settings and recover invalid/off-screen state before showing the window.

**Tech Stack:** Rust 1.97.1, Slint 1.17.1, winit 0.30 through Slint, serde/serde_json, windows-rs.

---

### Task 1: Lifecycle state machine

**Status:** Complete (2026-07-30)

**Files:**
- Modify: `crates/app-core/src/lib.rs`

1. Add failing tests for startup, collapse, hide/restore, exit and behavior settings.
2. Run `cargo test -p app-core --offline` and confirm the new tests fail.
3. Implement the minimal lifecycle and settings API.
4. Run the package tests and confirm they pass.

### Task 2: Real window behaviors

**Status:** Complete (2026-07-30)

**Files:**
- Modify: `apps/desktop/ui/app-window.slint`
- Modify: `apps/desktop/src/main.rs`

1. Add bindable properties and callbacks for topmost and position lock.
2. Replace visual-only switches with stable interactive switches.
3. Handle Escape through the winit event adapter and collapse only when expanded.
4. Gate both collapsed and expanded drag gestures on the lock setting.
5. Run desktop tests and compile the Slint UI.

### Task 3: Configuration persistence

**Status:** Complete (2026-07-30)

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/app-core/Cargo.toml`
- Create: `crates/app-core/src/config.rs`
- Modify: `apps/desktop/src/main.rs`

1. Add tests for default config, JSON round-trip, corrupt-file preservation and atomic replacement.
2. Implement versioned settings and a store whose path is injected by the caller.
3. Resolve the production path under `%LOCALAPPDATA%/ZZH Desktop Assistant`.
4. Restore settings and a clamped position before normal use; persist setting and move changes.

### Task 4: Tray and single instance

**Status:** Complete (2026-07-30)

**Files:**
- Modify: `Cargo.toml`
- Modify: `apps/desktop/Cargo.toml`
- Modify: `apps/desktop/ui/app-window.slint`
- Create: `apps/desktop/src/single_instance.rs`
- Modify: `apps/desktop/src/main.rs`

1. Add a tray icon derived from the approved ZZH mascot asset and implement the agreed menu.
2. Route every tray action through the shared lifecycle/settings state.
3. Add a current-user single-instance guard and wake-up channel.
4. Verify show/hide, expand/collapse, settings, behavior toggles and exit from the tray.

Monitoring pause remains in P6 with alert and worker-control behavior; it is not exposed as a non-functional tray action.

### Task 5: Startup and display recovery

**Status:** Complete with accepted follow-up validation (2026-07-31)

**Files:**
- Create: `apps/desktop/src/startup.rs`
- Modify: `apps/desktop/src/main.rs`
- Modify: `crates/app-core/src/lib.rs`

1. Implement a per-user startup toggle without administrator rights.
2. Re-clamp the window on display topology and scale-factor changes.
3. Choose the nearest available work area when the saved monitor no longer exists.
4. Verify on Windows 11 at 100%, 150% and a simulated removed secondary display.

Implementation and automated verification are complete for the current-user Run value, Win32 work-area enumeration, nearest-screen selection, removed-monitor geometry and ACPI thermal-zone monitoring. The isolated HKCU write/read/delete test passes without administrator rights. Physical placement across live multi-monitor and mixed-DPI changes remains a documented P6 compatibility-matrix check because the automated desktop session does not expose the Slint tool window as an interactive top-level window.

### Task 6: P2 quality gate

**Status:** Complete with accepted follow-up items (2026-07-31)

**Files:**
- Modify: `README.md`
- Modify: `docs/plans/2026-07-30-desktop-assistant-mvp-implementation-plan.md`
- Create: `docs/reports/2026-07-30-p2-desktop-shell.md`

1. Run formatting, all workspace tests, strict Clippy and a release build offline.
2. Perform the complete desktop-shell interaction checklist.
3. Recheck collapsed idle CPU and working set against the P1 baseline.
4. Record remaining limitations and update progress only for verified tasks.

Formatting, workspace tests, strict Clippy and the Release build passed on 2026-07-31. The final report records the accepted multi-monitor validation and animated-mascot resource optimization follow-ups. See `docs/reports/2026-07-31-p2-desktop-shell-final.md`.
