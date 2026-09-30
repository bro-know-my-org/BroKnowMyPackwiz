# NeoForge 原地升级

支持标准 NeoForge 服务端及 Prism Launcher 原实例。精确保持 Minecraft
版本，只切换加载器，不更新模组。其他客户端启动器 TODO。

先停止游戏或服务端。Prism 实例还需要退出整个 Prism Launcher，避免它
重写组件配置。发现 Java 进程或正在持有的世界锁时阻止；无法读取 Java
进程工作目录时也会保守阻止，并列出 PID。

## TUI

```sh
bkmpw neoforge tui
bkmpw neoforge tui "/path/with spaces/server"
bkmpw neoforge tui "/path/to/server" --java "/path/to/java21/bin/java"
```

选择实例目录，查看实际版本，再用方向键选择稳定目标。Prism 选择包含
`mmc-pack.json` 的实例目录；不要选择其 `minecraft/` 子目录。服务端
选择启动脚本所在目录。当前 beta 会如实显示，目标列表不提供 beta。

预检报告用 PgUp/PgDn 滚动；默认选择取消。明确不兼容时只能取消。
未知结果必须阅读具体文件和原因，再选择接受。静态预检不保证整包运行兼容。

TUI 也支持 `--script`、`--sync-source`、`--installer`，含义同下方 CLI。
目标和未知结果确认仍由界面交互决定。Java 默认使用 PATH 中的 `java`；
`--java` 只选择服务端安装器使用的 Java，不改原启动脚本的 Java 设置。

## 可脚本化 CLI

```sh
bkmpw neoforge inspect "/path/to/instance" --json
bkmpw neoforge candidates "/path/to/instance" --json
bkmpw neoforge plan "/path/to/instance" --target 21.1.252 --json
bkmpw neoforge upgrade "/path/to/instance" --target 21.1.252 --yes --json
```

先用 `plan` 阅读当前安装、目标、FML 版本及 `precheck.blocked` /
`precheck.unknown`。服务端根据实际脚本、参数和服务端 JAR 检测；Prism
根据其自身组件管理记录检测。`pack.toml` 不能作为实际安装证据。
`plan` 会下载并验证安装描述到工作目录，不切换实例文件。

CLI 永不读取 stdin 等待确认。升级总是要求 `--yes`；遇到未知还必须
显式增加 `--accept-unknown`。这两个选项都不能绕过确定不兼容或跨游戏版本。
失败退出码为 2；`--json` / `--json-lines` 使用现有机器输出协议。

```sh
# 已核对官方 SHA-256 的样本仍会再次匹配官方校验值
bkmpw neoforge upgrade "/path/to/server" \
  --target 21.1.252 \
  --installer neoforge-installers/neoforge-21.1.252-installer.jar \
  --java "/path/to/java21/bin/java" \
  --script "scripts/custom launch.sh" \
  --sync-source "/path/to/explicit pack source" \
  --accept-unknown --yes
```

`--script` 可重复，路径相对实例根。根目录直接引用 NeoForge 参数文件的
sh/bat/cmd/ps1 脚本自动识别；嵌套直接入口需显式指定。脚本仅替换加载器
版本路径，保留 Java 路径、内存、JVM/程序参数、引号和换行。
不直接引用参数文件的封装入口无法可靠推断时，会要求手动迁移。

服务端在隔离目录运行官方安装器，验证产物后切换。共享库同内容复用；
无归属且内容不同的库冲突会阻止。旧版本库保留。安装状态及受管文件写在
`.bkmpw-neoforge/state.json`，不扩展 `packwiz.json` 的清理边界。

Prism 修改原实例的 `mmc-pack.json` 组件版本/缓存约束，保留
`instance.cfg`、实例身份及全局继承设置。下次用户点击原实例时由 Prism
下载/安装客户端依赖。自定义 NeoForge/Minecraft 组件补丁需要先手动迁移。

实例根或游戏目录已有 `pack.toml` 时同步 `[versions].neoforge`，缺失跳过。
外部整合包源只处理显式 `--sync-source` 的目录；不搜索、不创建 pack.toml。
同步失败属于同一恢复事务。相同目标的重复升级返回 `unchanged`，不额外
创建备份或同步配置。

## 备份、恢复和回滚

```sh
bkmpw neoforge backups "/path/to/instance" --json
bkmpw neoforge recover "/path/to/instance" --json
bkmpw neoforge rollback "/path/to/instance" --json
bkmpw neoforge clean "/path/to/instance" old_1790767021822747491
```

每次切换前保存 `.bkmpw-neoforge/old_<Unix 纳秒时间>/`，默认永久保留。
日志及隔离安装文件在独立 `work_*` 目录。`clean` 只删除明确指定的备份；
它不会清理实例的 libraries。未回滚的已提交备份按最旧优先清理，以保持
回滚历史连贯；已回滚的备份可以直接清理。

安装失败时原实例尚未切换；切换失败自动恢复。进程中断后的下一次升级会
根据持久化日志恢复，也可显式执行 `recover`。恢复不依赖安装检测仍能成功。
损坏备份、升级后用户编辑过的受影响文件会阻止覆盖，并给出具体路径。
完成恢复前不要移动实例、外部源或备份；事务记录包含绝对路径。

升级完成后自行启动原实例。启动失败可用 `rollback` 恢复最近一次尚未
回滚的已提交升级；它也恢复该次同步的 pack.toml，不修改模组、配置和存档。

## 已验证与限制

Linux x86_64、Java 21.0.12.1：真实服务端 `21.1.242 → 21.1.252` 成功，
原脚本启动新版并到达 EULA 检查；回滚后启动旧版并到达同一检查。
未接受 EULA，未验证可游玩的完整服务端。Prism 11.1.0 测试实例的组件
切换、设置字节保持和回滚已验证；专用目录没有登录账户，未验证进入游戏。

Windows/macOS 路径、参数及平台进程 API 已适配，但本次没有本机运行或
交叉编译验证；目前不能据此宣称三平台运行验收全部通过。

数字 Maven 精确/开闭/并集区间可以判定；非数字比较、自定义语言加载器、
嵌套 JAR、旧 mods.toml 等保守归为未知。FML loaderVersion 使用目标 FML
组件版本，NeoForge 依赖使用目标 NeoForge 版本，不能互相替代。

当前支持实际使用 `--fml.neoForgeVersion` / `--fml.mcVersion` 的标准参数
布局。非 UTF-8 脚本、符号链接/Windows reparse 写入路径及自定义组件布局
需手动迁移。实例启动和模组更新始终由用户执行。

实施阶段、验收清单和运行证据见 [升级计划](neoforge-upgrade-plan.md)。
