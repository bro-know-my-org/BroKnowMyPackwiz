use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    ZhCn,
    #[default]
    En,
}

impl Language {
    pub fn error(self, error: &crate::operation::Error) -> String {
        use crate::operation::ErrorCode;
        let category = match error.code {
            ErrorCode::Io => "error_io",
            ErrorCode::Invalid => "error_invalid",
            ErrorCode::Conflict => "error_conflict",
            ErrorCode::Cancelled => "cancelled",
            ErrorCode::Busy => "error_busy",
            ErrorCode::Interrupted => "error_interrupted",
            ErrorCode::Failed => "error",
        };
        if let Some(key) = &error.message {
            let message = self.text(key);
            let context = error.message_context.as_deref().unwrap_or(&error.detail);
            if context.is_empty() || (error.message_context.is_none() && context == key) {
                message.into()
            } else {
                format!("{message}: {context}")
            }
        } else {
            format!("{}: {}", self.text(category), error.detail)
        }
    }
    pub fn detect() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
            .iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.is_empty())
            .or_else(sys_locale::get_locale)
            .map(|value| Self::from_locale(&value))
            .unwrap_or_default()
    }

    pub fn from_locale(locale: &str) -> Self {
        if locale.to_ascii_lowercase().starts_with("zh") {
            Self::ZhCn
        } else {
            Self::En
        }
    }

    pub fn toggle(&mut self) {
        *self = match self {
            Self::ZhCn => Self::En,
            Self::En => Self::ZhCn,
        };
    }

    pub fn text(self, key: &str) -> &str {
        MESSAGES
            .iter()
            .find(|(id, _, _)| *id == key)
            .map(|(_, en, zh)| if self == Self::ZhCn { *zh } else { *en })
            .unwrap_or(key)
    }
}

const MESSAGES: &[(&str, &str, &str)] = &[
    (
        "queue_identity_mismatch",
        "Saved queue version or pack directory does not match",
        "保存的队列版本或整合包目录不匹配",
    ),
    ("invalid_task_id", "Invalid task ID", "任务标识无效"),
    ("duplicate_task_id", "Duplicate task ID", "任务标识重复"),
    (
        "recovery_before_changes",
        "Finish recovery before adding changes",
        "请先完成恢复，再提交修改",
    ),
    (
        "recovery_unresolved",
        "Recovery is unresolved",
        "恢复尚未完成",
    ),
    ("unknown_task", "Task not found", "找不到任务"),
    (
        "recovery_before_exit",
        "Wait for recovery before exiting",
        "请等待恢复完成后退出",
    ),
    (
        "task_not_cancellable",
        "This task cannot be cancelled in its current state",
        "当前状态的任务无法取消",
    ),
    (
        "no_running_task",
        "No task is running",
        "当前没有运行中的任务",
    ),
    (
        "task_running",
        "A task is still running",
        "仍有任务正在运行",
    ),
    (
        "no_recovery_conflict",
        "This task has no recovery conflict",
        "此任务没有待处理的恢复冲突",
    ),
    (
        "task_worker_disconnected",
        "Task worker disconnected",
        "任务执行线程已断开",
    ),
    (
        "no_task_conflict",
        "This task has no conflict to resolve",
        "此任务没有待解决的冲突",
    ),
    (
        "queue_loader_disconnected",
        "Queue loader disconnected",
        "队列加载线程已断开",
    ),
    ("queue_unavailable", "Queue is unavailable", "队列暂不可用"),
    (
        "directory_changed",
        "Directory or permissions changed outside this task",
        "目录或权限被本任务之外的操作修改",
    ),
    (
        "transaction_not_prepared",
        "Transaction is not ready to commit",
        "事务尚未准备好提交",
    ),
    (
        "dependency_side_unknown",
        "Dependency installation side is unknown",
        "依赖的安装环境未知",
    ),
    (
        "duplicate_selected_project",
        "The same project was selected twice",
        "重复选择了同一项目",
    ),
    (
        "duplicate_installed_project",
        "Installed metadata duplicates a project",
        "已安装元数据包含重复项目",
    ),
    (
        "incompatible_dependency",
        "Projects declare an incompatibility",
        "项目声明了不兼容关系",
    ),
    (
        "dependency_cycle",
        "Required dependencies form a cycle",
        "必需依赖存在循环",
    ),
    (
        "dependency_selection_conflict",
        "Selected dependency versions conflict",
        "选中的依赖版本冲突",
    ),
    (
        "dependency_graph_too_large",
        "Dependency graph exceeds the supported size",
        "依赖关系规模超出支持范围",
    ),
    (
        "dependency_version_mismatch",
        "Dependency does not match Minecraft or loader",
        "依赖不匹配 Minecraft 或加载器版本",
    ),
    (
        "dependency_side_mismatch",
        "Installed dependency does not cover the required side",
        "已安装依赖不覆盖所需环境",
    ),
    (
        "dependency_pinned",
        "A pinned dependency blocks this update",
        "锁定的依赖阻止了本次更新",
    ),
    (
        "dependency_identity_mismatch",
        "Dependency response belongs to another project",
        "依赖响应属于其他项目",
    ),
    (
        "file_collision",
        "Destination already belongs to another file",
        "目标路径已被其他文件占用",
    ),
    (
        "unsupported_loader",
        "Unsupported mod loader",
        "不支持此模组加载器",
    ),
    (
        "missing_project_data",
        "Platform response has no project data",
        "平台响应缺少项目数据",
    ),
    (
        "project_identity_mismatch",
        "Platform returned a different project",
        "平台返回了其他项目",
    ),
    (
        "missing_file_data",
        "Platform response has no file data",
        "平台响应缺少文件数据",
    ),
    (
        "file_identity_mismatch",
        "Platform returned a different file",
        "平台返回了其他文件",
    ),
    (
        "file_project_mismatch",
        "Platform file belongs to another project",
        "平台文件属于其他项目",
    ),
    (
        "missing_sha1",
        "Platform file has no SHA-1 checksum",
        "平台文件缺少 SHA-1 摘要",
    ),
    (
        "invalid_sha1",
        "Platform file has an invalid SHA-1 checksum",
        "平台文件的 SHA-1 摘要无效",
    ),
    (
        "missing_platform_field",
        "Platform response is missing a field",
        "平台响应缺少字段",
    ),
    (
        "no_compatible_file",
        "No compatible file is available",
        "没有可用的兼容文件",
    ),
    (
        "directory_conflict",
        "Directory contents, type, or permissions conflict with recovery. Keep the external state, restore recorded permissions, or resolve the contents/type manually before retrying.",
        "目录内容、类型或权限与恢复操作冲突。可保留外部状态、恢复记录的权限，或手动处理内容及类型冲突后重试。",
    ),
    ("fabric", "Fabric", "Fabric"),
    ("quilt", "Quilt", "Quilt"),
    (
        "template_files",
        "Template destinations (comma separated)",
        "模板目标文件（逗号分隔）",
    ),
    ("error_io", "File access failed", "文件读写失败"),
    ("error_invalid", "Invalid input or data", "输入或数据无效"),
    ("error_conflict", "Conflicting changes", "检测到改动冲突"),
    ("error_busy", "Resource is busy", "资源正被占用"),
    ("error_interrupted", "Operation interrupted", "操作已中断"),
    (
        "asset_identity_mismatch",
        "Attachment identity does not match",
        "附件身份不匹配",
    ),
    (
        "curseforge_api_key_required",
        "Configure a CurseForge API key in Settings",
        "请在设置中配置 CurseForge API key",
    ),
    (
        "duplicate_download_target",
        "Multiple downloads share a destination",
        "多个下载使用同一目标路径",
    ),
    (
        "duplicate_managed_target",
        "Multiple managed files share a destination",
        "多个托管文件使用同一目标路径",
    ),
    (
        "duplicate_update_target",
        "Multiple updates share a destination",
        "多个更新使用同一目标路径",
    ),
    (
        "github_asset_changed",
        "GitHub attachment changed; preview again",
        "GitHub 附件已变化，请重新预览",
    ),
    (
        "github_asset_size_mismatch",
        "GitHub attachment size does not match",
        "GitHub 附件大小不匹配",
    ),
    (
        "http_url_required",
        "Enter an HTTP or HTTPS URL",
        "请输入 HTTP 或 HTTPS 链接",
    ),
    (
        "identifier_overflow",
        "Identifier exceeds the supported range",
        "标识符超出支持范围",
    ),
    (
        "invalid_asset_digest",
        "Attachment checksum is invalid",
        "附件校验摘要无效",
    ),
    (
        "invalid_asset_url",
        "Attachment URL is invalid",
        "附件链接无效",
    ),
    (
        "local_source_changed",
        "Local source changed; preview again",
        "本地源文件已变化，请重新预览",
    ),
    (
        "missing_array",
        "Platform response is missing a list",
        "平台响应缺少列表数据",
    ),
    (
        "missing_asset_id",
        "Attachment identifier is missing",
        "附件缺少标识符",
    ),
    (
        "missing_asset_size",
        "Attachment size is missing",
        "附件缺少大小信息",
    ),
    (
        "missing_curseforge_project",
        "CurseForge project is missing",
        "缺少 CurseForge 项目",
    ),
    (
        "missing_download_target",
        "Download destination is missing",
        "缺少下载目标路径",
    ),
    (
        "missing_github_project",
        "GitHub repository is missing",
        "缺少 GitHub 仓库",
    ),
    (
        "missing_id",
        "Platform identifier is missing",
        "缺少平台标识符",
    ),
    (
        "missing_payload",
        "Prepared file is missing",
        "暂存文件缺失",
    ),
    (
        "missing_release_id",
        "Release identifier is missing",
        "Release 缺少标识符",
    ),
    (
        "missing_release_tag",
        "Release tag is missing",
        "Release 缺少标签",
    ),
    ("name_required", "Enter a name", "请输入名称"),
    (
        "page_overflow",
        "Page exceeds the supported range",
        "页码超出支持范围",
    ),
    (
        "sha256_required",
        "Enter a valid SHA-256 checksum",
        "请输入有效的 SHA-256 摘要",
    ),
    (
        "unknown_file_type",
        "Unsupported file type",
        "不支持此文件类型",
    ),
    (
        "unknown_side",
        "Unsupported installation side",
        "不支持此安装环境",
    ),
    (
        "unknown_update_candidate",
        "Update candidate is no longer available",
        "更新候选已不可用",
    ),
    (
        "unsupported_curseforge_class",
        "Unsupported CurseForge project type",
        "不支持此 CurseForge 项目类型",
    ),
    (
        "update_hash_missing",
        "Update is missing a checksum",
        "更新缺少校验摘要",
    ),
    (
        "update_project_mismatch",
        "Update belongs to a different project",
        "更新属于其他项目",
    ),
    (
        "update_target_missing",
        "Update target is missing",
        "更新目标缺失",
    ),
    ("download_failed", "Download failed", "下载失败"),
    (
        "download_hash_mismatch",
        "Downloaded checksum does not match",
        "下载文件校验摘要不匹配",
    ),
    (
        "download_size_mismatch",
        "Downloaded size does not match",
        "下载文件大小不匹配",
    ),
    (
        "download_url_missing",
        "Download URL is missing",
        "缺少下载链接",
    ),
    (
        "duplicate_batch_target",
        "Multiple changes share a destination",
        "多个改动使用同一目标路径",
    ),
    (
        "duplicate_manifest_path",
        "Managed file record contains duplicate paths",
        "托管文件记录包含重复路径",
    ),
    (
        "file_size_overflow",
        "File size exceeds the supported range",
        "文件大小超出支持范围",
    ),
    (
        "hash_input_required",
        "Select an existing file and hash algorithm",
        "请选择现有文件和哈希算法",
    ),
    (
        "invalid_manifest_files",
        "Managed file records are invalid",
        "托管文件记录无效",
    ),
    (
        "invalid_manifest_path",
        "Managed file path is invalid",
        "托管文件路径无效",
    ),
    (
        "jobs_must_be_positive",
        "Parallel jobs must be greater than zero",
        "并发数必须大于零",
    ),
    (
        "output_missing",
        "Command produced no output file",
        "命令未生成输出文件",
    ),
    ("output_required", "Enter an output path", "请输入输出路径"),
    (
        "remove_selection_required",
        "Select metadata to remove",
        "请选择要移除的元数据",
    ),
    (
        "stderr_capture_failed",
        "Could not capture command errors",
        "无法读取命令错误输出",
    ),
    (
        "stdout_capture_failed",
        "Could not capture command output",
        "无法读取命令输出",
    ),
    (
        "unsupported_hash_format",
        "Unsupported checksum format",
        "不支持此校验摘要格式",
    ),
    (
        "unsupported_manifest_format",
        "Unsupported managed file record format",
        "不支持此托管记录格式",
    ),
    ("github_no_next_page", "No next page", "没有下一页"),
    (
        "github_page_required",
        "Enter a valid page number",
        "请输入有效页码",
    ),
    (
        "preview_running",
        "A preview is already running",
        "已有预览正在进行",
    ),
    (
        "preview_worker_disconnected",
        "Preview worker stopped unexpectedly",
        "预览任务意外停止",
    ),
    (
        "source_path_required",
        "Select a local source file",
        "请选择本地源文件",
    ),
    (
        "unexpected_add_action",
        "Unexpected add operation",
        "添加操作状态异常",
    ),
    ("remove", "Remove metadata", "移除元数据"),
    (
        "remove_preview",
        "Remove the selected metadata and refresh the index in one task. Installed files remain until a later synchronization.",
        "在一个任务中移除选中的元数据并刷新索引。已安装文件保留到后续同步处理。",
    ),
    ("raw_logs", "Original command logs", "命令原始日志"),
    ("inspect", "Pack overview", "整合包概览"),
    ("list", "List metadata", "列出元数据"),
    ("scan", "Scan files", "扫描文件"),
    ("check", "Check pack", "检查整合包"),
    ("refresh", "Refresh index", "刷新索引"),
    ("download-files", "Download missing files", "下载缺失文件"),
    ("sync", "Synchronize installation", "同步安装文件"),
    ("install-local", "Install to directory", "安装到指定目录"),
    ("prepare-pack", "Prepare pack templates", "准备整合包模板"),
    (
        "prepare-server",
        "Prepare server directory",
        "准备服务端目录",
    ),
    ("modlist", "Generate mod list", "生成模组列表"),
    ("export-client", "Export client archive", "导出客户端压缩包"),
    ("export-server", "Export server archive", "导出服务端压缩包"),
    (
        "export-server-installer",
        "Export server installer",
        "导出服务端安装包",
    ),
    (
        "export-curseforge",
        "Export CurseForge pack",
        "导出 CurseForge 整合包",
    ),
    ("init", "Initialize pack", "初始化整合包"),
    ("command_open", "Configure and enqueue", "配置并加入队列"),
    (
        "command_output",
        "Output path (relative to pack or absolute)",
        "输出路径（相对整合包或绝对路径）",
    ),
    ("archive_root", "Directory inside archive", "压缩包内根目录"),
    (
        "command_cleanup",
        "Clean previously managed files",
        "清理此前托管文件",
    ),
    (
        "command_readonly",
        "Read-only task; results appear in task logs.",
        "只读任务；结果显示在任务日志中。",
    ),
    (
        "command_transaction",
        "Runs in a private copy and publishes as one task. Cancel or failure rolls back this task. Blank numeric options use project defaults.",
        "在私有副本执行，作为一个任务提交。取消或失败会回滚本任务。数字选项留空使用项目默认值。",
    ),
    ("querying_updates", "Querying updates", "正在查询更新"),
    ("update_preview", "Preview updates", "预览更新"),
    ("update_direct", "Update directly", "直接更新"),
    ("update_files", "Update files", "更新文件"),
    ("select_all", "Select visible", "全选当前列表"),
    ("update_pinned", "Skipped: pinned", "已跳过：锁定更新"),
    (
        "update_no_provider",
        "Skipped: no update source",
        "已跳过：没有更新来源",
    ),
    ("update_unchanged", "Already up to date", "已是当前版本"),
    (
        "update_no_candidates",
        "No update candidates",
        "没有待更新项目",
    ),
    ("downloading", "Downloading", "正在下载"),
    ("retrying", "Waiting to retry", "等待重试"),
    ("transfer_bytes", "Transferred bytes", "已传输字节"),
    ("github_repository", "GitHub repository", "GitHub 仓库"),
    (
        "github_releases",
        "Release (←→ to choose)",
        "Release（←→ 选择）",
    ),
    ("github_assets", "Asset (←→ to choose)", "附件（←→ 选择）"),
    ("github_prerelease", "Prerelease", "预发布版本"),
    (
        "github_page",
        "Page (change, then save to load)",
        "页码（修改后保存翻页）",
    ),
    (
        "github_hash_pending",
        "Calculated by temporary download",
        "临时下载后计算",
    ),
    (
        "github_empty",
        "No entries on this page",
        "本页没有可选项目",
    ),
    ("source_add", "Add file", "添加文件"),
    ("source_url", "URL", "直链"),
    ("source_local", "Local file", "本地文件"),
    ("source_path", "Source file path", "源文件路径"),
    (
        "source_hash",
        "SHA-256 (empty: download temporarily to calculate)",
        "SHA-256（留空会临时下载计算）",
    ),
    (
        "source_download",
        "Also copy/download the file",
        "同时复制／下载实体文件",
    ),
    (
        "github_update_tag",
        "Update release tag (latest for new releases)",
        "更新标签（latest 跟随新版本）",
    ),
    (
        "github_update_filter",
        "Update asset name filter",
        "更新附件名称过滤",
    ),
    ("cf_plan", "Preview required dependencies", "预览必需依赖"),
    (
        "cf_confirm",
        "Confirm metadata and dependencies",
        "确认元数据与依赖变更",
    ),
    ("cf_update", "Update", "更新"),
    ("cf_reuse", "Reuse", "复用"),
    (
        "cf_add_task",
        "Add CurseForge files and dependencies",
        "添加 CurseForge 文件及依赖",
    ),
    (
        "cf_cancel_preview",
        "Preparing preview… cancel",
        "正在准备预览…取消",
    ),
    (
        "cf_preview_failed",
        "Could not prepare the preview; no pack changes were made.",
        "未能完成预览；整合包未发生变更。",
    ),
    (
        "cf_relaxed",
        "All versions (explicit override)",
        "全部版本（已放宽过滤）",
    ),
    ("cf_compatible", "Matching pack versions", "匹配整合包版本"),
    (
        "cf_search_hint",
        "Enter keywords with /, then press Enter to search. Esc returns to projects.",
        "按 / 输入关键词，Enter 搜索；Esc 返回项目列表。",
    ),
    (
        "cf_key_required",
        "Configure a CurseForge API key in Settings or CURSEFORGE_API_KEY.",
        "请在设置中填写 CurseForge API key，或设置 CURSEFORGE_API_KEY。",
    ),
    ("cf_choose", "Choose", "选择"),
    ("cf_type", "Type", "类型"),
    ("cf_filter", "Compatibility", "兼容过滤"),
    ("cf_prev", "Previous", "上一页"),
    ("cf_next", "Next", "下一页"),
    ("advanced", "External editor", "外部编辑器"),
    ("edit", "Edit", "编辑"),
    ("resume_pause", "Resume/pause", "继续/暂停"),
    ("keep_short", "Keep external", "保留外部修改"),
    ("restore_short", "Restore original", "恢复原文件"),
    (
        "keep_external",
        "Keep external changes and finish recovery",
        "保留外部修改并结束恢复",
    ),
    (
        "restore_original",
        "Archive external changes, then restore originals",
        "备份外部修改后恢复原文件",
    ),
    ("current_file", "Current file", "当前文件"),
    (
        "original_backup",
        "Original backup (if originally present)",
        "原文件备份（原文件存在时）",
    ),
    (
        "proposed_file",
        "Proposed file (if created by the task)",
        "任务新文件（任务生成时）",
    ),
    (
        "conflict_no_writes",
        "No published changes need restoring. Acknowledge and review the task before continuing.",
        "没有需要还原的已提交变更。确认后请检查任务，再继续队列。",
    ),
    (
        "review_changes",
        "Review changes (PgUp/PgDn to scroll)",
        "检查变更（PgUp/PgDn 滚动）",
    ),
    ("save", "Save / continue", "保存 / 继续"),
    ("cancel", "Cancel", "取消"),
    (
        "form_keys",
        "Tab: next  ←→: edit / choose  Enter: continue  Esc: cancel",
        "Tab: 下一项  ←→: 编辑 / 选择  Enter: 继续  Esc: 取消",
    ),
    ("pack_info", "Pack information", "整合包信息"),
    ("project_config", "Project configuration", "项目配置"),
    ("preferences", "Preferences", "个人偏好"),
    ("open_pack", "Open another pack", "打开其他整合包"),
    ("edit_metadata", "Edit metadata", "编辑元数据"),
    (
        "declared_side",
        "Declared side (folder takes priority)",
        "声明环境（目录位置优先）",
    ),
    ("optional", "Optional file", "可选文件"),
    ("default_enabled", "Enabled by default", "默认启用"),
    ("url", "Download URL", "下载地址"),
    ("download_mode", "Download mode", "下载模式"),
    ("hash_format", "Hash algorithm", "哈希算法"),
    ("hash", "Hash", "哈希值"),
    (
        "cf_export_project",
        "CurseForge export project ID",
        "CurseForge 导出项目 ID",
    ),
    (
        "cf_export_file",
        "CurseForge export file ID",
        "CurseForge 导出文件 ID",
    ),
    (
        "cf_export_latest",
        "Resolve latest file on export",
        "导出时解析最新文件",
    ),
    ("author", "Author", "作者"),
    ("version", "Version", "版本"),
    ("minecraft", "Minecraft version", "Minecraft 版本"),
    ("neoforge", "NeoForge version", "NeoForge 版本"),
    ("forge", "Forge version", "Forge 版本"),
    ("use_gitignore", "Use .gitignore", "使用 .gitignore"),
    ("packwizignore", "Pack ignore file", "整合包忽略规则文件"),
    ("jobs", "Parallel downloads", "并行下载数"),
    ("retries", "Retries", "重试次数"),
    ("retry_delay", "Retry delay (seconds)", "重试间隔（秒）"),
    (
        "force",
        "Replace existing managed files",
        "替换已有托管文件",
    ),
    ("api_key", "CurseForge API key", "CurseForge API 密钥"),
    ("cdn_fallback", "Allow CDN fallback", "允许回退到 CDN"),
    (
        "release_enabled",
        "Prepare templates for release",
        "发布时准备模板",
    ),
    ("template_dir", "Template directory", "模板目录"),
    ("language", "Language", "语言"),
    ("editor", "External editor command", "外部编辑器命令"),
    ("directory", "Directory", "目录"),
    ("recent", "Recently opened", "最近打开"),
    ("waiting", "Waiting", "等待中"),
    ("running", "Running", "执行中"),
    ("cancelling", "Stopping and rolling back", "正在停止并回滚"),
    ("completed", "Completed", "已完成"),
    ("failed", "Failed", "失败"),
    ("cancelled", "Cancelled", "已取消"),
    ("recovering", "Recovery required", "待恢复"),
    (
        "needs_check",
        "Recovered; review before continuing",
        "已恢复，请检查后继续",
    ),
    (
        "conflict",
        "Conflict; resolve before continuing",
        "存在冲突，解决后才能继续",
    ),
    ("paused", "Queue paused", "队列已暂停"),
    ("logs", "Logs", "日志"),
    ("preparing", "Preparing transaction", "准备事务"),
    ("snapshotting", "Preparing workspace", "准备工作目录"),
    ("committing", "Applying changes", "正在应用变更"),
    ("unpin", "Unpin", "解除更新锁定"),
    (
        "task_keys",
        "Space: resume/pause  C: cancel  R: retry recovery",
        "空格: 继续/暂停  C: 取消  R: 重试恢复",
    ),
    (
        "quit_task",
        "A task is running.\nW: wait for this task, then exit\nC: cancel and roll back, then exit\nEsc: return\nWaiting tasks will be saved.",
        "有任务正在执行。\nW：等待当前任务完成后退出\nC：取消并回滚后退出\nEsc：返回\n未执行的任务会保存。",
    ),
    ("files", "Files", "文件"),
    ("add", "Add", "添加"),
    ("pack", "Pack", "整合包"),
    ("tasks", "Tasks", "任务"),
    ("settings", "Settings", "设置"),
    ("details", "Details", "详情"),
    ("search", "Search", "搜索"),
    ("loading", "Loading…", "正在加载…"),
    ("empty", "No matching files", "没有匹配的文件"),
    (
        "no_pack",
        "Open or initialize a pack to get started",
        "打开或初始化整合包以开始",
    ),
    ("error", "Operation failed", "操作失败"),
    ("ready", "Ready", "就绪"),
    (
        "small",
        "Enlarge the terminal (at least 40 × 10)",
        "请扩大终端窗口（至少 40 × 10）",
    ),
    ("help", "Help", "帮助"),
    (
        "keys",
        "Tab: page  ↑↓: select  Space: mark  /: search  R: reload  L: language  Q: quit",
        "Tab: 页面  ↑↓: 选择  空格: 多选  /: 搜索  R: 刷新  L: 语言  Q: 退出",
    ),
    (
        "help_text",
        "↑↓ / wheel / PgUp / PgDn: scroll; Esc: close\n\nTab / Shift+Tab: change page; L: language; Q: quit\nFiles: / search, F type, S sort, Space/right click mark, A select visible\nEnter: details; E: edit; Shift+E: external editor; P: pin/unpin\nU: preview updates; D: update directly; Del: remove metadata\nAdd: / CurseForge search; F type; V relax compatibility; Enter select; ←→ page\nG: GitHub releases/assets; U: URL; A: local file; C: cancel preview\nPack: ↑↓ / wheel browse; Enter/click configure and enqueue\nForms: Tab / Shift+Tab move fields; ←→ choices; Enter next field or save\nPgUp / PgDn: scroll change preview; Esc: cancel\nTasks: Space pause/resume; C cancel; R retry recovery\nK: keep external changes; O: restore originals; PgUp/PgDn scroll logs\nQuit with a running task: W wait; C cancel and roll back; Esc return\nWaiting tasks are saved; reopening requires confirmation before resuming\nSettings: pack, loader, API, templates, language, editor, recent directories\n\nCLI details: bkmpw --help\nTo update bkmpw itself, exit the TUI and run bkmpw self-update",
        "↑↓ / 滚轮 / PgUp / PgDn：滚动；Esc：关闭\n\nTab / Shift+Tab：切换页面；L：语言；Q：退出\n文件：/ 搜索，F 类型，S 排序，空格/右键多选，A 全选当前列表\nEnter：详情；E：编辑；Shift+E：外部编辑器；P：锁定/解除锁定\nU：预览更新；D：直接更新；Del：移除元数据\n添加：/ 搜索 CurseForge；F 类型；V 放宽兼容过滤；Enter 选择；←→ 翻页\nG：GitHub Release/附件；U：直链；A：本地文件；C：取消预览\n整合包：↑↓ / 滚轮浏览；Enter/点击配置并加入队列\n表单：Tab / Shift+Tab 切换字段；←→ 切换选项；Enter 下一项或保存\nPgUp / PgDn：滚动变更预览；Esc：取消\n任务：空格暂停/继续；C 取消；R 重试恢复\nK：保留外部改动；O：恢复原内容；PgUp/PgDn 滚动日志\n有任务运行时退出：W 等待；C 取消并回滚；Esc 返回\n等待任务会保存，重新打开后需确认才继续\n设置：整合包、加载器、API、模板、语言、编辑器和最近目录\n\nCLI 详细帮助：bkmpw --help\n更新 bkmpw 本体：退出 TUI 后运行 bkmpw self-update",
    ),
    ("name", "Name", "名称"),
    ("source", "Source", "来源"),
    ("side", "Effective side", "有效环境"),
    ("path", "Metadata path", "元数据路径"),
    ("filename", "Filename", "文件名"),
    ("installed", "Present", "已存在"),
    ("missing", "Missing", "缺失"),
    ("pin", "Pinned", "锁定更新"),
    ("preserve", "Preserve", "保留文件"),
    ("yes", "Yes", "是"),
    ("no", "No", "否"),
    ("all", "All", "全部"),
    ("filter_keys", "F: type  S: sort", "F: 类型  S: 排序"),
    (
        "coming",
        "This workflow is being connected to transactional operations.",
        "此流程正在接入事务操作。",
    ),
    (
        "terminal_required",
        "TUI requires an interactive terminal",
        "TUI 需要交互式终端",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_errors_preserve_cli_text_and_persist_localized_context() {
        use crate::operation::{Error, ErrorCode};
        let error = Error::named(ErrorCode::Invalid, "unknown_task", "unknown task");
        assert_eq!(error.to_string(), "Invalid: unknown task");
        assert_eq!(Language::ZhCn.error(&error), "找不到任务");
        let error = error.context("123-456");
        let saved = serde_json::to_string(&error).unwrap();
        let restored: Error = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.to_string(), "Invalid: unknown task");
        assert_eq!(Language::ZhCn.error(&restored), "找不到任务: 123-456");
        assert_eq!(Language::En.error(&restored), "Task not found: 123-456");
        let legacy: Error = serde_json::from_str(
            r#"{"code":"Invalid","detail":"123-456","message":"unknown_task"}"#,
        )
        .unwrap();
        assert_eq!(Language::ZhCn.error(&legacy), "找不到任务: 123-456");
    }

    #[test]
    fn stable_error_messages_translate_without_parsing_raw_diagnostics() {
        use crate::operation::{Error, ErrorCode};
        let error = Error::key(ErrorCode::Invalid, "output_required");
        assert_eq!(Language::ZhCn.error(&error), "请输入输出路径");
        assert_eq!(Language::En.error(&error), "Enter an output path");
        let error = error.context("/tmp/中文");
        assert_eq!(Language::ZhCn.error(&error), "请输入输出路径: /tmp/中文");
        let raw: Error =
            serde_json::from_str(r#"{"code":"Failed","detail":"output_required"}"#).unwrap();
        assert_eq!(Language::ZhCn.error(&raw), "操作失败: output_required");
    }

    #[test]
    fn resources_are_complete_and_unique() {
        let mut keys = std::collections::HashSet::new();
        for (key, en, zh) in MESSAGES {
            assert!(keys.insert(key), "duplicate key {key}");
            assert!(
                !en.is_empty() && !zh.is_empty(),
                "missing translation {key}"
            );
        }
        assert_eq!(Language::from_locale("zh_CN.UTF-8"), Language::ZhCn);
        assert_eq!(Language::from_locale("de_DE"), Language::En);
    }
}
