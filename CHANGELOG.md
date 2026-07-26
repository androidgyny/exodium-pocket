# Changelog

## 0.8.0 - 2026-07-26

### Fixed (third adversarial review pass - 17 confirmed findings)

- **Uninstall no longer wipes the extras' download credit**: the piece
  ledger now only clears pieces of files actually deleted from disk - the
  still-present GameData ZIP keeps its credit, so reinstalling doesn't
  re-download gigabytes of extras.
- **Ledger restore survives Windows delete-pending** (exFAT/SMB/older NTFS):
  written via temp-file + rename with retries; failures now log at error
  level instead of silently reverting to the full re-check.
- **Uninstall during the extras phase** no longer leaves an orphaned poller
  that resurrects phantom stuck-download state for the removed game.
- **Cancel during validation sticks**: a deselect rejected by an
  initializing torrent is re-applied automatically once the check finishes
  (previously the cancelled game kept downloading invisibly).
- **Extras completion has a disk fallback** for librqbit's known stat-stall,
  and the extras phase resumes visibly after an app restart.
- **Disk preflight** credits on-disk bytes once (was double), and a refusal
  no longer leaves a phantom "My Games" entry.
- Selection updates hold the lock across the apply (closes a cancel race);
  install-moment refresh no longer skipped on re-downloads; UI reads an
  explicit installed flag instead of string-matching statuses.

### Fixed

- **The extras download phase is visible**: after a game installs, its
  GameData (manuals, videos, music - often larger than the game itself)
  keeps downloading; the card now shows "Installed - downloading extras…
  N%" instead of finishing silently, and when the extras land the manual
  button resolves automatically. Games stay playable the moment the game
  itself is installed.
- **Manual button tells the truth**: it now only appears for games that
  actually have a manual in the catalog, retries the lookup on click (the
  manual arrives inside the game's extras download, which often finishes
  after the game itself), and no longer caches a premature "no manual" for
  the whole session. Icon removed from the label.
- **Spanish/Polish metadata packs no longer show a 0 B download size** -
  the manifest carried placeholder sizes.

## 0.7.4 - 2026-07-26

### Fixed

- **No more "Validating torrent" after uninstall**: uninstall now patches
  librqbit's piece ledger surgically - only the removed game's pieces are
  cleared and the ledger is restored for the next add, which loads it via
  fastresume and starts downloading in seconds, exactly like a fresh
  install. The 15-30 minute full re-check (which field testing showed is
  slow regardless of antivirus or disk type) only remains for genuinely
  unrecoverable states (missing/corrupt ledger).

### Fixed

- **Uninstall is idempotent**: half-states (incomplete download, failed
  extraction) can be cleaned up instead of erroring with "not installed".
- **Detail panel no longer flickers** on install/uninstall completions -
  media state resets only when the displayed game actually changes.

## 0.7.3 - 2026-07-26

### Fixed

- **"Validating torrent" frozen forever after uninstall -> re-download**
  (observed twice on Windows): pushing a file-selection update into a
  torrent mid-initial-check could wedge librqbit's checking task. Selection
  updates now wait for the check to finish (without blocking progress
  polling) before applying.
- **librqbit upgraded** from a git-pinned 9.0.0-beta.2 to 9.0.0-rc.0 from
  crates.io - months of upstream fixes and no git-pin supply-chain
  dependency.

### Fixed (second adversarial review pass - 19 confirmed findings)

- **Linux deb/rpm installs no longer offered un-installable updates** - the
  tauri updater is AppImage-only on Linux; the update pill is now suppressed
  for package-manager installs.
- **Windows update flow asks before closing**: installing an update on
  Windows closes the app immediately (NSIS has no staged restart), so the
  pill now gets explicit confirmation first.
- **Support-file extraction is atomic**: staged to a temp dir and moved into
  place with renames, guarded by a process-wide lock, temp files cleaned on
  every path - a mid-extraction kill can no longer leave a silent
  half-extracted eXo/mt32 that every gate treats as complete forever.
- **Extraction watcher gains a disk-size fallback** - librqbit's per-file
  stat can stall short of total for fully-written files, and after a restart
  without session state the stats path never fires at all.
- **Cancelled downloads can't clobber retries**: a stale download_game
  promise from a cancelled attempt no longer overwrites a newer attempt's
  state with false errors.
- **Browse list fetches are epoch-guarded** - a slow background refresh can
  no longer drop an appended page or overwrite newer filter results.
- **Disk-space preflight credits bytes already on disk** - it was blocking
  exactly the resume/re-download recovery flows it should allow.
- **latest.json generation fails the release** if any platform's signed
  updater bundle is missing, instead of silently stranding that platform.
- Booter (`boot disk.img`) LP games no longer fall back to a generated
  autoexec; per-game CRT shader overrides skip DOSBox ECE; the asset
  protocol grant is narrowed to the eXoDOS media subtree; session eviction
  compares paths case-insensitively with proper boundaries on Windows/macOS;
  update check also runs after first-run setup; empty-state flash on cold
  start fixed; Escape guard made ordering-independent (capture phase).

## 0.7.2 - 2026-07-26

### Changed

- **Updates ask first**: a new release shows an "Update" pill in the top bar
  and a one-time toast - nothing downloads until you click it. After
  downloading, the pill turns into "Restart to update" and stays available
  until you're ready.

### Fixed

- **Factory reset clears recently-played history and per-game settings** -
  both previously survived a reset.
- **Manual button explains itself**: instead of silently disappearing when a
  game has no manual, it shows a disabled "No manual" state with a hint that
  manuals ship with the Metadata content pack.

## 0.7.1 - 2026-07-26

### Fixed

- **Support-file extraction survives restarts**: the watcher that extracts
  MT-32 ROMs / the ECE build from util.zip died with the app; if the 630 MB
  download finished in a later session, the assets never landed (observed in
  Windows testing). The watcher now re-arms at startup whenever util.zip is
  selected or on disk and the assets are still missing.
- **First-download feedback**: instead of sitting mute on "Starting
  download..." for minutes while the collection's 14,000 placeholder files
  are created (slow on Windows), the card now explains the one-time setup.
- **Log rotation**: exodium.log rotates at 10 MB keeping one predecessor -
  bounded size, and a wedged session can no longer destroy its own evidence.

## 0.7.0 - 2026-07-26

### Added

- **Auto-update**: Exodium checks GitHub releases at startup, downloads new
  versions in the background (signature-verified), and offers a one-click
  restart. Powered by tauri-plugin-updater; CI publishes a signed
  `latest.json` with every release.
- **DOSBox ECE on Windows**: games tuned for DOSBox ECE (~2,400) now run
  eXo's actual ECE build, extracted on demand from the collection's
  util.zip. On macOS/Linux they keep running under DOSBox Staging, with an
  "experience may vary" note in the game detail panel.
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

### Security / hardening

- **Seeding disclosure + toggle**: the setup flow now says plainly that
  Exodium joins the eXoDOS BitTorrent swarm and uploads while running, and
  Settings → Network gets a "Share with other players" toggle (default on;
  off caps upload at 1 KB/s, applied live and persisted).
- **Single-instance guard**: launching Exodium twice now focuses the existing
  window instead of corrupting the torrent session and contending on the DB.
- **Disk-space preflight**: downloads are refused upfront with a clear message
  when the data dir lacks space for the download plus extraction.
- **Narrowed asset-protocol scope**: was a blanket `**`/`$HOME/**`; now
  `$RESOURCE`/`$APPDATA` statically plus a runtime grant for the user-chosen
  data dir. A production Content-Security-Policy replaces `csp: null`.
- **DOSBox Staging's GPL license text** ships with the bundled binary
  (staged by get-dosbox.sh).

### Fixed

- **LP games launch via overlay mount** - the durable fix for the class of
  bugs where language-pack games flashed and exited (Cobra Mission ES et
  al.). Instead of guessing a launch command from directory contents, the
  EN config's autoexec now runs VERBATIM against a per-launch staging dir
  (`eXo/.exodium_lp/<lang>_<code>/`) whose `<code>` entry is a
  symlink/junction to the LP game dir. eXo's authored launch commands,
  CD imgmounts, and multi-step autoexecs all survive; an installed EN
  variant of the same game is shadowed correctly. A compatibility check
  (cd-chain simulation + launch-command verification) falls back to the
  old generated-autoexec heuristics only when the LP variant genuinely
  restructured the game.
- **Download stall feedback**: a download with no peers no longer sits at
  "0%" forever - after 15 s without progress the card shows "Looking for
  peers…", after 90 s an actionable stall warning. The premature
  "Download didn't start" verdict (which killed polling while the backend
  was still working) now waits for the backend command to actually resolve.
- **Browse keeps your scroll position**: background install/uninstall
  completion no longer resets the infinite-scroll list to page 1.
- **Jump bar stays in sync with search**, genre jump-bar keys can no longer
  point at sections that don't exist, and an empty search shows a
  "No results" state instead of a blank grid.
- **Content-pack install failures surface as toasts** even when the Settings
  dialog is closed (affects the first-run welcome flow).
- **Escape closes one overlay at a time** - manual/lightbox/settings no
  longer take the detail panel down with them.
- **Linux: PDF manuals** open via the system viewer with a clear hint -
  WebKitGTK has no inline PDF renderer, the old iframe stayed blank.
- **MT-32 / General MIDI music for ~2,900 games**: two stacked bugs left MIDI
  games silent or with wrong music. (1) The `!DOSmetadata.zip` download (15 MB:
  Roland MT-32 ROMs + SoundCanvas soundfont) never fired because the bundled
  configs zip pre-created the directory the check gated on - it now gates on
  `eXo/mt32/` itself. (2) ~1,500 configs use DOSBox-ECE key names
  (`[midi] mt32.romdir`, `fluid.soundfont`) that DOSBox Staging ignores -
  launch-time patching now translates them into Staging's `[mt32]` /
  `[fluidsynth]` sections (Staging-authored configs pass through unchanged).
  Correction during field testing: the ROMs actually live in
  `eXo/util/util.zip` (~630 MB), not `!DOSmetadata.zip` - the download is
  now fetched once, only when a game whose config requests MIDI is
  installed, and only the ~30 MB `mt32/` subtree is extracted.
- **38 games downloaded the wrong ZIP** (`find_game_files`): the torrent file
  matcher used an unanchored suffix match, so short titles matched longer ones -
  _Tetris_ fetched _Atomic Tetris_, _Pac-Man_ fetched _Ms. Pac-Man_, _Gods_
  fetched _Dusk of the Gods_, etc. The match is now anchored on a path boundary,
  the bundled DB is regenerated, and a regression test guards the collision set.
- **Versioned catalog refresh**: existing installs never re-read the bundled
  catalog, so fixes like the above (or a new eXoDOS torrent) would only reach
  fresh installs. A `catalog_version` check at startup now updates catalog rows
  in place - user state (installed, library, favorites, per-game config)
  and `games.id` are preserved.
- **Cross-collection placeholder cleanup**: downloading a game 10 s after a
  game from another collection could delete the first torrent's tracked 0-byte
  placeholders (all four collections overlay one root), reintroducing the
  "100% but ZIP missing" loop. Cleanup now keeps the union of all enabled
  collections' file lists.
- **Interrupted downloads resume after restart**: the download manager now
  adopts torrents auto-resumed from librqbit's session persistence (handle +
  file selection), and merges instead of replaces the selection when the
  session already manages a torrent. Previously a download in flight at
  shutdown kept downloading invisibly and the next download silently
  deselected it.
- **Uninstall → re-download stuck at 100%**: uninstalling deletes the game
  ZIP, but librqbit's fastresume bitfield still claimed its pieces existed,
  so a re-download instantly reported 100% with no file on disk. Uninstall
  now drops the torrent from the session (removing its fastresume state) and
  re-adds any still-selected files; the next download re-derives piece state
  from disk.
- **Startup failures show an error dialog** instead of a silent crash
  (unresolvable data dir, unreadable/uninstallable database).
- **LP games launch from unextracted ZIPs**: launch-time auto-extraction only
  looked for the EN ZIP location; it now also checks the language-pack dir.
- **Cancelling a download keeps the shared EN GameData** when another
  language variant of the game is still downloading.
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
