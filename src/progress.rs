use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::http::DownloadProgress;

const REFRESH_INTERVAL: Duration = Duration::from_millis(120);
const BAR_WIDTH: usize = 22;

pub struct ProgressRenderer {
    inner: Arc<ProgressInner>,
    handle: Option<thread::JoinHandle<()>>,
}

struct ProgressInner {
    slots: Mutex<Vec<ProgressSlot>>,
    output: Mutex<()>,
    stop: AtomicBool,
    rendered_lines: AtomicUsize,
}

#[derive(Clone)]
struct ProgressSlot {
    label: String,
    current: u64,
    total: Option<u64>,
    status: SlotStatus,
    started: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotStatus {
    Idle,
    Active,
    Done,
    Failed,
}

pub struct SlotProgress {
    inner: Arc<ProgressInner>,
    index: usize,
}

impl ProgressRenderer {
    pub fn start(slots: usize) -> Option<Self> {
        if slots == 0 || !std::io::stderr().is_terminal() {
            return None;
        }
        let inner = Arc::new(ProgressInner {
            slots: Mutex::new(vec![ProgressSlot::idle(); slots]),
            output: Mutex::new(()),
            stop: AtomicBool::new(false),
            rendered_lines: AtomicUsize::new(0),
        });
        let thread_inner = Arc::clone(&inner);
        let handle = thread::spawn(move || {
            while !thread_inner.stop.load(Ordering::Acquire) {
                thread_inner.render();
                thread::sleep(REFRESH_INTERVAL);
            }
            thread_inner.render();
        });
        Some(Self {
            inner,
            handle: Some(handle),
        })
    }

    pub fn slot_progress(&self, index: usize, label: &str) -> Arc<dyn DownloadProgress> {
        {
            let mut slots = self
                .inner
                .slots
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            debug_assert!(index < slots.len(), "progress slot index out of bounds");
            if let Some(slot) = slots.get_mut(index) {
                *slot = ProgressSlot {
                    label: shorten_middle(label, 42),
                    current: 0,
                    total: None,
                    status: SlotStatus::Active,
                    started: Instant::now(),
                };
            }
        }
        Arc::new(SlotProgress {
            inner: Arc::clone(&self.inner),
            index,
        })
    }

    pub fn finish_slot(&self, index: usize, failed: bool) {
        let mut slots = self
            .inner
            .slots
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(slot) = slots.get_mut(index) {
            slot.status = if failed {
                SlotStatus::Failed
            } else {
                SlotStatus::Done
            };
        }
    }

    pub fn clear_slot(&self, index: usize) {
        let mut slots = self
            .inner
            .slots
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(slot) = slots.get_mut(index) {
            *slot = ProgressSlot::idle();
        }
    }

    pub fn log(&self, message: &str) {
        let _guard = self
            .inner
            .output
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let lines = self.inner.rendered_lines.swap(0, Ordering::Relaxed);
        clear_rendered_lines(&mut std::io::stderr(), lines);
        eprintln!("{message}");
    }
}

impl Drop for ProgressRenderer {
    fn drop(&mut self) {
        self.inner.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl ProgressInner {
    fn render(&self) {
        let slots = self
            .slots
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        let _guard = self.output.lock().unwrap_or_else(|err| err.into_inner());
        let mut stderr = std::io::stderr();
        let previous_lines = self.rendered_lines.load(Ordering::Relaxed);
        if previous_lines > 0 {
            let _ = write!(stderr, "\x1b[{previous_lines}A");
        }
        for slot in &slots {
            let _ = write!(stderr, "\r\x1b[2K{}\n", self.format_slot(slot));
        }
        let _ = stderr.flush();
        self.rendered_lines.store(slots.len(), Ordering::Relaxed);
    }

    fn format_slot(&self, slot: &ProgressSlot) -> String {
        match slot.status {
            SlotStatus::Idle => format!("[{}] waiting", " ".repeat(BAR_WIDTH)),
            SlotStatus::Active => {
                let elapsed = slot.started.elapsed().as_secs_f64().max(0.1);
                let rate = slot.current as f64 / elapsed;
                match slot.total {
                    Some(total) if total > 0 => {
                        let filled = ((slot.current.min(total) as f64 / total as f64)
                            * BAR_WIDTH as f64)
                            .round() as usize;
                        let bar = format!(
                            "{}{}",
                            "#".repeat(filled.min(BAR_WIDTH)),
                            " ".repeat(BAR_WIDTH.saturating_sub(filled))
                        );
                        let pct = slot.current.min(total) as f64 * 100.0 / total as f64;
                        format!(
                            "[{bar}] {:>5.1}% {}/{} {:>8}/s {}",
                            pct,
                            format_bytes(slot.current),
                            format_bytes(total),
                            format_bytes(rate as u64),
                            slot.label
                        )
                    }
                    _ => format!(
                        "[{}] {} {:>8}/s {}",
                        "#".repeat(BAR_WIDTH),
                        format_bytes(slot.current),
                        format_bytes(rate as u64),
                        slot.label
                    ),
                }
            }
            SlotStatus::Done => format!(
                "[{}] done   {} {}",
                "#".repeat(BAR_WIDTH),
                format_bytes(slot.current),
                slot.label
            ),
            SlotStatus::Failed => format!(
                "[{}] failed {} {}",
                "!".repeat(BAR_WIDTH),
                format_bytes(slot.current),
                slot.label
            ),
        }
    }
}

impl ProgressSlot {
    fn idle() -> Self {
        Self {
            label: String::new(),
            current: 0,
            total: None,
            status: SlotStatus::Idle,
            started: Instant::now(),
        }
    }
}

impl DownloadProgress for SlotProgress {
    fn reset(&self) {
        let mut slots = self
            .inner
            .slots
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(slot) = slots.get_mut(self.index) {
            slot.current = 0;
            slot.total = None;
            slot.status = SlotStatus::Active;
            slot.started = Instant::now();
        }
    }

    fn set_total(&self, total: u64) {
        let mut slots = self
            .inner
            .slots
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(slot) = slots.get_mut(self.index) {
            slot.total = Some(total);
        }
    }

    fn add_bytes(&self, bytes: u64) {
        let mut slots = self
            .inner
            .slots
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(slot) = slots.get_mut(self.index) {
            slot.current = slot.current.saturating_add(bytes);
        }
    }
}

fn clear_rendered_lines(stderr: &mut std::io::Stderr, lines: usize) {
    if lines == 0 {
        return;
    }
    let _ = write!(stderr, "\x1b[{lines}A");
    for _ in 0..lines {
        let _ = write!(stderr, "\r\x1b[2K\n");
    }
    let _ = write!(stderr, "\x1b[{lines}A");
    let _ = stderr.flush();
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{}{}", bytes, UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

fn shorten_middle(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let prefix_len = keep / 2;
    let suffix_len = keep - prefix_len;
    let prefix = value.chars().take(prefix_len).collect::<String>();
    let suffix = value
        .chars()
        .rev()
        .take(suffix_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{prefix}...{suffix}")
}
