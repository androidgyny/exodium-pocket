# Handover: Win9x-Emulatoren als Content-Packs + DOSBox-X-AppImage für Linux

Stand 2026-08-07, Branch `feat/exowin9x`, alles UNCOMMITTED (13 geänderte +
3 neue Dateien, siehe `git status`). Implementiert und auf macOS verifiziert;
dieses Dokument ist die Übergabe an eine Linux-Session für Runtime-Tests und
den Workflow-Erstlauf. Plan-Referenz: die Design-Entscheidungen stehen in
CLAUDE.md §10/§16 (bereits aktualisiert).

## Was gebaut wurde

1. **Manifest-Schema** (`updates.rs`): `PlatformSource` + optionales
   `platforms`-Map pro Pack (`darwin-aarch64` / `linux-x86_64`, Tokens wie
   gen_latest_json.py). `for_current_platform()` substituiert url/sha256/size
   an drei Stellen (`list_content_packs`, `install_content_pack`,
   `adopt_packs_on_disk`); ohne Eintrag ist das Pack auf der Plattform
   unsichtbar (Windows sieht die Emulator-Packs nie). `manifest.json` trägt
   `eXoWin9x.content_packs.dosbox-x` + `.86box` mit TODO-URLs → "coming
   soon", solange content-v6 nicht existiert.
2. **Resolver** (`win9x.rs`): `pack_candidate()` probt
   `<data_dir>/content/emulators/<pack>/…` (Dateisystem, nie Ledger).
   Reihenfolge Linux dosbox-x: PATH-mit-cap_net_raw → Pack-AppImage →
   resource_candidate (Übergang) → PATH → Flatpak; 86box: Pack → resource →
   PATH. Dev-Builds proben zusätzlich `CARGO_MANIFEST_DIR/resources/`
   (get-emulators.sh bleibt Dev-only). Cap-Probe läuft jetzt per `getcap`
   auf dem Binary statt AF_PACKET im Exodium-Prozess (der Prozess-Probe-Bug:
   Datei-Capability sitzt auf dem Emulator, nicht auf uns) — treibt auch
   `ne2000_override` (pcap vs slirp) und `win9x_network_status`.
   `resolved_dosbox_x_path()` (setcap-Ziel) ist PATH-only und lehnt das
   Pack-Binary mit Begründung ab (secure-exec bricht sharun-Bundles).
3. **Auto-Queue**: `start_pack_install(app, col, pack)` aus dem Command
   extrahiert; `download_game` queued das passende Pack (Variante→Pack via
   `emulator_pack_for_variant`, Resolver-gated, Preflight +size×2.2) und
   emittet `content-pack-install-started` — erster Tauri-Event der App.
4. **Frontend**: `initContentPackEvents()` (App.tsx-Mount) macht
   Backend-Jobs im ActivityBadge sichtbar; die engine-missing-Note im
   GameDetailPanel wird zum "Download emulator (N MB)"-Button mit
   Live-Prozent, offline nur Hinweis; Note heilt sich über
   `installedPacks()`-Effect selbst.
5. **Build-Pipeline**: `.github/workflows/content-packs.yml`
   (workflow_dispatch): Job 1 baut DOSBox-X **2025.02.01 aus Source** im
   pkgforge-Arch-Container und packt per quick-sharun/uruntime zum
   anylinux-AppImage (Self-Updater raus, libslirp/libpcap-Gate); Jobs 2/3
   rollen die vier Tarballs (`scripts/build-emulator-packs.sh`: Wrapper-Dir
   = Pack-Id, COPYFILE_DISABLE, codesign VOR dem Tarren, chmod +x); Job 4
   verifiziert (Binary-Pfad+x, keine `._*`, .app-Symlinks intakt) und
   emittiert `manifest-snippet.json` + SHA256SUMS. Nur Artefakte, kein
   Release.
6. **Entbündelt**: `tauri.conf.json` ohne `dosbox-x`/`86box`-Resources,
   build.yml ohne Fetch+Verify-Steps (−344 MB macOS, −90 MB Linux-Installer),
   get-emulators.sh Dev-only (+ 2 Bugfixes: stale-stamp-Skip, `$DBX_ARCHIVE`
   unbound). `copy_dir_recursive` erhält Symlinks (EXDEV-Fallback).

## Verifiziert (macOS)

- `cargo clippy --all-targets -- -D warnings` sauber, `cargo test` 131 grün
  (neu: platforms-Suite, install_path-Uniqueness-Lint, Adoption-Skip,
  Symlink-Copy), `tsc --noEmit` sauber, `vitest` 116 grün.
- Dev-App bootet sauber, alle 6 Manager initialisiert (UI-Smoke abgebrochen —
  Maschine war in Benutzung).

## Für die Linux-Session

**A. Sanity nach Sync**: `cargo clippy --manifest-path src-tauri/Cargo.toml
--all-targets -- -D warnings` && `cargo test` && `pnpm test`.

**B. Workflow-Erstlauf** (braucht Push des Branches):
`gh workflow run content-packs.yml -r feat/exowin9x`. Erwartete Reibung
(budgetiert): der 2025.02.01-Tag unter Arch-aktuellem GCC — ggf.
`CXXFLAGS=-std=gnu++17` in `scripts/build-dosbox-x-appimage.sh`, oder Deps
nachziehen. Lokal iterierbar ohne CI:
`podman run -it ghcr.io/pkgforge-dev/archlinux:latest` + Setup-Schritte aus
pkgforge-dev/anylinux-setup-action, dann `sh scripts/build-dosbox-x-appimage.sh`.
Das libslirp/libpcap-Gate NICHT aufweichen — 67 Netzwerk-Parent-Spiele booten
über slirp.

**C. Runtime-Matrix Linux** (Artefakte aus B, lokal in
`<data_dir>/content/emulators/` entpacken oder per Settings installieren,
sobald URLs/Hashes in manifest.json gefüllt sind — für lokale Tests reicht
Entpacken, `pack_candidate` ist ein reiner Dateisystem-Probe):
1. AppImage läuft auf der Distro (uruntime, ohne FUSE): Connect4 (1995)
   end-to-end, danach Masque Solitaire Antics (IDE/CD-Pfad).
2. Resolver-Ordnung: Pack schlägt nacktes PATH-dosbox-x; `sudo setcap
   cap_net_raw+ep $(which dosbox-x)` → PATH gewinnt (Multiplayer-Präferenz);
   `setcap -r` → Pack wieder vorn.
3. `win9x_network_status`: ohne System-dosbox-x muss die Settings-Zeile den
   "braucht System-Installation"-Text zeigen, kein Enable-Button.
4. 86Box.AppImage aus dem Pack (Boso View Express), `pkexec`-Flows,
   Flatpak-Fallback (Pack-Verzeichnis wegbenennen).
5. Offline-Modus: Panel-Button disabled-mit-Hinweis, kein Job.

**D. Publish-Choreografie** (Reihenfolge ist hart, CLAUDE.md §10):
1. Artefakte smoke-testen (macOS-Tarballs auf dem Mac: Settings-Install +
   Launch).
2. `gh release create content-v6 --latest=false` mit 4 Tarballs + beiden
   Source-Tarballs (GPL) + SHA256SUMS — VOR jedem App-Release, das darauf
   zeigt.
3. manifest.json-TODOs aus `manifest-snippet.json` füllen (Basis-URL
   content-vN → content-v6 ersetzen), Tarballs danach NIE neu rollen
   (mtime→Hash). Dann App-Release (0.12.0) mit gefülltem Manifest +
   schlankem Installer. Release nur nach Freigabe durch Thomas.

## Offene Punkte / Risiken

- Erstlauf des Workflows ungetestet (Syntax/YAML validiert, Logik nicht).
- 86Box-6.0-macOS-Zip als "universal" angenommen (Explorer-Befund) — beim
  Smoke-Test bestätigen.
- Release-Notes müssen erwähnen: bestehende macOS/Linux-Installs laden den
  Emulator nach dem Update einmalig als Pack nach (52–292 MB je nach Bedarf);
  die alten Kopien sterben mit dem ersetzten Bundle, Doppel-Download
  unmöglich.
