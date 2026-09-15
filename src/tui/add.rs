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
    pending: Option<Receiver<Result<Output>>>,
    control: Control,
}
pub enum Output {
    CurseForge(Preview),
    Source(crate::catalog::source::Prepared),
    GitHub(super::github::Picker, super::form::Form),
}
impl Workflow {
    pub fn github(&mut self, action: super::github::Action) -> Result<()> {
        self.run(move |control| {
            use super::github::{Action, Picker};
            let client = crate::catalog::github::Client {
                transport: crate::catalog::github::Http,
            };
            control.check()?;
            let (picker, form) = match action {
                Action::Releases { repository, page } => {
                    let releases = client.releases(&repository, page)?;
                    Picker::releases(repository, page, releases)
                }
                Action::Assets {
                    repository,
                    release,
                    page,
                } => {
                    let assets = client.assets(&repository, release.id, page)?;
                    Picker::assets(repository, release, page, assets)
                }
                Action::Add(_) => {
                    return Err(Error::new(ErrorCode::Invalid, "unexpected_add_action"));
                }
            };
            Ok(Output::GitHub(picker, form))
        })
    }
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
        let root = root.to_path_buf();
        let state = state.to_path_buf();
        self.run(move |control| {
            Client::for_pack(&root)
                .and_then(|client| {
                    prepare::curseforge(
                        &root,
                        &state,
                        &client,
                        selected.file,
                        side,
                        selected.relaxed,
                        &control,
                    )
                })
                .map(Output::CurseForge)
        })
    }
    pub fn source(
        &mut self,
        root: &Path,
        state: &Path,
        input: crate::catalog::source::Input,
        options: crate::catalog::source::Options,
    ) -> Result<()> {
        let root = root.to_path_buf();
        let state = state.to_path_buf();
        self.run(move |control| {
            crate::catalog::source::prepare(&root, &state, input, options, &control)
                .map(Output::Source)
        })
    }
    fn run(&mut self, work: impl FnOnce(Control) -> Result<Output> + Send + 'static) -> Result<()> {
        if self.busy() {
            return Err(Error::new(ErrorCode::Busy, "preview_running"));
        }
        self.control = Control::default();
        let control = self.control.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let result = control
                .check()
                .and_then(|()| work(control.clone()))
                .and_then(|output| {
                    control.check()?;
                    Ok(output)
                });
            let _ = tx.send(result);
        });
        Ok(())
    }
    pub fn poll(&mut self) -> Option<Result<Output>> {
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
