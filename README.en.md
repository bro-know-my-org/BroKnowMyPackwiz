# bro-know-my-packwiz

[中文 README](README.md)

## Contents

- [Attribution / Credits](#attribution--credits)
- [What This Is](#what-this-is)
- [Differences From Upstream packwiz](#differences-from-upstream-packwiz)
- [Deliberately Not Included](#deliberately-not-included)
- [Commands](#commands)
- [Config](#config)
- [Size Policy](#size-policy)

## Attribution / Credits

**This project is explicitly inspired by the packwiz ecosystem.**

> [!IMPORTANT]
> This project intentionally follows the public packwiz metadata model and
> selected installer workflows. It is a customized Rust implementation tailored
> to this pack's workflow, not a vendored copy of upstream source code.
>
> - packwiz: https://github.com/packwiz/packwiz
> - packwiz-installer: https://github.com/packwiz/packwiz-installer

`bro-know-my-packwiz` is a small Rust tool for this pack's customized
packwiz-style workflow.

Binary name: `bkmpw`.

## What This Is

- A customized Rust implementation of the packwiz-style metadata refresh flow
  and local install/download flow used by this pack.
- A pack-oriented tool with deliberate project-specific behavior and defaults.
- A smaller command surface than upstream packwiz.

## Differences From Upstream packwiz

- Project config is read from `.pw/config.toml`.
- Scan filtering applies `.gitignore` first, then the configured
  `.packwizignore`.
- Metadata layout defaults to side-oriented folders:
  - `mods/server/*.pw.toml`
  - `mods/client/*.pw.toml`
  - `mods/common/*.pw.toml`
  - `mods/*.jar`
- `resourcepacks/*.pw` / `resourcepacks/*.pw.toml` and
  `shaderpacks/*.pw` / `shaderpacks/*.pw.toml` are also scanned; bare
  filenames install next to their metadata file.
- Direct `mods/*.pw.toml` and old `mods/*.pw` metadata are still accepted for
  compatibility and treated as common/both by default.
- `add-url` / `add-curseforge` / `add-github` / `add-file` now write new mod
  metadata to `mods/*.pw.toml` by default instead of auto-sorting into
  `mods/common|client|server`, so developers can move it manually.
- `refresh` does not rewrite an existing metadata file's `side`; declared
  metadata wins over folder placement.
- `install-local` prefers an existing local jar before any network request.
- CurseForge `metadata:curseforge` can use ForgeCDN URL mapping from
  `file-id` + filename before falling back to the official API.
- `add-curseforge` can resolve slugs, URLs, and project IDs through the
  CurseForge API. Without an API key, it can still write metadata when
  project ID, file ID, filename, and hash are supplied explicitly.
- `export-curseforge` writes CurseForge packs as pure Rust store ZIP files
  without adding a zip dependency.
- GitHub download metadata can add `[export.curseforge]`; CurseForge export
  uses that project/file ID while local sync still uses `[download]`.
- `modlist` writes `modlist.md` and `modlist.csv`.
- Parallel install/download jobs are configured by `[install].jobs` or the
  `CDPR_DOWNLOAD_THREADS` environment variable, then optionally overridden by
  the CLI jobs argument.
- Install supports retry, retry delay, and force overwrite behavior for the
  pack's devtool-style batch download workflow.
- Installed files are recorded in a small `packwiz.json` manifest.

## Deliberately Not Included

- Java bootstrapper flow.
- `RequiresBootstrap` startup enforcement.
- MultiMC-specific integration.
- Modrinth provider.
- `utils markdown`.
- CurseForge/Modrinth export and import workflows.
- GUI installer UI.

## Commands

```text
bkmpw init [pack-root]
bkmpw inspect [pack-root]
bkmpw scan [pack-root]
bkmpw refresh [pack-root]
bkmpw list [pack-root]
bkmpw check [pack-root]
bkmpw modlist <pack-root> [output-dir]
bkmpw export-curseforge <pack-root> [output.zip] [side]
bkmpw add-url <pack-root> <side> <name> <filename> <url> <sha256>
bkmpw add-resourcepack <pack-root> <name> <filename> <url> <sha256>
bkmpw add-shaderpack <pack-root> <name> <filename> <url> <sha256>
bkmpw add-curseforge <pack-root> <side> <project|url|project-id> [file-id] [options]
bkmpw add-github <pack-root> <side> <owner/repo|url> [options]
bkmpw add-file <pack-root> <side> <name> <source-file> [filename]
bkmpw pin <pack-root> <name>
bkmpw unpin <pack-root> <name>
bkmpw remove <pack-root> <name>
bkmpw update <pack-root> (--all|<name>) [--mc-version v] [--loader neoforge]
bkmpw download-files <pack-root> [jobs] [--force] [--retries n] [--retry-delay-seconds n]
bkmpw sync <source-root> <target-root> [side] [jobs] [--force] [--retries n] [--retry-delay-seconds n]
bkmpw install-files-headless <pack-root> [attempts] [delay-seconds]
bkmpw install-files-retry <pack-root> [attempts] [delay-seconds]
bkmpw install-local <source-root> <target-root> <side> [jobs] [--force] [--retries n] [--retry-delay-seconds n]
bkmpw hash <sha1|sha256|sha512|murmur2> <file>
```

## Config

Default config lives at `.pw/config.toml`:

```toml
[scan]
use-gitignore = true
packwizignore = ".packwizignore"

[layout]
metadata-root = "mods"
metadata-roots = "mods,resourcepacks,shaderpacks"
jar-root = "mods"
server-meta = "mods/server"
client-meta = "mods/client"
common-meta = "mods/common"
metadata-extension = "pw.toml"

[install]
jobs = 8
retries = 1
retry-delay-seconds = 5
force = false
split-download-min-bytes = 16777216
split-download-chunks = 4

[curseforge]
api-key = ""
cdn-fallback = true
```

`install-local ... [jobs]` overrides `[install].jobs` when provided.
`CDPR_DOWNLOAD_THREADS` also overrides configured jobs, and CLI jobs override
the environment variable.

Install options:

- `--force` / `[install].force = true`: ignore preserve and existing-file hash reuse, then copy or download again.
- `--retries n` / `[install].retries`: attempts per file, default 1.
- `--retry-delay-seconds n` / `[install].retry-delay-seconds`: seconds to wait before the next attempt, default 5.
- `[install].split-download-min-bytes`: try HTTP Range chunk downloads when a file is larger than this byte count, default 16777216, or 16 MiB.
- `[install].split-download-chunks`: chunks per large file, default 4; set it to 1 to effectively disable split downloads. Servers without Range support automatically fall back to the normal single stream.
- `download-files`: fills files referenced by metadata inside the pack root and never removes unrelated files.
- `sync` / `install-files-headless` / `install-files-retry`: sync managed files and clean files from the previous `packwiz.json` that are no longer needed under `mods/`, `resourcepacks/`, or `shaderpacks/`; runtime files such as saves and logs are not cleaned.
- `hash` supports `sha1`, `sha256`, `sha512`, and CurseForge `murmur2`.

Resource pack and shader pack add:

```text
bkmpw add-resourcepack <pack-root> <name> <filename> <url> <sha256>
bkmpw add-shaderpack <pack-root> <name> <filename> <url> <sha256>
```

These commands write `resourcepacks/*.pw.toml` / `shaderpacks/*.pw.toml`;
metadata `side` defaults to `client`, and a bare `filename` downloads next to
the metadata file.

GitHub Release add:

```text
bkmpw add-github <pack-root> <side> <owner/repo|url> [--tag tag] [--asset text] [--name n] [--filename f] [--cf-project-id id --cf-file-id id]
```

The command reads the latest release by default. If a release has multiple assets, use `--asset` to select by filename fragment.
`--cf-project-id` and `--cf-file-id` must be provided together and write an
export-only mapping:

```toml
[download]
url = "https://github.com/owner/repo/releases/download/v1/core.jar"
hash-format = "sha256"
hash = "..."

[update.github]
project = "owner/repo"
tag = "latest"
asset = "core"

[export.curseforge]
project-id = 123456
file-id = 789012
```

This stays backward compatible with existing metadata. Without
`[export.curseforge]`, behavior is unchanged. With it, `download-files` /
`sync` still use `[download]`; only `export-curseforge` writes the file to
`manifest.json` and keeps the runtime jar out of `overrides/`.

If a coremod is automatically uploaded to CurseForge by GitHub Actions, export
can resolve the latest CurseForge file matching the pack's Minecraft version
and loader:

```toml
[download]
mode = "metadata:curseforge"

[update.curseforge]
project-id = 123456
file-id = 789012

[export.curseforge]
latest = true
```

`latest = true` reuses `[update.curseforge].project-id`; you can also set
`project-id = 123456` directly under `[export.curseforge]`. Export needs
`CURSEFORGE_API_KEY` or `[curseforge].api-key` because it queries the latest
matching file.

Update:

```text
bkmpw update <pack-root> --all
bkmpw update <pack-root> <name>
```

- CurseForge metadata uses `[update.curseforge]` to query the latest matching file and update `filename`, hash, and `file-id`; it needs `CURSEFORGE_API_KEY` or `[curseforge].api-key`.
- GitHub metadata uses `[update.github]` to query the latest release or a pinned tag. Only metadata created by the newer `add-github` command includes this update source automatically.
- Plain URL metadata has no inferable upstream version and is skipped by `--all`.

CurseForge API keys can also be supplied with `CURSEFORGE_API_KEY`. Local jars
are used first, so a complete pack does not need any key at install time.

With `cdn-fallback = true`, CurseForge file IDs are first mapped to ForgeCDN
URLs like `https://edge.forgecdn.net/files/...`. If that fails and an API key
exists, the official API is tried.

Common `add-curseforge` forms:

```text
# With CURSEFORGE_API_KEY, resolve by slug/URL/project ID and pick the latest file
bkmpw add-curseforge . both always-eat
bkmpw add-curseforge . both https://www.curseforge.com/minecraft/mc-mods/always-eat
bkmpw add-curseforge . both 1259229

# Pin a file ID
bkmpw add-curseforge . both 1259229 6498183

# Without an API key, provide the required metadata explicitly
bkmpw add-curseforge . both 1259229 6498183 --name "Always Eat" --filename "AlwaysEat-neoforge-1.0.0.jar" --hash "5948137fed00d7bf3fe688e1aecc310b4fa5aaf7"
```

Slug/URL resolution needs `CURSEFORGE_API_KEY` or `[curseforge].api-key`. Without
a key, `project-id + file-id + filename + hash` still produces installable and
exportable metadata.

`export-curseforge <pack-root> [output.zip] [side]` refreshes first, then writes
a CurseForge-format zip: `metadata:curseforge` files and files with
`[export.curseforge]` go into `manifest.json`; config and other publish files go
under `overrides/`. If `[export.curseforge] latest = true`, the file ID is
resolved from the Minecraft version and loader in `pack.toml`. The zip uses
store mode instead of compression to avoid extra dependencies.

## Size Policy

- Release builds optimize for size.
- HTTP/HTTPS downloads use `ureq` + `rustls`.
- CurseForge API response handling avoids a JSON dependency and extracts only
  the `downloadUrl` field needed for install.

## License

MIT.
