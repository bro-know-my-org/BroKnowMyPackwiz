use crate::{
    catalog::{
        curseforge::Client,
        prepare::{self, Preview},
    },
    metadata::Side,
    operation::{Control, Error, ErrorCode, Result},
};
use std::{
    path::Path,
    sync::mpsc::{self, Receiver},
};

#[derive(Default)]
pub struct Workflow {
    pending: Option<Receiver<Result<Preview>>>,
    control: Control,
}
impl Workflow {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn cancel(&self) {
        self.control.cancel();
    }
    pub fn start(
        &mut self,
        root: &Path,
        state: &Path,
        selected: super::catalog::Selection,
        side: Side,
    ) -> Result<()> {
        if self.busy() {
            return Err(Error::new(ErrorCode::Busy, "preview_running"));
        }
        self.control = Control::default();
        let control = self.control.clone();
        let root = root.to_path_buf();
        let state = state.to_path_buf();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let result = Client::for_pack(&root).and_then(|client| {
                prepare::curseforge(
                    &root,
                    &state,
                    &client,
                    selected.file,
                    side,
                    selected.relaxed,
                    &control,
                )
            });
            let _ = tx.send(result);
        });
        Ok(())
    }
    pub fn poll(&mut self) -> Option<Result<Preview>> {
        let rx = self.pending.as_ref()?;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err(Error::new(
                ErrorCode::Interrupted,
                "preview_worker_disconnected",
            )),
        };
        self.pending = None;
        Some(result)
    }
}
impl Drop for Workflow {
    fn drop(&mut self) {
        self.cancel();
    }
}
