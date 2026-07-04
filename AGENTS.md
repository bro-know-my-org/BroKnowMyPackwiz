# AGENTS.md

## Project

`bro-know-my-packwiz` is a Rust CLI for a customized packwiz-style workflow.
The binary name is `bkmpw`.

Keep the CLI workflow stable and scriptable. GUI work is deferred; if added
later, put GUI code on top of the same core logic rather than replacing the CLI.

## Build And Test

Run these before handing off code changes:

```text
cargo fmt --check
cargo test
cargo build --release
```

For local CLI smoke tests, use the release binary:

```text
.\target\release\bkmpw.exe --help
```

## Metadata Layout

- Metadata roots are `mods`, `resourcepacks`, and `shaderpacks`.
- New mod metadata from `add-url`, `add-curseforge`, `add-github`, and
  `add-file` is written to `mods/*.pw.toml`, not auto-sorted into
  `mods/common`, `mods/client`, or `mods/server`.
- Existing `.pw` metadata remains supported for compatibility.
- Developers can manually move mod metadata into `mods/common`,
  `mods/client`, or `mods/server`; directory placement has priority over the
  `side` field during install/export.
- Mod jars are installed to `mods/*.jar`, sibling to the side metadata
  directories.
- Resource pack metadata is written to `resourcepacks/*.pw.toml`.
- Shader pack metadata is written to `shaderpacks/*.pw.toml`.
- Bare filenames in resource pack and shader pack metadata install next to
  their metadata file.

## Cleanup Rules

- `download-files` fills missing managed files in the pack root and does not
  clean unrelated files.
- `sync`, `install-files-headless`, and `install-files-retry` clean only files
  previously recorded in `packwiz.json` under `mods/`, `resourcepacks/`, and
  `shaderpacks/`.
- Manual jars not recorded in `packwiz.json` must not be deleted.

## CurseForge

- CurseForge API key is read from `CURSEFORGE_API_KEY` or
  `[curseforge].api-key`.
- Core API requests use the `x-api-key` header.
- `metadata:curseforge` remains the standard CurseForge download mode.
- `[export.curseforge] project-id + file-id` lets a GitHub/local download
  source export as a CurseForge manifest file instead of an override.
- `[export.curseforge] latest = true` resolves the latest file matching
  `pack.toml` Minecraft version and loader during `export-curseforge`.
- `latest = true` can reuse `[update.curseforge].project-id` if
  `[export.curseforge].project-id` is omitted.
