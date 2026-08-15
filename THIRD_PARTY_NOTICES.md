# Third-Party Notices

Exodium Pocket includes or depends on third-party open source software. Dependency versions are pinned in `pnpm-lock.yaml` and `src-tauri/Cargo.lock`; the authoritative license terms for each package are provided by the respective package distributions.

## Included Application Dependencies

- Tauri and Tauri plugins
- SolidJS
- Ark UI
- Vite, TypeScript, and Vitest development tooling
- Rust crates listed in `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock`, including SQLite, torrent, archive, HTTP, image, logging, and async-runtime libraries

These dependencies are included under their own licenses. Most are permissively licensed, but each package's own license file remains controlling.

## External Runtime Requirements

The following are required for Android gameplay but are not bundled with Exodium Pocket:

- RetroArch for Android
- DOSBox Pure libretro core

Users must install and configure these separately. Their projects, licenses, trademarks, and distribution terms are independent from Exodium Pocket.

## External Content

Exodium Pocket can browse metadata and download content from external eXoDOS-related torrents selected by the user. The repository does not include downloaded games, ROMs, manuals, screenshots, videos, poster packs, preview JPEGs, soundfonts, Roland ROMs, BIOS images, or other copyrighted game/media content.

Users are responsible for ensuring that their use of external content is lawful and permitted by the applicable rights holders.

## Upstream Attribution

Exodium Pocket is derived from Exodium by Thomas Vollstädt, licensed under MIT. See [NOTICE.md](NOTICE.md) and [LICENSE](LICENSE).
