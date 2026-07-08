# Workflows

## Build

```powershell
cargo fmt --check
cargo test
cargo build --release
```

Use the release binary for manual smoke tests:

```powershell
.\target\release\bkmpw.exe --help
```

## Initialize A Pack

```powershell
bkmpw init <pack-root>
```

This creates the project config and baseline pack files if missing.

## Add Mods

Direct URL:

```powershell
bkmpw add-url <pack-root> both "Mod Name" "mod.jar" "https://example/mod.jar" "<sha256>"
```

GitHub release:

```powershell
bkmpw add-github <pack-root> both owner/repo --asset mod --tag latest
```

GitHub release with CurseForge export mapping:

```powershell
bkmpw add-github <pack-root> both owner/repo --asset core --cf-project-id 123456 --cf-file-id 789012
```

CurseForge:

```powershell
bkmpw add-curseforge <pack-root> both <slug-or-project-id>
```

All mod add commands write new metadata to `mods/*.pw.toml`. Move the metadata
file into `mods/client`, `mods/server`, or `mods/common` manually when side
placement should be authoritative.

## Add Resource Packs And Shader Packs

```powershell
bkmpw add-resourcepack <pack-root> "Pack Name" "pack.zip" "https://example/pack.zip" "<sha256>"
bkmpw add-shaderpack <pack-root> "Shader Name" "shader.zip" "https://example/shader.zip" "<sha256>"
```

These commands write metadata to `resourcepacks/*.pw.toml` and `shaderpacks/*.pw.toml`.

## Refresh Metadata

```powershell
bkmpw refresh <pack-root>
```

This writes `index.toml` and updates the pack index hash in `pack.toml`.

## Fill Files In A Pack

```powershell
bkmpw download-files <pack-root> 32 --retries 2 --retry-delay-seconds 1
```

`download-files` does not clean unrelated files.

Files larger than `[install].split-download-min-bytes` default to a
Range-based split download with `[install].split-download-chunks` chunks.
Servers that do not support Range requests automatically use the normal
single-stream download path.

Downloads and local installs write to target-adjacent temporary files first,
then move completed files into place. If the process is interrupted by Ctrl+C
or killed, the final jar/zip path is not used for partial content. Later
`download-files`, `sync`, and `install-files-*` runs automatically remove stale
`bkmpw` temporary files only when they are at least one hour old and the
matching run lock under `.bkmpw/runs/` has also stopped refreshing for at least
one hour. Cleanup failures are reported as warnings and do not block the
install.

## Sync To A Runtime Folder

```powershell
bkmpw sync <source-root> <target-root> both 32 --retries 2 --retry-delay-seconds 1
```

Use `client` or `server` instead of `both` for side-specific installs.

## Update Metadata

Update everything:

```powershell
bkmpw update <pack-root> --all
```

Update one metadata entry:

```powershell
bkmpw update <pack-root> "Mod Name"
```

CurseForge update needs `CURSEFORGE_API_KEY` or `[curseforge].api-key`.

## Export CurseForge Pack

```powershell
bkmpw export-curseforge <pack-root> <output.zip> both
```

If any metadata uses `[export.curseforge] latest = true`, export needs
`CURSEFORGE_API_KEY` or `[curseforge].api-key`.

## Prepare Pack

The old devtool `prepare-pack` step is intentionally not reimplemented as Rust
CLI behavior yet. Keep it as a thin script around the current CLI commands if
the pack still needs that release workflow.
