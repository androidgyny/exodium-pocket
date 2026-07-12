# Changelog

## Unreleased

### Added

- **Toast notification system** (`stores/toasts.ts`, `ToastContainer.tsx`): download,
  uninstall, launch, and content-pack errors now surface as toasts instead of being
  silent or confined to inline status text. Includes a catalog-update notice on startup.
- **Hierarchical genre browsing**: genre sections and the jumpbar collapse
  " / "-delimited subgenres into ~15 top-level categories, matching the genre
  filter's new tree dropdown (`Select.tsx` depth rendering, `get_section_keys`
  parent collapsing).
- **README screenshots** and release plan under `docs/`.

### Changed

- **macOS: native titlebar** - macOS builds use the system traffic-light controls
  (`tauri.macos.conf.json` + runtime `set_decorations(true)`); the custom
  `WindowFrame` is now Linux/Windows-only.
- **Game detail panel rework**: pinned media strip, launch-button spinner, errors
  via toasts.
- Tab switches animate with a directional slide.

### Fixed

- **macOS: DOSBox launch EBADF** - Tauri 2 GUI builds hit `posix_spawn` EBADF when
  redirecting DOSBox stdio to log files. On macOS stdio is now nulled and a no-op
  `pre_exec` forces fork+exec; other platforms keep per-game DOSBox log files.

- **LP game launch - commented-out autoexec** (`patch_dosbox_conf`): LP games whose
  `dosbox.conf` has the game-launch lines commented out with `#` (e.g. _Das Amt_) now
  launch correctly. When Strategy 1 (redirect EN config) produces an autoexec with no
  executable command, `find_lp_launch` is called to locate the real launcher by
  inspecting the game directory.

- **LP game launch - extended launcher discovery** (`find_lp_launch`): Added two new
  fallback strategies beyond the existing `run.bat` / `.com` search:
  - **Strategy 2** - scans for any `.bat` file (excluding known utilities like
    `anleit`, `install`, `problem`) that calls a `.exe` or `.com`; returns the `.bat`
    itself so all its steps execute in sequence.
  - **Strategy 4** - looks for a `.exe` in named subdirectories, skipping DOS/4GW
    extenders (`rtm`, `dos4gw`, `dpmi`, `cwsdpmi`) and installers.

- **"Download incomplete" false positive** (`get_download_progress`): Games like
  _Captain Zins_ and _Skyworker_ could show a permanent "Download incomplete" error
  even though their download had never been attempted. Root cause: torrent pieces
  received while downloading a neighbouring file can cover a small game's bytes
  entirely, causing librqbit to report 100% for that file before it is ever selected
  - the file is therefore never assembled on disk. The code now re-requests file
  assembly via `download_files` (which calls `update_only_files`) and keeps polling
  rather than surfacing an error.

### Changed

- `autoexec_has_launch_cmd`: drive-switch detection generalised from a hard-coded
  `c:`/`d:`/`e:`/`f:` list to any single ASCII letter followed by `:`, covering
  floppy drives (`a:`, `b:`) and drives above `f:`. Also added `echo ` and `@exit`
  to the non-launch filter list.

- `DownloadManager`: new `is_file_selected` method used to gate re-trigger spawns in
  `get_download_progress`, preventing a new task being spawned on every 1-second poll
  while librqbit assembles the file.

### Added

- **Test suite**:
  - Frontend: `vitest` + `jsdom` wired up; `pnpm test` / `pnpm run test:watch` /
    `pnpm run test:all`.
  - Rust: `tempfile` + `pretty_assertions` dev-dependencies; tests for
    `queries` (insert/fetch, language merging, config), `import/xml`
    (shortcode extraction, LP path handling, full XML parse round-trip), and
    `commands/games` (`patch_dosbox_conf`, `find_lp_launch`,
    `collection_data_dir`).
