# TUI 第一版验收对照

对应 [实施计划](tui-plan.md)，本表记录当前证据，不代表七阶段已全部通过。
入口存在、单测通过、真实终端通过是不同层级的证据。

## 命令入口

整合包页由 `app.rs` 的页面事件打开 `pack.rs::form`，提交后经
`pack.rs::submit`、`operation/command.rs::Request` 进入持久队列。
下表核对实际页面入口及参数映射；命令自身输出保留 CLI 原文。

| 计划命令 | TUI 入口 | 当前证据 |
| --- | --- | --- |
| inspect / list / scan / check | 整合包页同名菜单 | `src/tui/pack.rs::ACTIONS`；Linux release 队列烟测 |
| init / refresh | 整合包页；新目录默认选中 init | 新目录入口单测；Linux release 队列烟测 |
| add-curseforge | 添加页搜索、项目、文件、依赖确认 | `src/tui/catalog.rs`、`dialog.rs`；可控响应测试；真实鉴权添加待验 |
| add-github | 添加页 G，仓库 → Release → 附件 | `src/tui/github.rs`、`source.rs`；Linux 公开仓库浏览烟测；真实添加待验 |
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
| 备份失败 | `backup_io_failure_never_changes_original_files` | 复制前/后注入失败；原文件保持原样 |
| 持续 I/O 故障后再恢复 | `persistent_io_failure_keeps_backups_for_later_recovery` | 故障撤销后从持久日志重开恢复 |
| 恢复中途再次崩溃 | `crash_after_restoring_a_file_can_resume_recovery_again` | 在恢复替换后模拟进程中断，再次打开日志完成恢复；非真实断电 |
| 外部修改不强制覆盖 | `transaction_tests.rs` 的文件与目录冲突、显式保留/恢复、外部内容归档测试 | 指纹检测依赖文件系统、日志及备份仍可用 |
| 串行队列和重启确认 | `src/operation/queue.rs` 测试；`tests/tui_smoke.py` 保存队列后重启，确认前不执行 | Linux release 真实进程验证；外平台待验 |
| 配置错误不泄露原始字段值到队列历史 | `queue.rs::malformed_config_values_do_not_leak_into_saved_task_errors`；配置错误序列化测试 | 虚构 token、错误落盘及重启读取；CLI 字符串诊断与私有备份保持原行为 |
| 不清理手动 JAR | Linux release 队列烟测在下载/同步/安装后核对未托管 JAR | 离线 fixture，非用户真实整合包 |
| 元数据与下载同任务 | `src/operation/edit.rs`、`transfer.rs`、`download.rs` 的文件树和本地 HTTP 测试 | 来源平台真实鉴权添加另列 |

## 界面、国际化与平台

- 文件搜索、多选、焦点、中文粘贴、窄窗口、鼠标入口与滚轮有 ratatui TestBackend 测试；帮助/错误关闭、退出选择、恢复重试的双语鼠标测试已通过，Linux PTY 已实测鼠标打开/关闭帮助和退出；Linux release PTY 有启动、语言切换、重启队列和终端恢复证据。
- 消息资源有唯一性/完整性测试；操作错误使用稳定消息键和上下文，CLI 字符串接口保留旧文案；第三方原始错误和 CLI 日志保留原文。剩余跨模块 helper 透传路径仍在最终核查。
- 配置/元数据表单及外部编辑器通过暂存与确认提交；TOML 无关字段、注释和外部冲突有测试。Windows/macOS 的外部编辑器与终端往返尚未实测。
- `.github/workflows/ci.yml` 定义 Windows/Linux/macOS 的格式、测试、release 构建及 CLI 烟测；尚无本次代码的远程执行结果。TUI PTY 自动烟测当前只在 Linux 运行。
- 当前本地证据不能替代三平台中文输入、键鼠、resize、外部编辑器、正常退出和错误退出的真实终端验收。

## 完成前仍需取得的证据

1. 剩余应用自身诊断与消息键调用的收尾核查。文件搜索/过滤/排序/详情、搜索焦点下页签切换、平台分页/右键文件选择、表单双向选项、设置页滚轮与可见行点击已有双语测试；错误弹窗已验证中文长文本滚动、缩放限位、替换重置及搜索焦点隔离；文件与 CurseForge 详情已验证独立滚动、底部内容可见和条目切换重置；任务日志与表单预览已有键鼠滚动入口，其余组合仍需最终核对。
2. 真实 CurseForge 鉴权搜索 → 文件/依赖选择 → 元数据添加及可选下载；真实平台添加结果验证。
3. 本次代码在 Windows、macOS、Linux 的 CI 结果，及 Windows/macOS 真实终端验收。
4. 将计划中其余队列持久化、目录切换、秘密不入任务参数、模板/多输出行为逐项归档到最终验收证据。已有测试不自动推导为所有组合已覆盖。

最新测试数量和构建记录见 [实施记录](tui-progress.md)。
