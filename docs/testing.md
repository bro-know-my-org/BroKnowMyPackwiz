# Testing

## Required Checks

Run before handing off code changes:

```powershell
cargo fmt --check
cargo test
cargo build --release
```

Current expected unit test count after the latest changes is 47.

## Smoke Tests Already Run

The following have been verified in temporary directories:

- `add-url` writes new mod metadata to `mods/*.pw.toml`.
- `add-url` does not write new mod metadata to `mods/common/*.pw.toml`.
- `add-resourcepack` writes to `resourcepacks/*.pw.toml`.
- `add-shaderpack` writes to `shaderpacks/*.pw.toml`.
- Resource pack and shader pack metadata are included in `index.toml`.
- `export-curseforge` with `[export.curseforge] latest = true` resolves a real
  CurseForge file through the API and writes it to `manifest.json`.
- Deleting all jars from a copied real pack and running `download-files`
  restored 304 root `mods/*.jar` files with zero nested jars.
- Side directory priority was verified for client/server installs.

## Manual Full-Pack Download Smoke Test

Use a copied pack directory, not the real working pack:

```powershell
.\target\release\bkmpw.exe download-files <temp-pack> 32 --retries 2 --retry-delay-seconds 1
```

Expected behavior:

- Managed jars are restored under `mods/*.jar`.
- No jars are placed under `mods/common`, `mods/client`, or `mods/server`.
- Manual unrelated files are not deleted.

## CurseForge API Smoke Test

Requires `CURSEFORGE_API_KEY` or `[curseforge].api-key`.

Create a temporary pack with a metadata file containing:

```toml
[download]
mode = "metadata:curseforge"

[update.curseforge]
project-id = 238222
file-id = 1

[export.curseforge]
latest = true
```

Then run:

```powershell
.\target\release\bkmpw.exe export-curseforge <temp-pack> <temp-pack>\cf.zip both
```

Expected behavior:

- Export succeeds.
- `manifest.json` contains project ID `238222`.
- `fileID` is resolved dynamically.
