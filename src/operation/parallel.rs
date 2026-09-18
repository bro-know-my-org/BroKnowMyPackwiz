//! Bounded independent work with ordered results and completion progress.
use crate::operation::{Control, Event, Result};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

/// Bound worker count and aggregate successful results in input order.
pub(crate) fn map<T: Sync, R: Send>(
    items: &[T],
    jobs: usize,
    control: &Control,
    label: &str,
    query: impl Fn(&T) -> Result<R> + Sync,
) -> Result<Vec<R>> {
    control.check()?;
    control.emit(Event::Progress {
        label: label.into(),
        current: 0,
        total: Some(items.len() as u64),
    });
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..jobs.clamp(1, 64).min(items.len()) {
            let (tx, next, stopped, query) = (tx.clone(), &next, &stopped, &query);
            scope.spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else { break };
                    let result = control.check().and_then(|()| query(item));
                    if result.is_err() {
                        stopped.store(true, Ordering::Release);
                    }
                    if tx.send((index, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let mut results = Vec::new();
        for result in rx {
            results.push(result);
            control.emit(Event::Progress {
                label: label.into(),
                current: results.len() as u64,
                total: Some(items.len() as u64),
            });
        }
        control.check()?;
        results.sort_by_key(|(index, _)| *index);
        results.into_iter().map(|(_, result)| result).collect()
    })
}
