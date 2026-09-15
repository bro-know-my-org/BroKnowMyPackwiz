//! Thread-local filesystem fault injection. Compiled only in unit tests.
use std::{cell::RefCell, io, path::Path, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    WriteBefore,
    WritePublished,
    ReplaceBefore,
    ReplacePublished,
    BackupBefore,
    BackupCopied,
}

type Hook = Rc<dyn Fn(Point, &Path) -> io::Result<()>>;
thread_local! {
    static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

pub struct Scope;
impl Scope {
    pub fn new(hook: impl Fn(Point, &Path) -> io::Result<()> + 'static) -> Self {
        HOOK.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none(), "nested filesystem fault injection");
            *slot = Some(Rc::new(hook));
        });
        Self
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        HOOK.with(|slot| *slot.borrow_mut() = None);
    }
}

pub fn check(point: Point, path: &Path) -> io::Result<()> {
    let hook = HOOK.with(|slot| slot.borrow().clone());
    hook.map_or(Ok(()), |hook| hook(point, path))
}
