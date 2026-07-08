# bro-know-my-packwiz

[English README](README.en.md)

## 目录

- [致谢与来源说明](#致谢与来源说明)
- [这是什么](#这是什么)
- [和上游 packwiz 的差异](#和上游-packwiz-的差异)
- [刻意不包含的功能](#刻意不包含的功能)
- [命令](#命令)
- [配置](#配置)
- [体积策略](#体积策略)

## 致谢与来源说明

**本项目明确受到 packwiz 生态启发。**

> [!IMPORTANT]
> 本项目沿用了公开的 packwiz 元数据模型，并借鉴了 packwiz-installer 的
> 部分安装工作流。它是为当前包工作流定制的 Rust 实现，不是对上游源码
> 的直接复制或 vendored copy。
>
> - packwiz: https://github.com/packwiz/packwiz
> - packwiz-installer: https://github.com/packwiz/packwiz-installer

`bro-know-my-packwiz` 是一个面向当前包工作流的定制化 Rust 工具。

二进制名称：`bkmpw`。

## 这是什么

- 针对当前包使用方式定制的 packwiz-style 元数据刷新与本地安装/下载工具。
- 带有明确项目默认行为的 pack-oriented 工具。
- 命令面比上游 packwiz 更小，只保留当前工作流需要的部分。

## 和上游 packwiz 的差异

- 项目配置从 `.pw/config.toml` 读取。
- 扫描过滤时先应用 `.gitignore`，再应用配置里的 `.packwizignore`。
- 元数据默认采用按 side 分桶的目录：
  - `mods/server/*.pw.toml`
  - `mods/client/*.pw.toml`
  - `mods/common/*.pw.toml`
  - `mods/*.jar`
- 也会扫描 `resourcepacks/*.pw` / `resourcepacks/*.pw.toml` 和
  `shaderpacks/*.pw` / `shaderpacks/*.pw.toml`，裸文件名会安装到 metadata
  所在目录。
- 兼容直接放在 `mods/*.pw.toml` 的元数据，也兼容旧式 `mods/*.pw`；默认按
  common/both 处理。
- `add-url` / `add-curseforge` / `add-github` / `add-file` 新增的 mod metadata
  默认写到 `mods/*.pw.toml`，不自动塞进 `mods/common|client|server`，方便开发者再手动分桶。
- `refresh` 不会根据文件夹位置改写已有 metadata 的 `side`；metadata 里
  已声明的值优先。
- `install-local` 优先使用已存在的本地 jar，缺失时才联网。
- CurseForge `metadata:curseforge` 会先尝试用 `file-id` + 文件名映射
  ForgeCDN 地址，再按需回退到官方 API。
- `add-curseforge` 支持通过 CurseForge API 解析 slug/URL/project ID；也支持在
  没有 API key 时显式传入 project-id、file-id、filename 和 hash 直接落元数据。
- `export-curseforge` 使用纯 Rust store ZIP 写出 CurseForge 包，不引入 zip 依赖。
- GitHub 下载源可以额外声明 `[export.curseforge]`，导出 CF 包时改用
  CurseForge project/file ID，本地同步仍按 `[download]` 下载。
- `modlist` 可生成 `modlist.md` 和 `modlist.csv`。
- 并行安装/下载数量由 `[install].jobs`、`CDPR_DOWNLOAD_THREADS` 或 CLI 的
  可选 jobs 参数控制，优先级依次升高。
- 安装支持 retry、重试等待时间和 force 覆盖，方便吸收原 devtool 的批量下载习惯。
- 安装结果会写入一个小型 `packwiz.json` manifest。

## 刻意不包含的功能

- Java bootstrapper 流程。
- `RequiresBootstrap` 启动强制检查。
- MultiMC 专用集成。
- Modrinth provider。
- `utils markdown`。
- CurseForge/Modrinth 导出和导入流程。
- GUI installer UI。

## 命令

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

## 配置

默认配置位于 `.pw/config.toml`：

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

`install-local ... [jobs]` 提供 jobs 参数时，会覆盖 `[install].jobs`。
`CDPR_DOWNLOAD_THREADS` 也可以覆盖配置里的 jobs，CLI jobs 又会覆盖环境变量。

安装相关开关：

- `--force` / `[install].force = true`：忽略 preserve 和已有文件哈希复用，重新复制或下载。
- `--retries n` / `[install].retries`：每个文件的尝试次数，默认 1。
- `--retry-delay-seconds n` / `[install].retry-delay-seconds`：失败后下一次尝试前等待秒数，默认 5。
- `[install].split-download-min-bytes`：HTTP 文件大于该字节数时尝试 Range 分块下载，默认 16777216，即 16 MiB。
- `[install].split-download-chunks`：单个大文件的分块数，默认 4；设为 1 可等效关闭分块下载。服务器不支持 Range 时会自动回退普通单流下载。
- `download-files`：在 pack root 内补齐 metadata 指向的文件，不清理任何无关文件。
- `sync` / `install-files-headless` / `install-files-retry`：同步受管文件，会根据上一次 `packwiz.json` 清理已不再需要的 `mods/`、`resourcepacks/`、`shaderpacks/` 文件，不清理存档、日志等运行产物。
- `hash` 命令支持 `sha1`、`sha256`、`sha512` 和 CurseForge `murmur2`。

资源包和光影包添加：

```text
bkmpw add-resourcepack <pack-root> <name> <filename> <url> <sha256>
bkmpw add-shaderpack <pack-root> <name> <filename> <url> <sha256>
```

这两个命令会写 `resourcepacks/*.pw.toml` / `shaderpacks/*.pw.toml`，metadata 里的
`side` 默认是 `client`，裸 `filename` 会下载到同目录。

GitHub Release 添加：

```text
bkmpw add-github <pack-root> <side> <owner/repo|url> [--tag tag] [--asset text] [--name n] [--filename f] [--cf-project-id id --cf-file-id id]
```

默认读取 latest release；如果 release 有多个 asset，使用 `--asset` 指定文件名片段。
`--cf-project-id` 和 `--cf-file-id` 必须成对提供，用于写入导出专用映射：

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

这个格式兼容旧 metadata：没有 `[export.curseforge]` 时，行为不变。存在时，
`download-files` / `sync` 仍使用 `[download]`，只有 `export-curseforge` 会把该文件
写入 `manifest.json`，并避免再把对应 jar 放进 `overrides/`。

如果 coremod 已经由 GitHub Actions 自动上传到 CurseForge，可以让导出时按当前
整合包的 Minecraft 版本和 loader 自动选择最新 CF 文件：

```toml
[download]
mode = "metadata:curseforge"

[update.curseforge]
project-id = 123456
file-id = 789012

[export.curseforge]
latest = true
```

`latest = true` 会复用 `[update.curseforge].project-id`；也可以在
`[export.curseforge]` 里显式写 `project-id = 123456`。导出时需要
`CURSEFORGE_API_KEY` 或 `[curseforge].api-key`，因为它要查询最新适配文件。

更新：

```text
bkmpw update <pack-root> --all
bkmpw update <pack-root> <name>
```

- CurseForge metadata 会根据 `[update.curseforge]` 查询最新匹配文件并更新 `filename`、hash 和 `file-id`；需要 `CURSEFORGE_API_KEY` 或 `[curseforge].api-key`。
- GitHub metadata 会根据 `[update.github]` 查询 latest release 或指定 tag。只有通过新版 `add-github` 添加的 metadata 才会自动带这个更新信息。
- 普通直链 metadata 没有可推断的上游版本，`--all` 会跳过。

CurseForge API key 也可以通过环境变量 `CURSEFORGE_API_KEY` 提供。安装时会
优先使用本地 jar，所以完整包不需要在安装阶段提供 key。

`cdn-fallback = true` 时，CurseForge file ID 会先映射为
`https://edge.forgecdn.net/files/...` 形式的 ForgeCDN 地址。如果失败且存在
API key，再尝试官方 API。

`add-curseforge` 常用方式：

```text
# 有 CURSEFORGE_API_KEY 时，按 slug/URL/project-id 自动查项目和最新文件
bkmpw add-curseforge . both always-eat
bkmpw add-curseforge . both https://www.curseforge.com/minecraft/mc-mods/always-eat
bkmpw add-curseforge . both 1259229

# 指定 file-id
bkmpw add-curseforge . both 1259229 6498183

# 没有 API key 时，显式提供必要元数据
bkmpw add-curseforge . both 1259229 6498183 --name "Always Eat" --filename "AlwaysEat-neoforge-1.0.0.jar" --hash "5948137fed00d7bf3fe688e1aecc310b4fa5aaf7"
```

slug/URL 自动解析需要 `CURSEFORGE_API_KEY` 或 `[curseforge].api-key`。没有 key
时无法从 CurseForge 搜索项目，但仍可用 `project-id + file-id + filename + hash`
直接写出可下载、可导出的 metadata。

`export-curseforge <pack-root> [output.zip] [side]` 会先 refresh，然后生成
CurseForge 格式的 zip：`metadata:curseforge` 和带 `[export.curseforge]` 的文件写入
`manifest.json`，配置和其它发布文件写入 `overrides/`。如果
`[export.curseforge] latest = true`，会按 `pack.toml` 里的 Minecraft 版本和 loader
查最新适配文件。zip 使用 store 模式，不压缩，换取零额外依赖和小体积。

## 体积策略

- release 构建按体积优化。
- HTTP/HTTPS 下载使用 `ureq` + `rustls`。
- CurseForge API 响应处理不引入 JSON 依赖，只提取安装所需的 `downloadUrl`。

## License

MIT。
