# NeoForge 原地升级计划

状态：首期实现完成，Linux 验收完成；Windows/macOS 本机验收和 Prism 登录后的进游戏验证待完成。本计划按 2026-09-30 用户确认需求重写。

## 已确认需求与边界

- 首期仅 NeoForge，标准服务端和 Prism Launcher 原实例；Windows、Linux、macOS 均需支持。其他启动器 TODO。
- 精确保持 Minecraft 版本，跨游戏版本直接拒绝，无强制绕过。只升级加载器，不更新模组。
- 游戏/服务端必须已关闭；检测到运行进程时阻止，不能以文件未锁定作为停止证据。
- 首期同时交付 TUI 和可脚本化 CLI，共用核心：选择实例 → 实际版本检测 → 稳定候选 → 用户选择 → 预检 → 升级。
- 候选仅当前 Minecraft 的稳定版，不提供 beta；当前 beta 如实显示。不能根据 pack.toml 推断实际安装。
- Prism 保持原实例身份和设置，用户点击原实例即使用新 NeoForge；保持模组、配置、存档。
- 检查实际 mods 中所有 JAR（含手动模组），读取内部加载器和必需依赖约束；区分 FML loaderVersion 和 NeoForge 本体。
- 明确不兼容阻止；未知结果列出文件和原因、让用户选择继续/取消；非交互 CLI 必须显式处理确认，不能等待 stdin。
- 静态声明预检不保证整包运行兼容。
- 尽量保留 Java 路径、内存及 JVM 参数，无法可靠迁移时明确提示手动迁移，不能静默丢失。
- 变更前以带时间标记的 old_ 名称备份，默认保留；提供手动回滚和清理。
- 安装/切换失败自动恢复；升级后启动由用户执行，启动失败手动回滚。
- 使用持久化日志支持进程中断恢复，加载器受管文件/状态独立于 packwiz.json。
- 不扫描删除 libraries，不删除归属不明确的文件。
- 实例内有 pack.toml 才同步 NeoForge 字段；不创建。外部源仅显式指定时同步。同步失败属于同一恢复事务。

## 样本与分析目录

样本位于 `neoforge-installers/`，SHA-256 在 `SHA256SUMS`；官方安装描述已确认两者 Minecraft 均为 `1.21.1`。
第一组真实验收：`21.1.242` → `21.1.252`。

专用分析目录：`/tmp/bkmpw-neoforge-analysis`。样本不作为发布产物。
需要反编译时使用 `/home/halo/.local/share/vineflower/vineflower-1.12.0.jar`。

## 可审查实施阶段

整体需求超过单个复杂变更 500 行限制，拆为独立阶段提交；每阶段记录实际范围和验证，避免以一个大 diff 交付。
最小首阶段为样本证据、实际检测和稳定候选核心，依赖方尚未接入前不开放执行入口。

1. **分析和只读核心**：核对安装器、标准服务端产物、Prism 组件机制；实际检测/精确 Minecraft/候选描述验证。
2. **模组预检**：JAR 元数据、加载器类型、Maven 区间、侧别、必需依赖、未知结果；独立测试。
3. **持久化恢复**：受管文件计划、备份、日志、原子替换、故障恢复/回滚/清理；独立测试。
4. **安装适配**：隔离官方服务端安装、验证产物、保留/迁移脚本；Prism 原实例组件切换；配置同步。
5. **CLI/TUI**：共用核心；显式目标、未知结果确认、非交互拒绝等待、维护入口。
6. **验收与文档**：真实样本升级、失败和中断恢复、保留校验、release CLI；平台限制如实记录。

## 验收清单

- [x] 读取 AGENTS.md 和旧计划，按已确认需求更新。
- [x] 样本 SHA-256 和 install_profile.json 已核对。
- [x] 服务端安装器实际输出、路径迁移及脚本分析。
- [x] Prism 组件管理和原实例切换方式核对。
- [x] 服务端与 Prism 的 21.1.242 → 21.1.252 原地升级（专用测试实例，Prism 组件级验证）。
- [x] 参数、手动模组、配置、存档的保持。
- [x] 精确 Minecraft 版本阻止、稳定候选过滤；当前版本不应用 beta 过滤。
- [x] 兼容性阻止、未知确认、FML 与 NeoForge 区分。
- [x] 安装失败、切换失败、中断恢复、手动回滚。
- [x] 重复升级、备份保留、手动清理、自定义脚本和带空格路径。
- [ ] Windows/macOS 本机路径和启动行为（Linux 已运行；跨平台路径字符串单元测试通过）。
- [x] cargo fmt --check、cargo test、cargo build --release 和 release 冒烟。

## 实施证据与限制

随每阶段追加证据、实际 diff 大小及验证结果。尚未完成项目不能标成已实现。

### 阶段 1：证据和只读核心

- 官方 `--installServer .` 在 `/tmp/bkmpw-neoforge-analysis/server old` 成功；产物包括版本目录的 server/universal JAR、unix_args.txt、win_args.txt，根目录 run.sh/run.bat/user_jvm_args.txt。库路径相对实例目录，可隔离安装后搬迁。
- 官方样本 `version.json`：NeoForge 21.1.242 / FML 4.0.43，目标 21.1.252 / FML 4.0.44。
- Prism 官方 PackProfile.cpp 的 componentToJsonV1/componentFromJsonV1 和 setComponentVersion 使用 mmc-pack.json；gameRoot 支持 minecraft 和旧 .minecraft。官方 meta 的 net.neoforged 索引提供精确 net.minecraft requires.equals、release 类型和组件 SHA-256。
- 实际检测依据服务端启动脚本+参数+server JAR，或 Prism 组件；不读取 pack.toml 作为安装依据。Prism 自定义加载器/游戏组件补丁拒绝自动迁移。

### 阶段 2：静态兼容预检

- 检查实际 mods/*.jar，支持压缩 ZIP、多个 modId、Manifest Implementation-Version 占位符、FML 独立约束、必需及不兼容依赖和侧别。
- 数字 Maven 精确/开闭/并集区间可判定；非数字比较、嵌套 JAR 和自定义语言加载器明确列为未知。未知库存下不将可能嵌套的缺失依赖误判为确定缺失。
- 明确不兼容不能通过 --accept-unknown 绕过；默认拒绝未知。4 个核心测试通过；此阶段复杂代码 398 行。
- 本地 Prism 11.1.0 的现有 NeoForge 实例确认使用 minecraft/ 和 mmc-pack.json（cachedVersion、cachedRequires）。仅只读核对，未修改用户实例。

### 阶段 3：持久化文件事务

- 411 行事务核心；独立 .bkmpw-neoforge/lock、old_<Unix 纳秒时间>、journal.json 和 ready/committed/restoring/rolled-back 标记。
- 全部原文件与 SHA-256 先落盘，再允许切换；失败恢复、重复恢复、手动回滚和保留备份已通过测试。恢复不覆盖升级后用户改动；备份损坏阻止恢复并保留证据。
- 只按明确的写入计划恢复/删除本次新建文件；不会扫描删除 libraries；packwiz.json 未参与事务。
- 7 个 NeoForge 核心测试通过，含切换失败、持久化中断恢复、未受管文件保持和带空格路径。

### 阶段 4a：服务端与 Prism 适配

- 官方 SHA-256 校验包括用户提供的本地安装器；服务端在独立 stage 执行 --installServer，保留日志，验证两个平台参数文件、运行库引用和 server/universal JAR。
- 仅复制本次隔离安装生成的 libraries 文件；同内容复用，内容冲突且旧状态无归属证据时阻止。旧版本库默认保留。
- 标准及显式 --script 的直接启动脚本仅替换 NeoForge 版本路径，原 Java 路径、JVM/内存、程序参数和换行保留；封装脚本无法识别时提示手动迁移。
- Prism 核对官方组件 SHA-256、精确 Minecraft 和 FML；更新原实例 mmc-pack.json 版本和缓存约束，Prism 在下次用户启动时负责客户端依赖下载/安装。instance.cfg 不写入。
- 现有实例/显式源目录的 pack.toml 纳入事务，缺失跳过；Minecraft 声明冲突阻止。8 个核心测试通过，适配核心 354 行。

### 阶段 4b：共用升级执行核心

- 共用 prepare/execute；下载与预检后复核真实安装和实际 JAR 指纹，切换后核对目标安装。实例、源目录和加载器锁独立持有。
- 每次变更前自动恢复 ready 但未提交的事务；专门恢复入口不依赖安装布局仍然完整。
- 用户选择目标，未知策略显式；已同版本时返回 unchanged。安装器失败保留 stage/日志，尚未改原实例；事务失败自动恢复。

### 阶段 5：首期 CLI 与 TUI

- `bkmpw neoforge tui [实例目录]`：路径选择、当前安装显示、稳定候选选择、可滚动预检、取消/明确继续和升级；复用 prepare/execute。
- CLI 提供 inspect/candidates/plan/upgrade/recover/rollback/backups/clean；upgrade 总是要求 --yes，未知还要求 --accept-unknown；永不从 stdin 等待。支持现有 --json/--json-lines 输出协议。
- 两端共用核心，CLI 另支持本地安装器、Java、自定义直接脚本及显式外部源同步。9 个核心测试通过；界面与接入约 350 行。

### 阶段 6a：恢复边界与回归验收

- 增加 11 项生产 JAR/Prism/配置/事务验收，20 个 NeoForge 测试通过。覆盖压缩手动 JAR、FML 本体区别、依赖侧别、多 modId、Manifest 占位符、Fabric 阻止、嵌套未知、Prism 新旧游戏目录、缓存/补丁冲突、外部源配置恢复、损坏备份、临时文件碰撞和符号链接边界。
- 自动恢复也锁定日志中明确记录的外部 pack 路径；无法读取 cwd 的 Java 保守阻止，并检查游戏实际路径。
- Release 真实 Prism 样本切换已成功（复制原实例描述/设置到专用验收目录）；未触碰用户现有实例。服务端首次隔离安装遇到 cli-utils 官方依赖网络下载失败，CLI 退出 2，旧安装保持 21.1.242。继续验证重试。

### 阶段 6b：真实升级、故障和运行检查

- 首次安装网络失败后，增加以官方目标 profile 的 SHA-1 为依据复用当前依赖；Minecraft 原始服务端 JAR 复制到隔离目录后仍由官方安装器核对 Mojang 哈希。不复制、接管无归属库。
- 服务端真实升级成功：`server-upgrade-retry.json`。Java 21.0.12.1 使用保留的原 run.sh 启动，日志 `server-launch-new.log` 明确列出 Minecraft 1.21.1、NeoForge 21.1.252，并到达 EULA 检查（未接受 EULA，也未验证可游玩服务端）。
- 重复请求返回 unchanged；手动回滚后 `server-launch-rollback.log` 列出 NeoForge 21.1.242，并再次到达 EULA 检查。外部 pack.toml 随回滚恢复旧字段。
- Prism 原实例 mmc-pack.json 切换与回滚成功；instance.cfg 字节相同。手动 `.JAR`、配置和存档保持；packwiz.json 哨兵字节保持。Prism 11.1.0 在独立数据目录识别了同一实例 ID，但没有账户，因此未验证登录后实际游戏启动。
- Release 故障验收：显式接受未知不能绕过不兼容；未知默认退出 2、明确接受后成功；跨版本 1.21.1 → 1.21.4 使用真实 21.4.158 安装描述退出 2；无 --yes 立即退出 2。
- 安装器注入退出码 7 时原脚本不变、无切换备份、日志保留。单元验收覆盖切换/配置同步失败和杀死真实切换子进程后的日志恢复。
- 实际 Java 进程（仅 cwd 指向实例）和 Java FileChannel 的 session.lock 均被 release CLI 阻止。Unix 同时检查 POSIX record lock 与 flock；不能将 flock 的成功误认为 Java 未持锁。
- 根目录直接自定义 sh/bat/cmd/ps1 入口自动识别迁移；嵌套直接入口用 --script；封装脚本不能可靠推断时明确要求手动迁移。
- TUI 在 PTY 完成实例选择、当前检测、候选显示和取消，退出 0 并恢复原终端属性。
- 用 Vineflower 1.12.0 核对 FML 4.0.44 的 BuiltInLanguageLoader：javafml、lowcodefml 都读取 FML JAR 版本，支持当前预检版本来源。
- 最新完整验证：cargo fmt --check 通过；cargo test **357 passed / 1 ignored**（ignored 是被父测试显式运行并杀死的子进程辅助入口）；cargo build --release 通过，仅有两项既有 dead_code 警告。

以上运行产物均在 `/tmp/bkmpw-neoforge-analysis/`，关键汇总为 `release-smoke-summary.json`、`running-check-summary.json`、`installer-failure.json` 和 `tui-smoke.log`。测试使用专用目录；没有升级用户原 CDPR 实例。

### 阶段 6c：使用文档和完整 TUI 验收

- 新增 [使用文档](neoforge-upgrade.md)。TUI 的 Java、嵌套直接脚本、本地安装器和显式源参数复用 CLI 参数解析与核心。
- PTY 中实际完成选择实例 → 稳定目标 21.1.252 → 预检 → 确认 → Prism 原测试实例升级，并同步显式外部 pack.toml；随后回滚恢复源配置。instance.cfg 字节和终端属性保持，日志 `tui-upgrade-smoke.log` / `tui-upgrade-rollback.json`。
- Release inspect 在 beta 组件夹具中如实输出 `21.1.0-beta`，记录 `current-beta-display.json`；稳定候选过滤有独立核心测试。
- 最终代码再次通过 fmt、357 项测试（另 1 个子进程辅助入口 ignored）和 release 构建。
- 共拆为 10 个可审查提交。机械核验确认前 9 个阶段均小于 800 行，复杂代码小于 500 行；第 10 阶段是约 170 行的界面参数接入及文档。首实现提交的额外 199 行是 Cargo.lock 机械依赖更新。

### 交付限制

- Windows/macOS 已实现对应路径、脚本和进程 API 适配，但本次没有本机或交叉编译验证；目前运行证据仅 Linux x86_64。
- Prism 客户端库下载/安装由原 Prism 在下次启动时执行；组件切换已验收，账户登录及可游玩客户端未验收。
- 数字 Maven 区间可判定；非数字排序、自定义语言加载器、嵌套 JAR、旧 mods.toml 等保守归为未知，需要明确确认。静态预检不保证运行兼容。
- 当前支持实际使用 --fml.neoForgeVersion / --fml.mcVersion 的标准参数文件布局；自定义 Prism 组件补丁、无法推断的脚本布局、非 UTF-8 脚本和符号链接/Windows reparse 写入路径需要手动迁移。
- 未能读取 Java cwd 时保守阻止；Prism 组件写入要求整个 Prism 已退出。事务记录使用绝对路径，未完成恢复前不要移动实例或备份。
- 其他启动器 TODO；不实现模组更新、跨 Minecraft 升级或自动试启动。
