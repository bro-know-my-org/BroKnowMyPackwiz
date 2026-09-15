# TUI 第一版验收对照

对应 [实施计划](tui-plan.md)，本表记录功能交付与 Linux 验收证据；Windows/macOS 验收按用户要求挂起，不能视为全部平台通过。
入口存在、单测通过、真实终端通过是不同层级的证据。

## 命令入口

整合包页由 `app.rs` 的页面事件打开 `pack.rs::form`，提交后经
`pack.rs::submit`、`operation/command.rs::Request` 进入持久队列。
下表核对实际页面入口及参数映射；命令自身输出保留 CLI 原文。

| 计划命令 | TUI 入口 | 当前证据 |
| --- | --- | --- |
| inspect / list / scan / check | 整合包页同名菜单 | `src/tui/pack.rs::ACTIONS`；Linux release 队列烟测 |
| init / refresh | 整合包页；新目录默认选中 init | 新目录入口单测；Linux release 队列烟测 |
| add-curseforge | 添加页搜索、项目、文件、依赖确认 | `src/tui/catalog.rs`、`dialog.rs`；可控响应测试；四类加载器从 pack.toml 到 API/本地兼容判断已有测试；Linux release PTY 真实鉴权搜索、必需依赖、取消预览和两种添加模式通过 |
| add-github | 添加页 G，仓库 → Release → 附件 | `src/tui/github.rs`、`source.rs`；Linux release PTY 真实 Release/附件选择、仅元数据及同时下载均通过 |
| add-url / add-file | 添加页 U / A | `src/tui/source.rs`、`catalog/source.rs`；本地及 HTTP 测试 |
| add-resourcepack / add-shaderpack | 添加来源表单中的资源类型；CF F 切换分类 | `src/tui/source.rs`、`catalog.rs`；类型和兼容规则测试 |
| pin / unpin / remove / rm / update | 文件页 P / Delete / U / D | `src/tui/app.rs`、`jobs.rs`、`update.rs`；预览、确认、队列测试 |
| download-files / sync / install-local | 整合包页；输出、side、并发、重试、延迟、force 表单 | `src/tui/pack.rs`、`operation/command.rs`；Linux release 队列烟测 |
| install-files / install-files-headless / install-files-retry | 使用 sync 或 install-local 参数表单；不分别增加菜单 | CLI 最终调用 install_local，目标为当前根、Both、cleanup=true；TUI 可复现这些参数。CLI 变体默认 retries=5、delay=10，TUI 可显式设置；不把不同默认值当成相同预设 |
| prepare-pack / prepare-server | 整合包页同名菜单 | Linux release 队列烟测，含输出文件/目录类型转换 |
| modlist | 整合包页，指定输出目录 | Linux release 队列烟测；产物沿用 CLI（Markdown/CSV） |
| export-client / export-curseforge / export-server / export-server-installer | 整合包页各导出表单 | 输出、root-dir 或 side 参数映射；Linux release 队列烟测 |
| hash | 整合包页，输入文件和算法 | 参数顺序为算法、文件；Linux release 队列烟测 |
| help / version / self-update 说明 | 全局 ?、标题版本、帮助内升级说明 | 双语帮助和窄窗口滚动测试；升级在退出 TUI 后通过 CLI 执行 |

JSON/protocol-version 仍作为 CLI 集成接口。Modrinth 项目搜索、GitHub 仓库关键词搜索不在第一版范围。

## 关键事务证据

| 要求 | 最强现有证据 | 证据边界 |
| --- | --- | --- |
| 每任务整批回滚 | `src/operation/transaction_tests.rs`：创建/替换/删除、目录模式、文件/目录转换后的取消和失败 | 临时真实文件树；不声称外部进程看不到提交中间态 |
| 取消与提交完成点 | `transaction_fault_tests.rs::cancellation_before_final_check_rolls_back_but_after_commit_marker_keeps_success` | 确定性检查点；提交成功后不追溯撤销 |
| 日志和目标写入失败 | `each_commit_write_boundary_recovers_from_a_single_io_failure` | 每个已覆盖写入边界注入 StorageFull/PermissionDenied；非真实磁盘满载 |
| 多输出跨磁盘提交后失败 | `runner.rs::partial_commit_failure_restores_pack_and_external_outputs` | Linux 默认路径及 `BKMPW_TEST_OUTPUT_BASE=/dev/shm` 均通过；整合包与外部文件/目录均已发布后注入 StorageFull，两边恢复原内容，重开日志重复回滚；非真实磁盘满载 |
| 备份失败 | `backup_io_failure_never_changes_original_files` | 复制前/后注入失败；原文件保持原样 |
| 持续 I/O 故障后再恢复 | `persistent_io_failure_keeps_backups_for_later_recovery` | 故障撤销后从持久日志重开恢复 |
| 恢复中途再次崩溃 | `crash_after_restoring_a_file_can_resume_recovery_again` | 在恢复替换后模拟进程中断，再次打开日志完成恢复；非真实断电 |
| 外部修改不强制覆盖 | `transaction_tests.rs` 的文件与目录冲突、显式保留/恢复、外部内容归档测试 | 指纹检测依赖文件系统、日志及备份仍可用 |
| 串行队列和重启确认 | `src/operation/queue.rs` 测试；`tests/tui_smoke.py` 保存队列后重启，确认前不执行 | Linux release 真实进程验证；外平台待验 |
| 切换前处理当前队列 | `tui::jobs::tests::switching_pack_waits_for_queue_loading_and_retains_its_result` | 双语、手填/最近目录：加载中及 Waiting/NeedsRecovery/Conflict 阻止切换，加载结果仍归原目录；Linux release PTY 验证全部完成后手填切换及最近列表返回、历史不变 |
| 配置错误不泄露原始字段值到队列历史 | `queue.rs::malformed_config_values_do_not_leak_into_saved_task_errors`；配置错误序列化测试 | 虚构 token、错误落盘及重启读取；CLI 字符串诊断与私有备份保持原行为 |
| 不清理手动 JAR | Linux release 队列烟测在下载/同步/安装后核对未托管 JAR | 离线 fixture，非用户真实整合包 |
| 元数据与下载同任务 | `src/operation/edit.rs`、`transfer.rs`、`download.rs` 的文件树和本地 HTTP 测试 | 来源平台真实鉴权添加另列 |

空间预检查当前覆盖私有工作副本所在文件系统，不能预知提交时各目标磁盘的可用容量。发布通过目标目录中的临时文件替换，跨盘无需 rename 源文件；提交 I/O 失败进入回滚，持续故障时保留备份等待恢复。

## 界面、国际化与平台

- 文件搜索、多选、焦点、中文粘贴、窄窗口、鼠标入口与滚轮有 ratatui TestBackend 测试；帮助/错误关闭、退出选择、恢复重试的双语鼠标测试已通过，Linux PTY 已实测鼠标打开/关闭帮助和退出；Linux release PTY 有启动、语言切换、重启队列和终端恢复证据。
- 消息资源有唯一性/完整性测试；操作错误使用稳定消息键和上下文，CLI 字符串接口保留旧文案；第三方原始错误和 CLI 日志保留原文。GitHub 仓库/名称校验、更新附件选择与临时目录诊断已接入消息键，并核对 CLI 原字符串不变；最终核查覆盖 TUI、operation、catalog 的表单、来源、编辑器、恢复与诊断适配，未发现新的可达应用诊断缺口；未做纯 CLI 私有 helper 的全量调用拓扑证明。
- 配置/元数据表单及外部编辑器通过暂存与确认提交；TOML 无关字段、注释和外部冲突有测试。Linux release PTY 已用真实编辑器子进程验证保存、取消、失败、无效 TOML 和外部修改五种往返场景，见 `tests/tui_editor_smoke.py`；Windows/macOS 尚未实测。
- `.github/workflows/ci.yml` 定义 Windows/Linux/macOS 的格式、测试、release 构建及 CLI 烟测；尚无本次代码的远程执行结果。TUI PTY 自动烟测当前只在 Linux 运行。用户已于 2026-09-15 明确挂起 Windows/macOS 测试；保留实现与 CI 配置，状态仍为未验证。
- 当前本地证据不能替代三平台中文输入、键鼠、resize、外部编辑器、正常退出和错误退出的真实终端验收。

## 外部编辑器真实终端往返

`python3 tests/tui_editor_smoke.py` 使用隔离的中文整合包路径与脚本编辑器，
通过真实 release TUI 的 Shift+E 启动子进程。编辑器收到的是用户状态目录中的私有草稿；
子进程记录终端标志，确认交接时恢复普通输入，返回 TUI 后恢复原始输入模式，退出后与启动前一致。

2026-09-15 Linux 本地通过：

- 确认保存：任务成功，中文新名称生效，原注释和未知字段保留。
- 取消预览：原文件保持不变，不产生写任务。
- 编辑器以状态 7 退出、返回无效 TOML：双语资源中的英文错误正确呈现，原文件不变、无写任务，保留草稿供修复。
- 预览后外部修改原文件：确认提交后任务失败/冲突，外部内容保留。

脚本已加入 Linux CI 步骤，但尚无远程 CI 结果。这验证本程序的编辑器交接流程，
不代表所有具体编辑器、终端模拟器和 Windows/macOS 均已验收。

## GitHub 真实联网添加

`python3 tests/tui_github_smoke.py` 为手动运行的 Linux release PTY 烟测，
不加入离线 CI；要求 Python 3.11+ 和可访问 GitHub API/附件下载的网络。
它通过实际键盘输入完成仓库 → Release → 附件 → 添加表单 → 预览确认，
分别使用隔离临时整合包验证默认仅元数据和勾选同时下载；不执行下载文件。
确认前整合包及元数据不变；确认后核对唯一队列任务、完成状态、来源身份、
下载地址、元数据哈希，以及实体文件是否存在、大小与 SHA-256。两次正常退出均检查终端恢复。

2026-09-15 本地实际通过的附件：

- 仓库 `FabricMC/fabric`，Release `0.160.5+26.3`。
- 附件 ID `564132163`，`fabric-api-0.160.5+26.3.jar`，2,589,795 字节。
- SHA-256：`5096e9774d629850338fc1c73b5571b527171374b11ac231691fb93731ee0ae6`。

预期值取自真实 API，脚本会核对 TUI 最终选择及产物；发布期间附件发生变化会失败。
这项证据不覆盖 CurseForge 鉴权、Windows/macOS 终端或所有 GitHub 项目。

## CurseForge 真实鉴权添加

`python3 tests/tui_curseforge_smoke.py` 从环境变量 `CURSEFORGE_API_KEY` 读取密钥，
通过 Linux release PTY 搜索 AppleSkin、选择 Fabric 1.21.1 文件、预览并确认必需的 Fabric API 依赖。
测试只操作隔离临时整合包，不执行下载文件；不把密钥传入命令行参数。

2026-09-15 本地通过取消预览、仅元数据、同时下载三种模式。取消不写入整合包；
两种添加模式均为单一队列任务，核对项目/file ID、`metadata:curseforge` 模式、SHA-1，
下载模式还核对文件大小和实际摘要；全部正常恢复终端，队列历史和终端输出均不含该密钥。
实际文件如下：

| 项目 | 项目 ID / 文件 ID | 文件 | SHA-1 |
| --- | --- | --- | --- |
| AppleSkin | 248787 / 5864741 | appleskin-fabric-mc1.21-3.0.6.jar | 97452cfadfde1f8f8c67838643019eabafa58fbb |
| Fabric API（必需依赖） | 306612 / 8786256 | fabric-api-0.116.17+1.21.1.jar | 810b2b0195371a012906241d8b85a32a1d6de53c |

真实 API 的当前匹配文件作为预期，执行期间文件变化会令测试失败。
这项证据不替代其他项目的依赖冲突/循环测试，也不代表所有下载源或平台组合均已联网实测。

## 收尾核查与交付边界

- 诊断与消息键核查已完成上述范围；资源完整性、中文输入、焦点、窄窗口、长文本、键鼠入口有自动测试。表单预览收尾修复了滚轮误改焦点与越过末尾的问题，`form.rs::preview_scroll_keeps_tail_visible_and_mouse_preserves_focus` 覆盖双语、中文尾部、缩放和内容替换重置。任务日志键盘/滚轮入口分别在 `jobs.rs::key`、`Jobs::mouse`，未把其他列表测试冒称为日志滚动专用测试。
- 队列持久化与重启证据见事务表；API 配置通过私有草稿提交，队列保存草稿引用；非法配置和真实 CurseForge 烟测均检查队列历史无对应 token。私有配置草稿和恢复备份含恢复所需内容，不承诺任意用户输入与第三方日志的通用秘密清洗。
- `tests/tui_smoke.py` 的模板 fixture 和 17 类队列任务核对整合包/服务端模板、modlist 和四类导出产物；runner 跨盘部分提交失败测试补充多输出回滚证据。未声称覆盖任意任务排列或所有文件系统。
- GitHub 与 CurseForge 的代表性真实添加流程已通过独立联网烟测；真实响应会变化，不能代表所有项目和版本组合。
- 本地 Linux 构建、测试和真实 PTY 验收已完成。三平台 CI 配置保留，尚无远程运行结果；Windows/macOS 构建和真实终端验收按用户要求挂起，状态为未验证。
- 真实断电、真实磁盘满载未实测；相关异常保证由确定性故障注入支撑，恢复仍要求文件系统、日志和备份可用。

最新测试数量和构建记录见 [实施记录](tui-progress.md)。
