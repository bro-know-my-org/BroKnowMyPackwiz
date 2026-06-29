# Metadata Format

The tool accepts packwiz-style `.pw` and `.pw.toml` files under configured
metadata roots. The default metadata extension for new files is `.pw`.

## Basic Fields

```toml
name = "Example Mod"
filename = "example.jar"
side = "both"
preserve = false
```

`side` may be `both`, `client`, or `server`. Directory placement can override
this for mod metadata in `mods/client`, `mods/server`, and `mods/common`.

## Download By URL

```toml
[download]
url = "https://example.invalid/example.jar"
hash-format = "sha256"
hash = "..."
```

Supported hash formats for verification are `sha1`, `sha256`, and `sha512`.

## CurseForge Download

```toml
[download]
hash-format = "sha1"
hash = "..."
mode = "metadata:curseforge"

[update.curseforge]
project-id = 123456
file-id = 789012
```

Install first tries local files. If it must download from CurseForge, it can
use ForgeCDN fallback and then the official API when an API key exists.

## GitHub Update Source

```toml
[download]
url = "https://github.com/owner/repo/releases/download/v1/example.jar"
hash-format = "sha256"
hash = "..."

[update.github]
project = "owner/repo"
tag = "latest"
asset = "example"
```

`tag` and `asset` are optional. `asset` is matched as a filename fragment.

## CurseForge Export Mapping

Use this when local/dev download should use one source but CurseForge export
should declare a CurseForge manifest file:

```toml
[export.curseforge]
project-id = 123456
file-id = 789012
```

During `export-curseforge`, this file goes into `manifest.json` and the runtime
jar is not copied to `overrides/`.

## CurseForge Latest Export

Use this when a project, such as a coremod, is uploaded to CurseForge by CI and
export should always select the newest file matching the current pack version:

```toml
[download]
mode = "metadata:curseforge"

[update.curseforge]
project-id = 123456
file-id = 789012

[export.curseforge]
latest = true
```

`latest = true` reuses `[update.curseforge].project-id`. You can also write:

```toml
[export.curseforge]
project-id = 123456
latest = true
```

Export resolves Minecraft version and loader from `pack.toml`, then queries
CurseForge for the latest matching file. This requires `CURSEFORGE_API_KEY` or
`[curseforge].api-key`.

## Optional Files

```toml
[option]
optional = true
default = false
```

Optional metadata with `default = false` is skipped by install/sync.

