# GPU Temperature Selection Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace unrelated ACPI thermal-zone values with GPU-only temperature selection that prefers a discrete GPU and truthfully degrades when the chosen adapter exposes no sensor.

**Architecture:** DXGI determines one temperature target class before sampling. A dynamically loaded Intel Level Zero Sysman adapter reads only Intel devices matching that class; the desktop receives both metric status and target class so unavailable states remain explicit.

**Tech Stack:** Rust 1.97, windows-rs, DXGI 1.3, Intel Level Zero Sysman dynamic FFI, Slint 1.17.1.

---

### Task 1: Freeze GPU temperature target policy

**Files:**
- Modify: `crates/system-monitor/src/lib.rs`

1. Add failing tests proving a dedicated adapter wins over an active integrated adapter, integrated is selected only when no dedicated adapter exists, and virtual/unsupported vendors are ignored.
2. Run `cargo test -p system-monitor --offline` and confirm the new symbols are missing.
3. Add `GpuTemperatureTarget`, adapter identity candidates and deterministic selection.
4. Rerun the package tests and require the policy tests to pass.

### Task 2: Replace ACPI with Intel Level Zero

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/system-monitor/src/lib.rs`

1. Add pure tests for Intel integrated/dedicated device matching, valid Celsius filtering and GPU/GPU-board/global sensor priority.
2. Add the Windows LibraryLoader feature and fixed Level Zero FFI definitions verified against Intel's official headers.
3. Load `ze_loader.dll` only from System32, support new and legacy Sysman initialization, enumerate matching devices and retain only matching temperature handles.
4. Remove the ACPI PDH reader and return `Unsupported` when no matching sensor exists; sampling failures remain retryable.
5. Run `cargo test -p system-monitor --offline` and strict package Clippy.

### Task 3: Propagate target-aware UI states

**Files:**
- Modify: `apps/desktop/src/main.rs`
- Modify: `apps/desktop/ui/app-window.slint`
- Modify: `README.md`

1. Add failing format tests for available, unsupported and temporarily unavailable integrated/dedicated GPU temperatures.
2. Add the selected target to `MetricSnapshot` and update all snapshot fixtures.
3. Replace ACPI labels with `核显温度` / `独显温度`; no target must say that no physical GPU was detected.
4. Run desktop tests, full workspace tests, strict Clippy, formatting and `git diff --check`.
5. Build Release, replace the root `desktop-assistant.exe`, and provide manual display verification steps without automating the UI.

## Completion Status

Completed on 2026-08-02. The Intel Level Zero path, target-aware UI states, and truthful unsupported fallback are implemented. Automated checks and the current-device probe passed; visual confirmation remains manual by prior user decision. NVIDIA and AMD native temperature adapters remain part of the P6 hardware compatibility work.
