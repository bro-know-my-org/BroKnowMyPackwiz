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

## Release Preparation (All Four Exports)

Enable isolated release preparation in `.pw/config.toml`:

```toml
[release]
enabled = true
template-dir = "pack"
template-files = "pack.toml,icon.png,start.sh,start.bat,variables.txt,PCL/Setup.ini"
```

`export-client`, `export-curseforge`, `export-server`, and
`export-server-installer` keep their existing arguments. When enabled, each
copies publish inputs into a temporary directory, overlays the listed template
files, validates metadata and target collisions, refreshes the temporary index,
and exports. The source workspace's pack descriptor, index and runtime files
are not modified. Temporary files are removed on success or ordinary failure.
Allow enough temporary disk space for the publish files and local managed files.

Template paths are relative to `template-dir`, and preserve their relative
destination in the archive. Every listed file is required; symlinks, parent
traversal and reserved bookkeeping destinations are rejected. Explicit template
files are included even if their generated copies are gitignored. Template
destinations cannot contain ignore wildcards (`*` or `?`). Runtime template
filenames must match the metadata target spelling, including ASCII case.
Included workspace files that differ from template destinations only by ASCII
case are rejected too. Existing side
and root-overlay rules still apply. Omit both template fields for preparation
without templates. Without `enabled = true`, exports retain their legacy refresh
behavior. No dependency updates or downloads are added: full exports require
their local managed files; the server installer needs no local runtime jars.
CurseForge-mapped files still use manifest IDs, while other managed files are
bundled as overrides even when ignored by the scan rules.

For an explicit local template expansion, run `bkmpw prepare-pack <pack-root>`.
This overwrites the listed development-root files and refreshes the local index.
Copy/refresh failures restore the affected files from backups. If restoration
itself fails, the error reports the retained backup directory for recovery.
Template directories must be separate from managed metadata/runtime directories,
overlays and bookkeeping. Exclude other template source files from the publish
scan (for example, add `pack/` to `.packwizignore`); unlisted publish inputs in a
template directory are rejected. Template sources/destinations cannot be export
outputs. In-pack archive outputs must also avoid root-overlay target paths;
ambiguous output paths are rejected before writing, including ASCII case aliases. Reserved ASCII paths are checked without regard to case for portability.
Project-specific generators, such as CDPR's KubeJS integrity manifest, can run
before export in a thin project script.

JSON exports use the same preparation path. `data.releaseStaged` and
`data.refresh.temporary` identify a temporary index; `data.refresh.indexPath` is null for staged exports
because that index is removed when the command finishes. `prepare-pack` also supports JSON and JSON Lines.
