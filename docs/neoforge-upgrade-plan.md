# NeoForge 原地升级计划

状态：实施中；本计划按 2026-09-30 用户确认需求重写。此前“客户端延期”和“TUI 后续”的假设作废。

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
- [ ] 服务端与 Prism 的 21.1.242 → 21.1.252 原地升级。
- [ ] 参数、手动模组、配置、存档的保持。
- [ ] 精确 Minecraft 版本阻止、稳定候选过滤、当前 beta 显示。
- [ ] 兼容性阻止、未知确认、FML 与 NeoForge 区分。
- [ ] 安装失败、切换失败、中断恢复、手动回滚。
- [ ] 重复升级、备份保留、手动清理、自定义脚本和带空格路径。
- [ ] Windows/Linux/macOS 路径和启动行为；未运行的平台必须标明。
- [ ] cargo fmt --check、cargo test、cargo build --release 和 release 冒烟。

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
