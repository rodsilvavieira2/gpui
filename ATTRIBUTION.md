# Attribution

This fork vendors code from two upstream projects, both Apache-2.0 licensed.
Per Apache-2.0 §4, this file records what was taken, from where, and what
was changed. Original copyright notices are preserved in `LICENSE-APACHE`
(Zed) and in the per-file headers below (Longbridge).

## Zed Industries — GPUI 0.2.2 (base of this repo)

- Source: <https://github.com/zed-industries/zed> (crate `gpui` 0.2.2,
  `repository = "https://github.com/zed-industries/zed"`)
- License: Apache-2.0, copyright 2022–2025 Zed Industries, Inc.
  (`LICENSE-APACHE`)
- Local changes on top of 0.2.2:
  - `src/app.rs`: `QuitMode::Explicit` backport (quit only via `App::quit`).
  - `src/app.rs`, `src/window.rs`: motion backports (see below).

## Longbridge — gpui-kit motion system (ported, adapted)

- Source: <https://github.com/longbridge/gpui-kit> at commit `cfab7b2`
  (Apache-2.0)
- Files and their origin:
  | This fork | gpui-kit origin (`crates/base/src/`) |
  | --- | --- |
  | `src/motion.rs` (Transition core + tests, springs excluded) | `motion.rs` |
  | `src/motion/timing.rs` | `motion/timing.rs` |
  | `src/motion/easing.rs` | `motion/easing.rs` |
  | `src/motion/keyframes.rs` | `motion/keyframes.rs` |
  | `src/motion/stagger.rs` | `motion/stagger.rs` |
  | `src/motion/sequence.rs` | `motion/sequence.rs` |
  | `src/motion/presence.rs` | `motion/presence.rs` |
  | `src/motion/reveal.rs` | `motion/reveal.rs` |
  | `src/motion/effects.rs` | `animation.rs` |
- Adaptations for GPUI 0.2 APIs:
  - `Spring`/`spring()` not ported (needs 0.3-native `SpringConfig` etc.).
  - `std::time::Instant` unconditionally (no `web_time`/wasm target here).
  - Easing helpers live under `motion::effects`, not beside `motion`.
  - `reveal.rs`: `gpui-fast` cfg gate removed (no Fast backend here).
  - Deprecated `Transition` alias dropped (would collide with
    `motion::Transition`).
  - Tests: `TestAppContext::open_window(size, _)` → `add_window(_)` (0.2
    test API), `draw(cx).clear(cx)` → `draw(cx).clear()`.
  - Docs added throughout for `#![deny(missing_docs)]`.
- `patches/xattr`: Hefesto-authored errno mapping (Linux/Android ENOATTR →
  ENODATA), copied from the Hefesto workspace; required to build on modern
  libc. No upstream involved.
- Wave-0 shims added by this fork for the port (not from either upstream):
  - `App::reduce_motion` / `App::set_reduce_motion` (`src/app.rs`),
    backporting the GPUI 0.3 flag.
  - `Window::simulate_next_frame` (`src/window.rs`, test-only), backporting
    the GPUI 0.3 test helper onto 0.2.2's `next_frame_callbacks` queue.
