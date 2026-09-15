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
    UpdatePreview(crate::catalog::updates::Preview),
    UpdateReady(crate::catalog::update_plan::Planned),
    UpdateDirect(crate::catalog::update_plan::Planned),
    CurseForge(Preview),
    Source(crate::catalog::source::Prepared),
    GitHub(super::github::Picker, super::form::Form),
}
impl Workflow {
    pub fn update(
        &mut self,
        root: &Path,
        state: &Path,
        paths: Vec<String>,
        direct: bool,
    ) -> Result<()> {
        let root = root.to_path_buf();
        let state = state.to_path_buf();
        self.run(move |control| {
            let preview = crate::catalog::updates::query(
                &root,
                &paths,
                &crate::catalog::updates::Online { root: &root },
                &control,
            )?;
            if direct {
                let selected = preview
                    .candidates
                    .iter()
                    .map(|c| c.relative.clone())
                    .collect::<Vec<_>>();
                crate::catalog::update_plan::prepare(
                    &root, &state, &preview, &selected, false, &control,
                )
                .map(Output::UpdateDirect)
            } else {
                Ok(Output::UpdatePreview(preview))
            }
        })
    }
    pub fn apply_updates(
        &mut self,
        root: &Path,
        state: &Path,
        preview: crate::catalog::updates::Preview,
        paths: Vec<String>,
        download: bool,
    ) -> Result<()> {
        let root = root.to_path_buf();
        let state = state.to_path_buf();
        self.run(move |control| {
            crate::catalog::update_plan::prepare(
                &root, &state, &preview, &paths, download, &control,
            )
            .map(Output::UpdateReady)
        })
    }
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
                .map(|mut preview| {
                    preview.download = selected.download;
                    Output::CurseForge(preview)
                })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{
        durable, edit,
        preview::Guard,
        queue::{Queue, Request, Status},
    };
    use std::{
        fs,
        time::{Duration, Instant},
    };
    #[test]
    fn direct_updates_submit_the_same_prepared_request_without_a_confirmation_dialog() {
        let base = std::env::temp_dir().join(durable::unique_id());
        fs::create_dir_all(base.join("pack/mods")).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let root = base.join("pack");
        let state = base.join("state");
        fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
        let guard = Guard::capture(&root, &Control::default()).unwrap();
        let draft = edit::draft(
            &state,
            "mods/new.pw.toml",
            None,
            &"name = \"New\"".parse().unwrap(),
        )
        .unwrap();
        let planned = crate::catalog::update_plan::Planned {
            request: Request::PreparedEdit {
                drafts: vec![draft],
                guard,
            },
            rows: Vec::new(),
            download: false,
        };
        let mut app = super::super::app::App::new(root.clone());
        app.jobs.queue = Some(Queue::open(&root, &state).unwrap());
        app.adding
            .run(move |_| Ok(Output::UpdateDirect(planned)))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.poll();
            let queue = app.jobs.queue.as_ref().unwrap();
            if queue.tasks.len() == 1 && !queue.pending() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.dialog.is_none());
        assert_eq!(
            app.jobs.queue.as_ref().unwrap().tasks[0].status,
            Status::Completed
        );
        assert!(root.join("mods/new.pw.toml").exists());
        drop(app);
        fs::remove_dir_all(base).unwrap();
    }
}
