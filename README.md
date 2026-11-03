# bro-know-my-packwiz

**用元数据维护整合包，用同一套流程同步实例、发布客户端和服务端。**

[![Release](https://img.shields.io/github/v/release/bro-know-my-org/BroKnowMyPackwiz)](https://github.com/bro-know-my-org/BroKnowMyPackwiz/releases)
[![npm version](https://img.shields.io/npm/v/%40bro-know-my%2Fpackwiz?logo=npm)](https://www.npmjs.com/package/@bro-know-my/packwiz)
[![crates.io](https://img.shields.io/crates/v/bkmpw?logo=rust)](https://crates.io/crates/bkmpw)
[![Rust](https://img.shields.io/badge/built_with-Rust-dea584?logo=rust)](Cargo.toml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[English](README.en.md) · [下载安装](#安装) · [快速开始](#快速开始) · [发布整合包](#发布整合包) · [文档导航](#文档导航)

`bro-know-my-packwiz` 是面向 Minecraft 整合包维护与分发的 Rust 工具，命令名为
`bkmpw`。支持 packwiz-style `.pw` / `.pw.toml` 元数据，提供可脚本化 CLI 和交互式 TUI。

| 你要做什么 | bkmpw 提供什么 |
| --- | --- |
| 收录内容 | 从 CurseForge、GitHub Release、直链或本地文件添加模组，也支持资源包和光影包 |
| 维护版本 | 检查更新、更新元数据、锁定版本，生成 Markdown / CSV 模组清单 |
| 同步实例 | 按客户端或服务端安装，复用本地文件，校验哈希，并行下载和重试 |
| 发布整合包 | 导出 CurseForge 包、客户端全量包、服务端全量包或下载型安装包 |
| 自动化 | JSON / JSON Lines 输出，供 CI、脚本和其他进程集成 |

> [!NOTE]
> 项目持续维护中。提交间隔较长通常意味着暂时没有待修复问题或新功能计划。
> 欢迎通过 [Issue](https://github.com/bro-know-my-org/BroKnowMyPackwiz/issues) 提出需求。

## 安装

**下载二进制：** 前往 [GitHub Releases](https://github.com/bro-know-my-org/BroKnowMyPackwiz/releases)，
选择对应系统和架构的文件，重命名为 `bkmpw`（Windows 为 `bkmpw.exe`），放入 `PATH`。
Linux / macOS 还需执行 `chmod +x bkmpw`。发布附件提供 `.sha256` 校验文件。

**通过 npm 安装：** 需要 Node.js 18 或更高版本，由 npm 选择平台包。

```sh
npm install -g @bro-know-my/packwiz
bkmpw --help
```

**通过 Cargo 安装：** 需要 Rust 工具链和当前平台的链接器。

```sh
cargo install bkmpw --locked
bkmpw --help
```

> [!IMPORTANT]
> Cargo 安装会在本机从源码编译，默认不包含 CurseForge API key。
> 使用 CurseForge 查询和自动添加功能前，请设置环境变量 `CURSEFORGE_API_KEY`，
> 或在整合包的 `.pw/config.toml` 中配置 `[curseforge].api-key`。

目标平台包括 Windows、macOS 和 Linux（glibc）的 x64 / ARM64；实际附件以所选版本为准。
v0.2.5 不包含 Linux ARM64 和 Windows ARM64 npm 平台包，详见 [npm 包说明](packages/npm/README.md)。

<details>
<summary>从源码构建</summary>

需要 Rust 工具链和当前平台的链接器。在仓库根目录运行：

```sh
cargo build --release --locked
./target/release/bkmpw --help
```

Windows PowerShell 使用 `.\target\release\bkmpw.exe --help`。
后文命令假定 `bkmpw` 已在 `PATH` 中，也可替换为构建产物的路径。
从源码构建同样需要自行配置 CurseForge API key，才能使用对应的 API 查询功能。

</details>

## 快速开始

以添加 CurseForge 模组并发布 CF 整合包为例，先初始化：

```sh
bkmpw init my-pack
```

编辑 `my-pack/pack.toml`，填写整合包名称、Minecraft 和加载器版本。
创建 `my-pack/.gitignore`，加入以下规则，避免本地 jar 被重复扫描为普通发布文件：

```gitignore
mods/*.jar
```

添加模组时，项目标识只需填 **slug、项目 URL、项目 ID 中的一种**。
下面三条命令任选一条，工具会查询适配整合包版本的最新文件：

```sh
bkmpw add-curseforge my-pack both always-eat
bkmpw add-curseforge my-pack both https://www.curseforge.com/minecraft/mc-mods/always-eat
bkmpw add-curseforge my-pack both 1259229
```

如需换成自己的 CurseForge API key，可设置 `CURSEFORGE_API_KEY` 或
`.pw/config.toml` 的 `[curseforge].api-key`，详见[配置](#配置)。

然后检查、下载并导出：

```sh
# 刷新索引并检查整合包
bkmpw refresh my-pack
bkmpw check my-pack

# 下载受管文件并生成 CurseForge 包
bkmpw download-files my-pack
bkmpw export-curseforge my-pack cf.zip both
```

有指定版本或特殊来源时，也可以手动指定 file ID，或使用 `add-github`、
`add-url`、`add-file` 的完整参数，见[命令](#命令)和[配置示例](#配置)。
要同步到服务端运行目录，可运行 `bkmpw sync my-pack server-instance server`。

喜欢交互操作，可以直接运行 `bkmpw tui my-pack`，在终端内添加、更新和导出。

> [!TIP]
> `check-updates` 只查看更新；`update` 修改元数据和索引。
> 要下载新文件，再运行 `download-files` 或 `sync`。

### 目录与 side

```text
my-pack/
├── pack.toml              # 整合包信息、Minecraft 和加载器版本
├── index.toml             # refresh 生成的索引
├── .gitignore             # 排除本地 jar 等不应重复扫描的文件
├── .pw/config.toml        # 扫描、安装和导出配置
├── mods/
│   ├── example.pw.toml    # 新增模组元数据默认写在这里
│   ├── example.jar        # 模组实际文件
│   ├── common/            # 可手动整理：两端共用的元数据
│   ├── client/            # 可手动整理：仅客户端的元数据
│   └── server/            # 可手动整理：仅服务端的元数据
├── resourcepacks/         # 资源包元数据与文件
├── shaderpacks/           # 光影包元数据与文件
└── roots/                 # 可选：common / client / server 根目录覆盖文件
```

`side` 可取 `both`、`client`、`server`。新增模组不会自动分桶；手动移动到
`mods/common`、`mods/client` 或 `mods/server` 后，安装和导出以目录位置为准。
`refresh` 不会据此改写元数据中的 `side` 字段。旧式 `.pw` 文件仍可使用。
资源包和光影包的裸文件名安装到元数据所在目录。

> [!IMPORTANT]
> `download-files` 只补齐文件，不清理无关内容。`sync`、`install-files-headless`
> 和 `install-files-retry` 只清理上次 `packwiz.json` 记录中已不再需要的
> `mods/`、`resourcepacks/`、`shaderpacks/` 文件；未受管的手动 jar 不会被删除。

## 发布整合包

| 交付方式 | 命令 | 产物 |
| --- | --- | --- |
| CurseForge 分发 | `bkmpw export-curseforge my-pack cf.zip both` | `manifest.json` 与 `overrides/` |
| 客户端全量包 | `bkmpw export-client my-pack client.zip` | 以实例目录包裹的客户端文件 |
| 服务端全量包 | `bkmpw export-server my-pack server.zip` | 文件直接放在 ZIP 根目录 |
| 下载型服务端包 | `bkmpw export-server-installer my-pack installer.zip` | 元数据与安装脚本，模组在目标机器下载 |

CurseForge 导出按元数据分发：`metadata:curseforge` 或带 `[export.curseforge]`
映射的条目写入 `manifest.json`，由平台下载对应文件；没有 CF 映射的本地文件
（包括 `add-file` 添加的 jar）会打进 `overrides/`，随包分发。导出依据已有元数据，
不会自动查询本地 jar 是否已发布到 CF；本地文件也可手动补充 CF 映射，见[元数据格式](docs/metadata-format.md#curseforge-export-mapping)。

全量导出前先用 `download-files` 补齐本地受管文件。下载型服务端包不夹带模组 jar
或本机二进制，安装脚本会下载并校验 `bkmpw`，因此模组需具有可用的远程下载源。
发布模板、根目录覆盖和隔离准备见
[发布流程](docs/workflows.md#release-preparation-all-four-exports)。

## 文档导航

- 日常维护：[工作流](docs/workflows.md) · [元数据格式](docs/metadata-format.md)
- 进阶操作：`bkmpw neoforge tui` 可原地升级标准 NeoForge 服务端与 Prism 实例
- 集成开发：[机器协议](docs/machine-protocol.md) · [架构](docs/architecture.md) · [测试](docs/testing.md)
- 本页参考：[交互式终端界面](#交互式终端界面) · [命令](#命令) · [配置](#配置) · [机器接口](#机器接口)

## 交互式终端界面

<details>
<summary>展开页面说明、快捷键与任务恢复规则</summary>

在交互式终端运行 `bkmpw tui [pack-root]`，省略目录时打开当前目录。
从源码构建后可运行 `./target/release/bkmpw tui /path/to/pack`；Windows 使用
`.\target\release\bkmpw.exe tui C:\path\to\pack`。新目录会进入初始化入口。

TUI 提供文件、添加、整合包、任务、设置五个页面，支持中文/英文、键盘与鼠标。
按 `Tab` / `Shift+Tab` 切换页面，`L` 切换语言，`?` 查看可滚动帮助，`Q` 退出。
输入框内的字母会作为输入，不触发全局快捷键。

- 文件页：`/` 搜索，空格标记，`Shift+U` 检查全部更新，`U` 预览所选更新，`D` 直接更新，`E` 编辑常用字段，
  `Shift+E` 调用外部编辑器，`Delete` 确认批量移除元数据。
  更新列表用 `↑↓` 移动、空格勾选、`A` 全选/取消全选、回车更新所选项，`Esc` 取消。
  宽终端左右显示可更新项与无更新/跳过项；`H` 隐藏或恢复无更新/跳过列表。
  `←→` 横向查看长版本名，`PgUp/PgDn` 滚动跳过原因。
  `V` 切换光标所在的单个 MOD 版本：CurseForge 填文件 ID，GitHub 填具体 Release tag
  和附件筛选，支持降级，确认预览后执行。已锁定项需先按 `P` 解除锁定。
  切换不会修改后续更新规则；要留在该版本，可切换完成后按 `P` 锁定。
  默认只更新元数据，可勾选同时下载；额外依赖变更会再次显示确认。
- 添加页：CurseForge 搜索、文件和依赖选择；`G` 选择 GitHub Release 附件，
  `U` 添加直链，`A` 添加本地文件。可通过 `CURSEFORGE_API_KEY`
  或 `[curseforge].api-key` 换成自己的 key。默认只保存元数据，可勾选同时下载。
- 整合包页：方向键或滚轮选择检查、安装、同步、模板准备及导出操作，回车打开表单。
- 任务页：空格暂停/继续，`C` 取消任务，`R` 重试恢复，`K` 保留外部内容，
  `O` 恢复原内容；处理冲突前先查看目标路径和保留的备份。

修改操作按队列顺序执行，每个任务独立回滚。失败或取消会暂停后续任务；重启后
需要手动继续等待队列，中断任务先恢复再标为待检查。正常取消须等待停止和回滚完成。
检测到外部修改时保留内容并等待解决；恢复依赖文件系统可用且日志、备份完整。
高级编辑器编辑暂存副本，确认后才保存。工具自身升级请退出 TUI 后使用 `bkmpw self-update`。

Linux release 真实终端、离线任务、外部编辑器，以及真实 GitHub/CurseForge 添加与下载已验证。
Windows/Linux/macOS 的远程 CI 测试、release 构建与 CLI 烟测已通过；
Windows/macOS 的真实终端交互验收仍按用户要求挂起。

</details>

## 命令

运行 `bkmpw --help` 查看当前版本的命令与用法。

<details>
<summary>展开常用命令语法</summary>

```text
bkmpw init [pack-root]
bkmpw tui [pack-root]
bkmpw inspect [pack-root]
bkmpw scan [pack-root]
bkmpw refresh [pack-root]
bkmpw list [pack-root]
bkmpw check [pack-root]
bkmpw check-updates <pack-root> [<name>...] [--mc-version v] [--loader neoforge]
bkmpw modlist <pack-root> [output-dir]
bkmpw export-client <pack-root> [output.zip] [root-dir]
bkmpw export-curseforge <pack-root> [output.zip] [side]
bkmpw export-server <pack-root> [output.zip]
bkmpw export-server-installer <pack-root> [output.zip]
bkmpw prepare-server <pack-root> [output-dir]
bkmpw prepare-pack <pack-root>
bkmpw add-url <pack-root> <side> <name> <filename> <url> <sha256>
bkmpw add-resourcepack <pack-root> <name> <filename> <url> <sha256>
bkmpw add-shaderpack <pack-root> <name> <filename> <url> <sha256>
bkmpw add-curseforge <pack-root> <side> <project|url|project-id> [file-id] [options]
bkmpw add-github <pack-root> <side> <owner/repo|url> [options]
bkmpw add-file <pack-root> <side> <name> <source-file> [filename]
bkmpw pin <pack-root> <name>
bkmpw unpin <pack-root> <name>
bkmpw remove <pack-root> <name>
bkmpw update <pack-root> (--all|<name>...) [--mc-version v] [--loader neoforge]
bkmpw download-files <pack-root> [jobs] [--force] [--retries n] [--retry-delay-seconds n]
bkmpw sync <source-root> <target-root> [side] [jobs] [--force] [--retries n] [--retry-delay-seconds n]
bkmpw install-files-headless <pack-root> [attempts] [delay-seconds]
bkmpw install-files-retry <pack-root> [attempts] [delay-seconds]
bkmpw install-local <source-root> <target-root> <side> [jobs] [--force] [--retries n] [--retry-delay-seconds n]
bkmpw hash <sha1|sha256|sha512|murmur2> <file>
```

</details>

四种 `export-*` 支持通过 `[release]` 配置启用发布准备：在临时目录展开模板、
校验元数据并刷新索引，不覆盖开发实例。`prepare-pack` 用于显式展开本地模板。
配置示例和行为说明见 [发布流程](docs/workflows.md#release-preparation-all-four-exports)。

## 配置

扫描先应用 `.gitignore`，再应用配置指定的 `.packwizignore`。

<details>
<summary>展开默认配置、下载选项、来源示例与导出细节</summary>

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
root-overlays = "roots"
metadata-extension = "pw.toml"

[install]
jobs = 32
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
`[export.curseforge]` 里显式写 `project-id = 123456`。导出时会查询最新适配文件。

更新：

```text
bkmpw check-updates <pack-root>
bkmpw check-updates <pack-root> --json
bkmpw update <pack-root> --all
bkmpw update <pack-root> <name> [<name>...]
```

`check-updates` 只查询并列出可更新项（名称、元数据路径、旧版本 → 新版本），不修改整合包。
可指定多个名称或元数据路径缩小范围，也支持 `--mc-version` / `--loader`。
CLI 与 TUI 的更新预览共用并发查询，默认同时检查 32 项；可用 `.pw/config.toml` 的 `[install].jobs` 调整（预览上限 64，最少 1；已有配置保留原值）。TUI 会显示查询完成数量。
`update` 会修改元数据和索引；下载新文件需另运行 `download-files` 或 `sync`。

- CurseForge metadata 会根据 `[update.curseforge]` 查询最新匹配文件并更新 `filename`、hash 和 `file-id`。
- GitHub metadata 会根据 `[update.github]` 查询 latest release 或指定 tag。只有通过新版 `add-github` 添加的 metadata 才会自动带这个更新信息。
- 普通直链 metadata 没有可推断的上游版本，`--all` 会跳过。

如需换成自己的 CurseForge API key，可设置 `[curseforge].api-key` 或环境变量
`CURSEFORGE_API_KEY`；配置文件中的非空值优先。安装时优先复用本地 jar。

下载先通过官方 API 获取地址；`cdn-fallback = true` 时，API 查询或
下载失败后，再将 CurseForge file ID 映射为
`https://edge.forgecdn.net/files/...` 形式的 ForgeCDN 地址。
CLI 下载中两种方式都失败时会同时报告错误。

`add-curseforge` 常用方式：

```text
# 按 slug/URL/project-id 自动查项目和最新文件（三选一）
bkmpw add-curseforge . both always-eat
bkmpw add-curseforge . both https://www.curseforge.com/minecraft/mc-mods/always-eat
bkmpw add-curseforge . both 1259229

# 指定 file-id
bkmpw add-curseforge . both 1259229 6498183

# 手动指定固定版本的完整元数据
bkmpw add-curseforge . both 1259229 6498183 --name "Always Eat" --filename "AlwaysEat-neoforge-1.0.0.jar" --hash "5948137fed00d7bf3fe688e1aecc310b4fa5aaf7"
```

通常只需提供 slug、URL 或 project-id 中的一种。手动提供
`project-id + file-id + filename + hash` 时，会直接写出可下载、可导出的 metadata。

`export-curseforge <pack-root> [output.zip] [side]` 会先 refresh，然后生成
CurseForge 格式的 zip：`metadata:curseforge` 和带 `[export.curseforge]` 的文件写入
`manifest.json`，配置和其它发布文件写入 `overrides/`。如果
`[export.curseforge] latest = true`，会按 `pack.toml` 里的 Minecraft 版本和 loader
查最新适配文件。zip 使用 store 模式，不压缩。

在 `.pw/config.toml` 中设置 `[export] include-metadata = true`，四种导出都会保留
当前导出 side 适用的 `.pw` / `.pw.toml` 元数据（包括未默认启用的可选条目、
材质包、光影包及自定义 metadata root）；CurseForge 包将其放在 `overrides/` 下。
`prepare-server` 使用相同规则。默认关闭；服务端安装器原本就会保留已选条目的
元数据，不受此默认值影响。

根目录 overlay：

```text
roots/common/   # 客户端包和服务端包都会铺到目标根目录
roots/client/   # 只进入 export-client 的实例根目录
roots/server/   # 只进入 export-server 和 export-server-installer 的服务端根目录
```

例如 `roots/common/icon.png` 导出到客户端包时会变成 `<实例目录>/icon.png`，
导出到服务端包时会变成 `icon.png`。如果 `roots/common` 和目标 side 目录里有同名文件，
目标 side 目录优先。

`export-server <pack-root> [output.zip]` 会先 refresh，然后生成直接解压使用的服务端包。
它不写 `manifest.json`，也不套 `overrides/` 目录；服务端和 common metadata 对应的
实际 jar 会写到 `mods/`，其它被扫描包含且适用于服务端的配置/脚本文件写到 zip 根目录
对应路径。`roots/common` 和 `roots/server` 的内容会铺到 zip 根目录。
`mods/client`、`resourcepacks` 和 `shaderpacks` 不会进入服务端包。

`export-client <pack-root> [output.zip] [root-dir]` 会先 refresh，然后生成客户端全量包。
zip 内会套一层实例目录，默认使用 `pack.toml` 的 `name`，也可以用 `root-dir` 指定；
client 和 common metadata 对应的实际 jar 会写到该目录的 `mods/` 下，资源包和光影包会按
扫描结果保留。`roots/common` 和 `roots/client` 的内容会铺到该实例目录根部。

`export-server-installer <pack-root> [output.zip]` 会生成下载型服务端安装包。
它包含服务端/common metadata、服务端适用的配置/脚本文件、`roots/common` 和
`roots/server` 铺到根目录后的文件，以及 `install-server.bat`、`install-server.sh`；
不夹带实际 mod jar，也不夹带本机 `bkmpw` 二进制。用户解压后运行安装脚本，脚本会从
GitHub latest release 下载当前平台的 `bkmpw`、校验 `.sha256`，再执行
`bkmpw install-local . . server` 在目标机器下载服务端所需 jar。

`prepare-server <pack-root> [output-dir]` 使用同一套服务端包文件规则，但输出为目录而不是
zip。默认目录是 `.bkmpw/server-pack`。运行前会删除旧输出目录，避免 CI 补文件、压缩发包时
混入上次构建遗留内容。

</details>

## 机器接口

桌面壳、CI 和其它进程集成可以使用稳定的机器输出，不需要解析普通终端文本：

```text
bkmpw protocol-version --json
bkmpw inspect . --json
bkmpw check . --json
bkmpw update . --all --json-lines
```

`--json` 在 stdout 输出一个 JSON 对象；`--json-lines` 输出可实时读取的
`started`、`progress`、`completed` / `failed` 事件。诊断和底层下载进度仍写入
stderr。机器模式保留正常退出码，例如 `check` 校验失败时仍退出 2，但会返回完整的
结构化检查结果。

协议版本独立于 CLI 版本。接入方应先读取 `protocol-version`，并忽略协议对象中未知的
可选字段。完整字段、兼容规则和支持命令见
[`docs/machine-protocol.md`](docs/machine-protocol.md)。

## 开发

提交改动前运行：

```sh
cargo fmt --check
cargo test
cargo build --release
```

## 致谢与来源说明

本项目受到 [packwiz](https://github.com/packwiz/packwiz) 生态启发，沿用公开的
packwiz 元数据模型，并借鉴 [packwiz-installer](https://github.com/packwiz/packwiz-installer)
的部分安装工作流。项目是独立实现的 Rust CLI，没有直接复制或 vendoring 上游源码。

## License

[MIT](LICENSE)。
