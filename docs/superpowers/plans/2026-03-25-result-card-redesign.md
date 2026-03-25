# Result Card Redesign Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign the Rust telemetry share image so `/backend/results?id=<uuid>` returns a `1200x720` dark Nexio-branded card, and add a local dummy-image generation path for visual validation.

**Architecture:** Keep the existing telemetry route shape and storage flow intact, but replace the flat renderer in `src/results/telemetry.rs` with a structured compositor built around explicit header, metric, and footer regions. Because the crate is binary-only and the renderer currently depends on global config/font initialization, the implementation must add module-local tests with explicit one-time setup and a CLI early-exit path for sample image generation that runs before normal server boot.

**Tech Stack:** Rust, `image`/`imageproc`, `ab_glyph`, existing telemetry route stack, local filesystem output for sample rendering

---

## File Structure

- `src/results/telemetry.rs`
  Responsibility: result-card display-model mapping, renderer, renderer-focused tests, and a reusable sample-render helper.
- `src/http/routes.rs`
  Responsibility: route-level `/backend/results` behavior and malformed-telemetry regression coverage.
- `src/http/response.rs`
  Responsibility: image response content type and response helper correctness.
- `src/cmd.rs`
  Responsibility: CLI argument parsing for the sample-image generation path.
- `src/main.rs`
  Responsibility: early-exit sample generation before normal config/database/server startup.
- `README.md`
  Responsibility: brief developer-facing documentation for generating a dummy result image.

## Shared Testing Setup Requirement

This crate does not expose a library target, and `draw_result()` currently depends on initialized `FONT` and `SERVER_CONFIG` globals. Every module-local test that touches the renderer must include a one-time setup path that initializes config/font state before assertions run.

Use a shared test helper inside `src/results/telemetry.rs` or a small internal helper function to:

- initialize `SERVER_CONFIG` and `FONT` exactly once
- set deterministic rendering config for tests
- avoid depending on normal `main()` server startup

Do not leave this implicit in the tests.

### Task 1: Add Safe Telemetry Display Mapping And Route Regression Coverage

**Files:**
- Modify: `src/results/telemetry.rs`
- Modify: `src/http/routes.rs`
- Reference: `src/results/mod.rs`
- Reference: `src/ip/ip_info.rs`
- Reference: `docs/superpowers/specs/2026-03-25-result-card-redesign-design.md`

- [ ] **Step 1: Write the failing test**

Add module-local tests that exercise display mapping and route safety.

Tests must live in the source modules they cover and include explicit setup for config/font globals where required.

Cover at least:

- valid `isp_info` JSON yields provider/footer data
- malformed `isp_info` JSON does not panic display mapping
- malformed `extra` does not panic display mapping
- long provider/location/IP strings collapse or truncate without empty placeholder junk
- route-level malformed stored telemetry does not panic `/backend/results`

- [ ] **Step 2: Run test to verify it fails**

Run:
- `cargo test telemetry -- --nocapture`
- `cargo test routes -- --nocapture`

Expected:
- mapping tests fail because no safe display-model layer exists yet
- route-level malformed telemetry regression fails because the current renderer can still panic on bad `isp_info`

- [ ] **Step 3: Write minimal implementation**

Refactor `src/results/telemetry.rs` so the renderer first maps `TelemetryData` into an explicit display-model struct with safe fallbacks:

- provider/ISP from `isp_info.processedString` when valid
- location/server context from parsed `isp_info`, or from parsed `extra` only if `extra` matches a known trustworthy shape chosen during implementation
- shortened optional IP display
- formatted timestamp
- stable attribution string

Add route-safe handling so malformed stored telemetry no longer panics the `/backend/results` flow.

- [ ] **Step 4: Run test to verify it passes**

Run:
- `cargo test telemetry -- --nocapture`
- `cargo test routes -- --nocapture`

Expected:
- new display-model tests pass
- malformed route regression passes without panics

- [ ] **Step 5: Commit**

```bash
git add src/results/telemetry.rs src/http/routes.rs src/results/mod.rs src/ip/ip_info.rs
git commit -m "test: harden telemetry result mapping and route safety"
```

### Task 2: Replace The Flat Renderer With The New 1200x720 Card Contract

**Files:**
- Modify: `src/results/telemetry.rs`
- Reference: `assets/open-sans.ttf`
- Reference: `docs/superpowers/specs/2026-03-25-result-card-redesign-design.md`

- [ ] **Step 1: Write the failing test**

Add rendering-focused tests in `src/results/telemetry.rs`.

Tests must assert at least:

- output bytes are non-empty
- rendered image decodes successfully
- decoded image dimensions are exactly `1200x720`
- the outer safe margin is preserved by keeping primary drawn regions away from the image edge
- long footer strings do not force content outside the card bounds

If exact pixel-perfect assertions are too brittle, test structural invariants that still prove the `1200x720` contract and overflow protection.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test draw_result -- --nocapture`
Expected: fail because the current renderer still emits the old `500x286` contract and lacks the new grouped layout assumptions

- [ ] **Step 3: Write minimal implementation**

Rewrite the compositor in `src/results/telemetry.rs` to render the approved design:

- `1200x720` dark full-bleed card
- branded header with Nexio monogram/title and timestamp
- primary metric row for download/upload
- secondary metric row for ping/jitter
- footer metadata band with better spacing
- procedural metric/icon treatment instead of fetched assets
- safe truncation/collapse for long metadata fields

Prefer grouped layout helpers or small internal functions over one large coordinate block.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test draw_result -- --nocapture`
Expected: rendering tests pass and confirm the new fixed image contract plus overflow protections

- [ ] **Step 5: Commit**

```bash
git add src/results/telemetry.rs
git commit -m "feat: redesign telemetry share card layout"
```

### Task 3: Switch The Image Encoding And Response Helper To PNG

**Files:**
- Modify: `src/results/telemetry.rs`
- Modify: `src/http/response.rs`
- Modify: `src/http/routes.rs`

- [ ] **Step 1: Write the failing test**

Add or extend tests so the renderer and HTTP helper are verified together.

Cover at least:

- renderer encodes PNG bytes
- image response helper uses `Content-Type: image/png`
- `/backend/results` still returns a successful image response for valid stored telemetry

- [ ] **Step 2: Run test to verify it fails**

Run:
- `cargo test result -- --nocapture`
- `cargo test response -- --nocapture`

Expected: fail because the current path still writes JPEG and returns `Content-Type: image/jpeg`

- [ ] **Step 3: Write minimal implementation**

Update the rendering and response path consistently:

- change `draw_result()` to encode PNG instead of JPEG
- update the image response helper to return `Content-Type: image/png`
- keep the route contract stable so the frontend share flow still works unchanged

- [ ] **Step 4: Run test to verify it passes**

Run:
- `cargo test result -- --nocapture`
- `cargo test response -- --nocapture`

Expected: renderer/response tests pass with PNG output and matching headers

- [ ] **Step 5: Commit**

```bash
git add src/results/telemetry.rs src/http/response.rs src/http/routes.rs
git commit -m "feat: serve telemetry share cards as png"
```

### Task 4: Add A Sample-Image CLI Path That Exits Before Server Boot

**Files:**
- Modify: `src/results/telemetry.rs`
- Modify: `src/main.rs`
- Modify: `src/cmd.rs`
- Modify: `README.md`

- [ ] **Step 1: Write the failing test**

Define a concrete dummy-generation path and verify it does not require normal server startup.

Cover at least:

- CLI argument parsing for `--generate-sample-result <path>`
- early-exit behavior before normal server listen/database boot
- successful writing of a sample image from hard-coded telemetry data

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo run -- --generate-sample-result /tmp/result-card-sample.png`
Expected: fail because no such argument/early-exit path exists yet

- [ ] **Step 3: Write minimal implementation**

Add a developer-only sample-generation path:

- parse `--generate-sample-result <path>` in `src/cmd.rs`
- add an early-return branch in `src/main.rs` before normal server startup
- create deterministic sample `TelemetryData`
- render with the same production compositor
- write the output to the requested path
- document the command in `README.md`

The command must not require a live DB row or a running HTTP server.

- [ ] **Step 4: Run test to verify it passes**

Run:
- `cargo run -- --generate-sample-result /tmp/result-card-sample.png`
- `file /tmp/result-card-sample.png`

Expected:
- command exits successfully without starting the server
- `/tmp/result-card-sample.png` exists
- `file` identifies it as PNG image data

- [ ] **Step 5: Commit**

```bash
git add src/results/telemetry.rs src/main.rs src/cmd.rs README.md
git commit -m "feat: add sample telemetry result image generation"
```

### Task 5: Verify The Full Result Image Path End To End

**Files:**
- Test: `src/results/telemetry.rs`
- Test: `src/http/response.rs`
- Test: `src/http/routes.rs`
- Test: `src/main.rs`
- Test: `/tmp/result-card-sample.png`

- [ ] **Step 1: Write the failing test**

Run the full verification bundle before final cleanup and record any remaining gaps.

Run:
- `cargo test -- --nocapture`
- `cargo run -- --generate-sample-result /tmp/result-card-sample.png`
- `file /tmp/result-card-sample.png`

Expected: any remaining failures or regressions are surfaced clearly before final fixes

- [ ] **Step 2: Run test to verify it fails**

Inspect the exact output and isolate the final regression if one exists.

- [ ] **Step 3: Write minimal implementation**

Fix only the remaining issues discovered during the final verification pass:

- rendering regressions
- header/content-type mismatches
- sample-generation path issues
- malformed telemetry route regressions
- layout-breaking edge cases from oversized strings

- [ ] **Step 4: Run test to verify it passes**

Run:
- `cargo test -- --nocapture`
- `cargo run -- --generate-sample-result /tmp/result-card-sample.png`
- `file /tmp/result-card-sample.png`

Expected:
- all Rust tests pass
- sample image generation succeeds
- the generated file is confirmed as PNG image data

This generated sample image is the artifact to show the user for visual validation.

- [ ] **Step 5: Commit**

```bash
git add src/results/telemetry.rs src/http/response.rs src/http/routes.rs src/main.rs src/cmd.rs README.md
git commit -m "test: finalize telemetry result card redesign"
```
