# Live builds: editing Ornithochory while it runs

Written 2026-10-26. What shortens the loop from saving a file to seeing the
change, for a Bevy 0.20 library developed on tir (aarch64, 12 cores, 15 GB)
and toth (x86_64), what the repository now does, and what was left out and
why. Numbers are from tir unless they say otherwise.

## 1. Where the time went

Before this change, a debug `grove` was 2.2 GB, almost all of it Bevy's
debuginfo (dependencies build at `opt-level = 3` with full debuginfo), and
tir linked it with GNU ld. The shared devenv module asks for mold through
`env.RUSTFLAGS = mkDefault "-C link-arg=-fuse-ld=mold"`, but devenv 2.3's
rust module sets `RUSTFLAGS` itself from `languages.rust.rustflags` (empty)
at normal priority, so the mold flag never reached cargo. Its own linker
choice (`languages.rust.clangLinker`, on by default on Linux) forces lld only
on x86_64, so toth got lld and tir got clang's default, GNU ld (the release
`grove` carries no linker stamp, which mold and lld always write).

Linking the same debug `grove` (deps with line tables only, 760-780 MiB),
three runs each, from its captured link line:

| Linker | Link, s | Notes |
|---|---|---|
| GNU ld (bfd) | 12.6-13.9 | what tir was using |
| lld 21 | 1.7-1.9 | devenv's default on x86_64 |
| mold 3.0 | 0.87-0.90 | now used |
| wild 0.10 | 0.72-0.77 | fastest, see section 3 |

With mold and line-table-only dependencies, a real edit to a library file
(a constant in `mesh/leaves.rs`) rebuilds and relinks `grove` in 2.3-3.5 s,
an edit to the example in about 2 s. A full debug build of `grove` from
nothing takes 6 min 40 s at 4 jobs. The binary went from 2.2 GB to 814 MB.

## 2. What the repository does now

1. **mold through devenv** (`languages.rust.mold.enable` in `devenv.nix`),
   the way devenv 2.3 wants a linker set: as the host target's
   `CARGO_TARGET_<triple>_LINKER` wrapper, never through `RUSTFLAGS`. The
   shared module (`chousx/devenv/bevy`) should do the same so every Bevy
   project gets it; its `env.RUSTFLAGS` line is dead under devenv 2.3.
2. **Line tables only for dependencies** (`[profile.dev.package."*"] debug =
   "line-tables-only"`): backtraces still name files and lines in Bevy, the
   binary is a third the size, and the linker has a third as much to copy.
   Ornithochory itself keeps full debuginfo.
3. **Species reload in the examples**: Bevy's `file_watcher` on the
   examples' dev-dependency. `reload_species` already regrew plants on
   `AssetEvent::Modified`, but the examples never enabled the watcher, so
   no event came.
4. **Hot patching** with Bevy's `hotpatching` feature, which is Dioxus'
   subsecond driven by `dx serve --hot-patch` (dioxus-cli 0.7.10 in the
   devenv shell, matching the `dioxus-devtools` 0.7.10 in `Cargo.lock`).
   Every system call goes through subsecond's jump table, so a patch takes
   effect on the next frame. Ornithochory's `hotpatching` feature adds
   `regrow_after_hotpatch`: on `HotPatchChanges` it drops every cached
   variant and material and regrows, so an edit to a grower or mesher shows
   at once rather than only on plants generated after the patch. `live/` is
   the grove example as a crate of its own (`grove-live` runs it).

## 3. Options weighed

- **Bevy `dynamic_linking`** (Bevy as one dylib): the largest win in Bevy's
  setup guide, because it takes Bevy out of every link. With mold and slim
  debuginfo the whole link is under a second here, so it has little left to
  save, and dx cannot find the dylib when hot patching (dioxus#4154). Not
  used.
- **wild**: 0.15 s faster than mold per link here, on aarch64 too. devenv
  has `languages.rust.wild.enable`. Kept mold because it is the linker the
  subsecond and Bevy hot-patching guides test with; worth a try with dx.
- **Cranelift** for the crate itself: Bevy's guide quotes about 30% faster
  codegen, nightly only, LLVM for dependencies. Cranelift supports
  `std::arch` only in part, and glam uses NEON on aarch64, so tir is the
  risky platform. Each try costs a full rebuild of every dependency (the
  profile changes). Not tried.
- **`-Zshare-generics=y`, `-Zthreads=N`** (Bevy's `config_fast_builds.toml`):
  nightly flags for the compile half of the loop, about 1.5-2.5 s of the
  2.3-3.5 s now. Not measured: each needs a full rebuild. `-Zthreads`
  raises memory per rustc, and tir already OOM'd at 12 jobs.
- **`bevy_simple_subsecond_system`** (now TheBevyFlock's
  `bevy_hotpatching_experiments`): the predecessor of Bevy's built-in
  hot patching, adds `#[hot(rerun_on_hot_patch)]` for setup systems and
  signature changes. Its newest release targets Bevy 0.18. Not needed:
  plants regenerate from `Update`, which built-in patching covers.

## 4. Hot patching: what works and what does not

Subsecond patches the crate that holds `main` and, since dx 0.7.6
(dioxus#5291, workspace hot patching), the workspace crates it depends on
directly. Known limits, from the subsecond docs and open issues:

- **Not a library beside the example**: when the running crate's package
  also has a `lib` (an example of this package, or `src/main.rs` beside
  `src/lib.rs`), dx reports a patch but leaves the library's code out
  (dioxus#5778, fix in #5779 still open). Hence `live/`.
- **Direct dependencies only**: edits two crates down the chain are not
  seen (dioxus#5314). Ornithochory is a direct dependency of `live`;
  genetics, tarasaur and ornisculpt are not patched at all.
- **No struct changes**: a changed layout for a type the running app holds
  (a `Species`, a component, a resource) crashes or misreads it. Rebuild
  (`r` in dx) instead.
- **No new system parameters, no added or removed systems, no rerun of
  `Startup`.**
- **Thread-locals reset** on each patch; statics' destructors never run.
- **Only code reached through a system** changes. Tasks already running on
  `AsyncComputeTaskPool` finish with the old code; `regrow_after_hotpatch`
  drops them.

## Sources

- [Bevy setup guide (fast compiles)](https://bevy.org/learn/quick-start/getting-started/setup/)
- [Bevy `.cargo/config_fast_builds.toml`](https://github.com/bevyengine/bevy/blob/main/.cargo/config_fast_builds.toml)
- [Bevy PR #19309: hot patching systems with subsecond](https://github.com/bevyengine/bevy/pull/19309)
- [Bevy `hotpatching_systems` example](https://docs.rs/bevy/latest/src/hotpatching_systems/hotpatching_systems.rs.html)
- [subsecond docs (0.7.10)](https://docs.rs/subsecond/latest/subsecond/)
- [Dioxus releases](https://github.com/DioxusLabs/dioxus/releases); [dioxus#4160 / #5291 workspace hot patching](https://github.com/DioxusLabs/dioxus/issues/4160); [#5778 lib + bin](https://github.com/DioxusLabs/dioxus/issues/5778); [#5779](https://github.com/DioxusLabs/dioxus/pull/5779); [#5314 transitive crates](https://github.com/DioxusLabs/dioxus/issues/5314); [#4154 dynamic linking](https://github.com/DioxusLabs/dioxus/issues/4154); [#4149 mold and a global target dir](https://github.com/DioxusLabs/dioxus/issues/4149)
- [TheBevyFlock/bevy_hotpatching_experiments](https://github.com/TheBevyFlock/bevy_simple_subsecond_system)
- [Bevy workshop (EuroRust 2025): hot patching](https://vleue.github.io/bevy_workshop-eurorust-2025/0-intro/hot-patching.html)
- [devenv `languages/rust.nix`](https://github.com/cachix/devenv/blob/main/src/modules/languages/rust.nix)
- [Lattimore, "Rust dylib rabbit holes" (2024)](https://davidlattimore.github.io/posts/2024/08/27/rust-dylib-rabbit-holes.html); [Wild 0.10](https://phoronix.com/news/Wild-Linker-0.10)
