# Architecture

`bkmpw` is a single Rust CLI with small modules for each workflow. Keep the CLI
surface pragmatic and preserve compatibility with existing packwiz-style `.pw`
metadata used by the pack.

## Module Map

- `main.rs`: CLI parsing and command dispatch. There is no CLI framework.
- `config.rs`: reads `.pw/config.toml` and applies project defaults.
- `scan.rs`: walks the pack root, applies `.gitignore` and `.packwizignore`,
  and classifies metadata, jars, included files, and excluded files.
- `metadata.rs`: parses packwiz-style `.pw` metadata plus local extensions.
- `refresh.rs`: writes `index.toml` and updates the `[index]` hash in
  `pack.toml`.
- `install.rs`: implements `download-files`, `sync`, and installer-style local
  install. It owns target path resolution and cleanup rules.
- `ops.rs`: creates and edits metadata files.
- `update.rs`: updates CurseForge and GitHub metadata.
- `export_cf.rs`: exports CurseForge ZIPs and maps metadata to manifest files or
  overrides.
- `curseforge.rs` / `github.rs`: provider-specific HTTP logic.
- `zipstore.rs`: minimal store-only ZIP writer used by CurseForge export.

## Layout Rules

Default metadata roots are:

```text
mods
resourcepacks
shaderpacks
```

Mod side buckets are supported:

```text
mods/common/*.pw
mods/client/*.pw
mods/server/*.pw
```

New mod metadata from `add-url`, `add-curseforge`, `add-github`, and `add-file`
must be written to `mods/*.pw`. Do not auto-sort new metadata into side
buckets; developers move files manually when they want a stronger side signal.

Directory placement has priority over the metadata `side` field:

- `mods/client/*.pw` installs/exports as client.
- `mods/server/*.pw` installs/exports as server.
- `mods/common/*.pw` installs/exports as both.
- Direct `mods/*.pw` falls back to the metadata `side`, or both when unset.

Runtime mod jars are installed to `mods/*.jar`, not inside `mods/common`,
`mods/client`, or `mods/server`.

Resource pack and shader pack metadata use:

```text
resourcepacks/*.pw
shaderpacks/*.pw
```

Bare filenames in these roots install next to their metadata file.

## Cleanup Rules

`download-files` fills missing managed files in the source pack root and does
not clean unrelated files.

`sync`, `install-files-headless`, and `install-files-retry` may clean managed
files, but only those recorded by the previous `packwiz.json` and only under:

```text
mods/
resourcepacks/
shaderpacks/
```

Manual jars or runtime files that are not in `packwiz.json` must not be deleted.

## CurseForge Export

CurseForge export writes:

- CurseForge-managed files to `manifest.json`.
- Config and non-CF publish files to `overrides/`.

Supported CF metadata sources:

```toml
[download]
mode = "metadata:curseforge"

[update.curseforge]
project-id = 123456
file-id = 789012
```

Local extension for split local download/export:

```toml
[export.curseforge]
project-id = 123456
file-id = 789012
```

Local extension for automatic latest export:

```toml
[export.curseforge]
latest = true
```

When `latest = true`, export resolves the latest file matching `pack.toml`
Minecraft version and loader. It uses `[export.curseforge].project-id` when
present, otherwise `[update.curseforge].project-id`.

## GUI Direction

GUI is deferred until the CLI workflow is complete. If added later, keep
`bkmpw` as the scriptable CLI and add a separate GUI binary over shared Rust
logic. Prefer `egui`/`eframe`; only use Tauri if the app needs web-style UI
tooling.

