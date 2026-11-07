# Insight — Local-First Research Agent

Tauri 2 desktop app: Rust backend (`crates/insight-core`, `src-tauri`), SvelteKit + Svelte 5 frontend, Tailwind 4.

Architecture, data model, ingestion, storage layout and the user-facing behaviour live in `docs/src/` (`architecture.md`, `development.md`, …). Read those rather than re-deriving them from source.

**Product invariant:** document storage, search and sync stay local. Only the conversation — queries and retrieved excerpts — is sent to the configured chat provider. No code path may ship document content off-device.

## Commands

Prerequisite: libclang for `mupdf-sys`'s bindgen (`clang` on Arch, `libclang-dev clang` on Debian/Ubuntu, Xcode command line tools on macOS, LLVM + `LIBCLANG_PATH` on Windows).

```bash
pnpm install
pnpm tauri dev
pnpm tauri build

pnpm test:run                        # frontend tests
pnpm check                           # svelte-check
pnpm lint                            # eslint

cd src-tauri && cargo test --workspace
cd src-tauri && cargo clippy --workspace --all-targets
cd src-tauri && cargo fmt --all --check
```

All of these run in the pre-commit hook; the work is finished only when they pass.

## State: who owns what

Rust owns storage truth. The frontend mirrors it and never merges entities into it.

- Route `load` functions own data (`+layout.ts` / `+page.ts` with `depends('app:…')`); components read `data`.
- A mutation the UI initiated re-reads by calling `invalidate(...)` after the command resolves.
- The only backend→frontend signals are changes the UI cannot derive: `collection-invalidated` (a peer changed a collection) and `pipeline-progress` (display, plus "this collection's pipeline went idle" → re-read the counts). They are consumed only in `src/lib/invalidation.ts`. Do not add per-write change events.
- The chat run is the one client-owned stream: `send_message` streams app-owned `ChatEvent`s over a Tauri `Channel` and the store applies them to its transcript. Rust normalizes both stored history and the live run into `TranscriptItem`s; optimistic run state belongs in the store and nowhere else.

Adding data: if Rust knows it, it is a command read through a load. If it changes behind the UI's back, that is peer sync and rides the existing invalidation.

## Invariants the code does not state

- Sources are immutable; text is a derivation stored per `{parser}@{version}`. Parser names may not contain `/` or `@`; bump the version by hand when a parser's output changes.
- A document's page count is derived from its canonical extraction's units, never stored.
- Conversations are local and never sync; collections sync fully, blobs included.
- No OCR: MuPDF's text layer only, so scanned PDFs import with little or no text. Known limitation, not a bug.
- No migrations anywhere — settings, conversation layouts, documents predating extractions. There are no users; invalid legacy state throws or is ignored, never bridged.
- A dependency's stored or wire format and stream semantics stop at the Rust boundary; the UI and its DTOs are app-owned.

## Conventions

- Rust: stdlib first, `tokio` (iroh requires it), `thiserror` error types (never `String` errors), doc comments on public items.
- Frontend: Svelte 5 runes, TypeScript. Tailwind 4 utilities; no `theme()` in component styles.
- No back-compat shims, fallbacks or defensive guards. Invalid state throws with a clear error.
- Do not add a dependency for what the stdlib or the framework already does.
- rig: work from the local checkout at `~/Projects/rig` (tag `v0.46.0`, matching the crates.io dependency), its `AGENTS.md` and `examples/`. For other Rust dependencies prefer `cargo doc`, `cargo tree` and `~/.cargo/registry/src` over the web. Local references: `~/Projects/tauri`, `~/Projects/tauri-docs`, `~/Projects/kit`, `~/Projects/svelte`.
- Commit messages: conventional prefix (`refactor(chat): …`, `fix(ui): …`); integration merges are `merge: <topic>`.

## Testing

Assert what a caller observes — return values, files written, what renders — through public interfaces. Never private helpers, internal state or call order: a refactor that changes no behaviour changes no test. Mock at the boundary (rig's test-utils model, temp directories for iroh/milli, `mockIPC` for Tauri commands), never our own code. No test-only APIs in production code, no smoke tests.

Frontend: Vitest + @testing-library/svelte + jsdom, tests co-located as `*.test.ts`, SvelteKit mocks in `src/tests/mocks/`. Rust: `#[tokio::test]` with its own temporary data directory per test.
