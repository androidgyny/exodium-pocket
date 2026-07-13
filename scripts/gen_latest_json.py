#!/usr/bin/env python3
"""Assemble the tauri-updater latest.json from CI build artifacts.

Usage: gen_latest_json.py <artifacts_dir> <tag> <owner/repo>

Walks the downloaded artifacts, finds each platform's updater bundle and its
.sig (produced by `tauri build` when TAURI_SIGNING_PRIVATE_KEY is set and
bundle.createUpdaterArtifacts is true), and prints latest.json to stdout.
The URLs point at the GitHub release assets for <tag>, so this file must be
uploaded to the same release.
"""
import datetime
import json
import os
import sys
import urllib.parse

art_dir, tag, repo = sys.argv[1:4]
version = tag.lstrip("v")


def find(suffix: str):
    out = []
    for root, _, files in os.walk(art_dir):
        for f in files:
            if f.endswith(suffix):
                out.append(os.path.join(root, f))
    return sorted(out)


def entry(suffix: str):
    for f in find(suffix):
        sig = f + ".sig"
        if os.path.exists(sig):
            name = urllib.parse.quote(os.path.basename(f))
            return {
                "signature": open(sig).read().strip(),
                "url": f"https://github.com/{repo}/releases/download/{tag}/{name}",
            }
    return None


platforms = {}
mac = entry(".app.tar.gz")
if mac:
    platforms["darwin-aarch64"] = mac
lin = entry(".AppImage")
if lin:
    platforms["linux-x86_64"] = lin
win = entry("-setup.exe") or entry(".msi")
if win:
    platforms["windows-x86_64"] = win

if not platforms:
    sys.exit("no signed updater bundles found - is TAURI_SIGNING_PRIVATE_KEY set?")

json.dump(
    {
        "version": version,
        "notes": f"Exodium {version} - see the release page for details.",
        "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime(
            "%Y-%m-%dT%H:%M:%SZ"
        ),
        "platforms": platforms,
    },
    sys.stdout,
    indent=2,
)
print()
