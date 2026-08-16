# Exodium Pocket

Exodium Pocket is an unofficial Android fork of [Thomas Vollstädt's Exodium](https://github.com/tvollstaedt/exodium), a Tauri/SolidJS launcher for the eXoDOS collections.

This fork targets Android handhelds and launches installed DOS games through a separately installed RetroArch app with the DOSBox Pure core. It is based on upstream Exodium commit `c815d5143bdbf0b8023115b1210f4b7c32e8fffc`.

## Project Expectations

Exodium Pocket is a vibe-coded personal project, built with ChatGPT to make Exodium usable on my own Android device. I am publishing the source in case it is useful to others, but it should be treated as an unpolished personal fork rather than a supported product.

There is no guarantee of compatibility, support, releases, fixes, documentation updates, or future improvements. Expect sharp edges.

## Status

Exodium Pocket is an MVP Android port. It keeps the upstream catalog, torrent download flow, library UI, and installed-game tracking, while adding Android resource handling and a small native bridge that asks RetroArch to launch downloaded game ZIPs.

The current Android support is focused on DOS/eXoDOS games. Windows 3.x and Windows 9x desktop launch paths remain in the codebase for upstream compatibility, but the Android MVP does not launch those collections.

## External Requirements

Exodium Pocket does not bundle emulators, games, ROMs, or media packs.

Install these separately on the Android device:

- RetroArch for Android.
- The DOSBox Pure libretro core inside RetroArch.

By default the app launches package `com.retroarch`, activity `com.retroarch.browser.retroactivity.RetroActivityFuture`, and core path `/data/data/com.retroarch/cores/dosbox_pure_libretro_android.so`. These values are stored in app config so device-specific builds can override them.

## Android Storage Requirement

Exodium Pocket requires Android "All files access" so it can create and manage a shared game library folder that both Exodium Pocket and RetroArch can resolve.

The current Android port still uses old-school filesystem paths rather than a fully scoped-storage or Storage Access Framework workflow. In practice, that means the app needs permission to use `/storage/emulated/0/ExodiumPocket` and similar shared folders for downloads, artwork, and RetroArch launches.

After installing an APK, grant Exodium Pocket "All files access" in Android settings. Without it, downloads, artwork installation, disk-space checks, or game launches may fail even when ordinary storage permissions appear to be enabled.

## What Is Included

- Android-specific Tauri configuration for the `app.exodiumpocket` package.
- A small `tauri-plugin-retroarch-launcher` bridge that sends `ROM`, `LIBRETRO`, and RetroArch path extras to RetroArch.
- Bundled catalog metadata and torrent files needed to browse and selectively download supported eXoDOS content.
- The upstream MIT license and attribution.

## What Is Not Included

- Generated Android build output.
- APKs or Android App Bundles.
- JNI/native build artifacts.
- Downloaded game archives.
- Downloaded screenshots, videos, poster packs, thumbnails, or preview JPEGs.
- DOSBox, RetroArch, DOSBox Pure, BIOS/ROM images, Roland ROMs, or copyrighted game/media assets.

## Development

Prerequisites:

- pnpm
- Rust toolchain
- Android SDK/NDK configured for Tauri Android builds
- RetroArch and DOSBox Pure installed on the target Android device for runtime launch testing

Install dependencies:

```bash
pnpm install
```

Run checks:

```bash
pnpm test
pnpm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml
```

Build/run with the Tauri Android workflow appropriate for your local SDK setup.

For a local sideloadable Android APK:

```bash
pnpm android:init
pnpm android:build:apk
```

The Android project under `src-tauri/gen/android` is generated and not published. The APK build script patches the generated manifest so the local APK requests the All Files Access permission needed by the current filesystem-based Android workflow.

## Upstream

Exodium Pocket is not affiliated with or endorsed by upstream Exodium, RetroArch, DOSBox Pure, DOSBox, libretro, eXoDOS, or the eXo project.

Upstream Exodium remains available at [tvollstaedt/exodium](https://github.com/tvollstaedt/exodium). Please direct Exodium Pocket issues to [androidgyny/exodium-pocket](https://github.com/androidgyny/exodium-pocket).

## Legal

The application code is MIT licensed. See [LICENSE](LICENSE), [NOTICE.md](NOTICE.md), and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Exodium Pocket does not grant rights to download, distribute, or play any third-party game or media content. Users are responsible for complying with applicable laws and licenses for any external content, emulator, core, BIOS/ROM, soundfont, or game files they install or download.
