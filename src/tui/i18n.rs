use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    ZhCn,
    #[default]
    En,
}

impl Language {
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
        "Tab / Shift+Tab: change page\n↑↓ / mouse wheel: move\nSpace / right click: mark a file\nEnter: focus details\n/: search; Enter or Esc: stop editing\nR: reload  L: switch language\nQ: quit  ?: close help",
        "Tab / Shift+Tab：切换页面\n↑↓ / 滚轮：移动\n空格 / 右键：选中文件\nEnter：切换详情\n/：搜索；Enter 或 Esc：结束输入\nR：刷新  L：切换语言\nQ：退出  ?：关闭帮助",
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
