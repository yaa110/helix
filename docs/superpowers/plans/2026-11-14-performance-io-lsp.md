# Performance I/O and LSP Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the remaining UI-thread I/O from workspace switching, session persistence, replace-all, and tree search; fix file-watcher decoding; and make workspace switches stop and reload language services.

**Architecture:** Pure filesystem/serialization work moves into `Job` futures. UI callbacks only apply already-loaded data to `Editor`/components. Tree search uses a background index builder with generation checks and a cached result, while the UI applies filtering from the completed index. LSP servers are stopped before workspace teardown and language/config loaders are refreshed for the new root.

**Tech Stack:** Rust, Tokio, Helix `Job` futures, `ignore` walker, `ciborium`, `encoding_rs`.

**Spec:** User request: performance is critical; implement async workspace switching/session restore, file-watcher encoding correctness, and LSP stop/reload; no backward compatibility required.

## Global Constraints

- No legacy config compatibility work.
- No synchronous filesystem walks or session serialization on the UI thread for the covered paths.
- Preserve existing user-visible behavior unless it was incorrect.
- Every fix gets a regression test or a documented test limitation.

## Review Focus

- Workspace switch while a background job is in flight must not lose edits or apply stale results.
- Tree search results arriving after a query/workspace change must be discarded.
- Replace-all must not write outside the current workspace.
- File-watcher comparison must handle BOM/non-UTF-8 files.
- LSP processes must not accumulate across repeated workspace switches.

---

### Task 1: Async workspace/session I/O

**Files:**
- Modify: `helix-term/src/commands.rs`
- Modify: `helix-term/src/ui/folder_picker.rs`
- Modify: `helix-view/src/editor.rs`
- Test: `helix-term/tests/test/sidebar.rs`

**Interfaces:**
- `switch_workspace` becomes a UI entry point that validates current state, collects the old session, submits `SessionStore::save`, target `SessionStore::load`, branch lookup, and UI-state save/load as a `Job`, then applies the result in a callback.
- A staged struct carries `target`, `old_session`, `target_session`, `branch`, and `ui_state`.

- [ ] Add failing test that a modified buffer blocks switching (already present) and a test that switch remains responsive while session I/O is delayed.
- [ ] Move session save/load, git branch lookup, and UI-state persistence into the job.
- [ ] Apply only staged data on the UI callback; re-check modified buffers before destructive teardown.
- [ ] Run targeted tests.

### Task 2: Async replace-all

**Files:**
- Modify: `helix-term/src/ui/sidebar/search.rs`
- Modify: `helix-term/src/search.rs`
- Test: `helix-term/tests/test/search.rs`

**Interfaces:**
- `SearchPane::replace_all` applies open-buffer replacements on the UI thread and submits closed-file read/replace/write work as a `Job`.
- The completion callback updates status with replaced/file/skipped totals.

- [ ] Add a failing test with a delayed filesystem write and assert the event loop remains idle-responsive.
- [ ] Split open-document and closed-file replacement paths.
- [ ] Submit closed-file replacement as a background job with workspace-root validation.
- [ ] Run targeted tests.

### Task 3: Background tree search index

**Files:**
- Modify: `helix-term/src/ui/sidebar/file_tree.rs`
- Modify: `helix-term/src/ui/sidebar.rs`
- Test: `helix-term/src/ui/sidebar/file_tree.rs` unit tests and sidebar integration tests.

**Interfaces:**
- `FileTree` gains a generation-tagged background index request and cached entries.
- `rebuild_search` never walks the filesystem synchronously; it applies cached entries or marks search pending.
- Stale index responses are ignored by generation/epoch.

- [ ] Add a failing test that typing a filter does not call the walker synchronously.
- [ ] Add index request/result types and background worker.
- [ ] Poll completed indexes during rendering and apply filters from cached entries.
- [ ] Update search tests to wait for index completion deterministically.
- [ ] Run targeted tests.

### Task 4: File-watcher decoding correctness

**Files:**
- Modify: `helix-term/src/file_watcher.rs`
- Test: `helix-term/src/file_watcher.rs`

**Interfaces:**
- `disk_content_matches` decodes disk bytes using `Document::encoding()` and compares against the document rope without allocating a full copy.

- [ ] Add BOM and non-UTF-8 reload tests.
- [ ] Replace `read_to_string` with document-aware decoding.
- [ ] Compare with `doc.text().chars().eq(contents.chars())`.
- [ ] Run targeted tests.

### Task 5: LSP lifecycle on workspace switch

**Files:**
- Modify: `helix-lsp/src/lib.rs`
- Modify: `helix-view/src/editor.rs`
- Modify: `helix-term/src/commands.rs`
- Test: `helix-term/tests/test/sidebar.rs`

**Interfaces:**
- `Registry::stop_all()` force-shuts down all clients and clears event routing.
- Workspace switching stops all servers before closing documents and reloads the workspace language loader/config after setting the new root.

- [ ] Add a test-visible server-count helper and failing switch A→B→A test.
- [ ] Implement `Registry::stop_all()`.
- [ ] Stop servers and reload language/config state during switch.
- [ ] Run targeted tests.

### Task 6: Full verification, commit, reinstall

- [ ] Run `cargo fmt --all`.
- [ ] Run library tests for `helix-term`, `helix-view`, `helix-tui`, and `helix-lsp`.
- [ ] Run the full integration suite and report the known pre-existing failures by name.
- [ ] Commit, push, and reinstall via Homebrew.
