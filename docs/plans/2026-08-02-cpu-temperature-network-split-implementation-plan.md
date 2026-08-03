# CPU Temperature And Network Split Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add native low-overhead CPU thermal-zone monitoring and display network receive/send as separate cards and trend rows.

**Architecture:** `system-monitor` owns a reusable Windows PDH CPU-temperature sampler and exposes its status in `MetricSnapshot`. The desktop keeps the existing single network sample, but formats and renders receive/send independently using a shared trend scale.

**Tech Stack:** Rust 1.97, windows-rs/PDH, Slint 1.17, Cargo tests and Clippy.

---

### Task 1: CPU temperature conversion and snapshot contract

**Files:**
- Modify: `crates/system-monitor/src/lib.rs`

**Step 1: Write failing tests**

Add tests proving that Kelvin values convert to plausible Celsius values, invalid/non-finite readings are rejected, and multiple readings select the hottest valid thermal zone.

**Step 2: Run the focused tests**

Run: `cargo test -p system-monitor cpu_temperature --offline`

Expected: FAIL because the conversion/selection helpers and snapshot field do not exist.

**Step 3: Implement the snapshot field and pure helpers**

Add `cpu_temperature_celsius: MetricValue<f32>` to `MetricSnapshot`, a 0..=150 Celsius validity rule, and deterministic selection of the highest valid converted value.

**Step 4: Run focused tests**

Run: `cargo test -p system-monitor cpu_temperature --offline`

Expected: PASS.

### Task 2: Native Windows ACPI thermal-zone sampler

**Files:**
- Modify: `crates/system-monitor/src/lib.rs`
- Modify: `crates/system-monitor/examples/probe.rs`

**Step 1: Add the reusable PDH reader**

Open one query for `\Thermal Zone Information(*)\Temperature`, prime it once, and reuse it for each sample. Map missing counter support to `Unavailable` and subsequent collection failures to `TemporarilyUnavailable`.

**Step 2: Wire it into `SystemSampler`**

Create the sampler once and fill `MetricSnapshot::cpu_temperature_celsius` on every sample without affecting the other metrics.

**Step 3: Update fixtures and probe output**

Populate the new snapshot field in tests and print it in the monitor probe.

**Step 4: Run system-monitor tests**

Run: `cargo test -p system-monitor --offline`

Expected: all tests pass.

### Task 3: Split desktop network state and trends

**Files:**
- Modify: `apps/desktop/src/main.rs`

**Step 1: Write failing formatting/trend tests**

Cover independent receive/send text, shared network scaling, missing points and separate one-series trend models.

**Step 2: Replace combined desktop fields**

Expose CPU-temperature text and independent download/upload values. Keep network normalization on one shared maximum, then produce distinct receive and send `TrendSeriesPoint` vectors.

**Step 3: Run desktop tests**

Run: `cargo test -p desktop-assistant --offline`

Expected: all tests pass.

### Task 4: Compact 2 x 4 monitoring layout

**Files:**
- Modify: `apps/desktop/ui/app-window.slint`

**Step 1: Add Slint properties**

Add CPU-temperature, download, upload and separate network trend properties. Remove the combined network-card-only properties and component.

**Step 2: Rebuild the monitoring grid**

Use four compact rows in the confirmed order: CPU/CPU temperature, memory/video memory, GPU/GPU temperature, download/upload. Preserve the 412 x 640 window.

**Step 3: Split trend rows**

Render download in blue and upload in red on separate rows, both from the shared normalized scale.

**Step 4: Compile the UI**

Run: `cargo check -p desktop-assistant --offline`

Expected: PASS with all generated setters resolved.

### Task 5: Quality gate and delivery

**Files:**
- Modify: `README.md`
- Replace: `desktop-assistant.exe`

**Step 1: Document behavior and limitations**

State that CPU temperature uses the Windows ACPI thermal-zone counter and can be unsupported on some hardware.

**Step 2: Run quality checks**

Run:

```powershell
cargo fmt --all -- --check
cargo test --workspace --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
git diff --check
```

Expected: all checks pass; only the existing Windows registry/credential tests may remain intentionally ignored.

**Step 3: Build and copy the executable**

Run: `cargo build --release -p desktop-assistant --offline`, then replace the root `desktop-assistant.exe` with the release artifact.

Expected: Release build succeeds and both executable hashes match.

**Step 4: Hand off manual UI verification**

Ask the user to confirm all eight cards fit, CPU temperature status is honest, down/up colors are blue/red, four trend rows use the full width, and no content is clipped.
