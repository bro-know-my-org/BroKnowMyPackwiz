//! Read-only dependency planning. Every proposed file is frozen before confirmation.
use super::curseforge::{Client, File, Filter, Transport};
use crate::{
    metadata::Side,
    operation::{Control, Error, ErrorCode, Result},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Installed {
    pub file: File,
    pub side: Side,
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Add,
    Update,
    Reuse,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub file: File,
    pub side: Side,
    pub action: Action,
}

pub trait Source {
    fn latest(&self, project: u64, filter: &Filter) -> Result<File>;
}
impl<T: Transport> Source for Client<T> {
    fn latest(&self, project: u64, filter: &Filter) -> Result<File> {
        self.latest_compatible(project, filter)
    }
}

/// Unknown side values cannot establish dependency coverage.
fn covers(have: &Side, need: &Side) -> bool {
    !matches!(have, Side::Unknown(_))
        && !matches!(need, Side::Unknown(_))
        && (have == need || *have == Side::Both)
}
fn overlaps(a: &Side, b: &Side) -> bool {
    // Unknown sides are conservatively conflicting, never silently ignored.
    !matches!(
        (a, b),
        (Side::Client, Side::Server) | (Side::Server, Side::Client)
    )
}
fn conflict(reason: &str, project: u64) -> Error {
    Error::new(ErrorCode::Conflict, format!("{reason}: {project}"))
}

/// Entries are dependency-first, deterministic and deduplicated. Installed files
/// include their own dependency declarations so reverse incompatibilities count.
pub fn plan(
    source: &impl Source,
    selected: File,
    side: Side,
    filter: &Filter,
    installed: &[Installed],
    control: &Control,
) -> Result<Vec<Entry>> {
    control.check()?;
    if matches!(side, Side::Unknown(_)) {
        return Err(conflict("dependency_side_unknown", selected.project_id));
    }
    let mut existing = BTreeMap::new();
    for item in installed {
        if existing.insert(item.file.project_id, item).is_some() {
            return Err(conflict(
                "duplicate_installed_project",
                item.file.project_id,
            ));
        }
    }
    let mut planner = Planner {
        source,
        filter,
        existing,
        active: BTreeSet::new(),
        entries: Vec::new(),
        control,
    };
    planner.visit(selected, side)?;
    // Compare the complete resulting selection, including untouched installed
    // files, in both directions. A replaced file's old conflicts no longer apply.
    let mut final_files: BTreeMap<_, _> = planner
        .existing
        .iter()
        .map(|(id, item)| (*id, (&item.file, &item.side)))
        .collect();
    for entry in &planner.entries {
        final_files.insert(entry.file.project_id, (&entry.file, &entry.side));
    }
    for (file, side) in final_files.values() {
        control.check()?;
        for dep in file.dependencies.iter().filter(|d| d.relation == 5) {
            if let Some((_, other_side)) = final_files.get(&dep.project_id) {
                if overlaps(side, other_side) {
                    return Err(conflict("incompatible_dependency", dep.project_id));
                }
            }
        }
    }
    Ok(planner.entries)
}

struct Planner<'a, S> {
    source: &'a S,
    filter: &'a Filter,
    existing: BTreeMap<u64, &'a Installed>,
    active: BTreeSet<u64>,
    entries: Vec<Entry>,
    control: &'a Control,
}
impl<S: Source> Planner<'_, S> {
    fn visit(&mut self, file: File, requested_side: Side) -> Result<()> {
        self.control.check()?;
        let id = file.project_id;
        if self.active.contains(&id) {
            return Err(conflict("dependency_cycle", id));
        }
        if let Some(previous) = self.entries.iter().find(|e| e.file.project_id == id) {
            if previous.file.id != file.id || !covers(&previous.side, &requested_side) {
                return Err(conflict("dependency_selection_conflict", id));
            }
            return Ok(());
        }
        if self.active.len() >= 128 || self.entries.len() + self.active.len() >= 512 {
            return Err(conflict("dependency_graph_too_large", id));
        }
        if !file.compatible(self.filter) {
            return Err(conflict("dependency_version_mismatch", id));
        }
        let (action, side) = match self.existing.get(&id) {
            None => (Action::Add, requested_side),
            Some(old) => {
                if !covers(&old.side, &requested_side) {
                    return Err(conflict("dependency_side_mismatch", id));
                }
                if old.file.id == file.id {
                    (Action::Reuse, old.side.clone())
                } else if old.pinned {
                    return Err(conflict("dependency_pinned", id));
                } else {
                    (Action::Update, old.side.clone())
                }
            }
        };
        self.active.insert(id);
        for dep in file.dependencies.iter().filter(|d| d.relation == 3) {
            self.control.check()?;
            if self.active.contains(&dep.project_id) {
                return Err(conflict("dependency_cycle", dep.project_id));
            }
            let next = if let Some(entry) = self
                .entries
                .iter()
                .find(|e| e.file.project_id == dep.project_id)
            {
                entry.file.clone()
            } else if let Some(old) = self
                .existing
                .get(&dep.project_id)
                .filter(|old| old.file.compatible(self.filter))
            {
                old.file.clone()
            } else {
                if self
                    .existing
                    .get(&dep.project_id)
                    .is_some_and(|old| old.pinned)
                {
                    return Err(conflict("dependency_pinned", dep.project_id));
                }
                self.source.latest(dep.project_id, self.filter)?
            };
            if next.project_id != dep.project_id {
                return Err(conflict("dependency_identity_mismatch", dep.project_id));
            }
            self.visit(next, side.clone())?;
        }
        self.active.remove(&id);
        self.entries.push(Entry { file, side, action });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::curseforge::Dependency;
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        files: BTreeMap<u64, File>,
        queries: RefCell<Vec<u64>>,
    }
    impl Source for Fake {
        fn latest(&self, id: u64, _: &Filter) -> Result<File> {
            self.queries.borrow_mut().push(id);
            self.files
                .get(&id)
                .cloned()
                .ok_or_else(|| conflict("missing", id))
        }
    }
    fn file(id: u64, deps: &[(u64, u64)]) -> File {
        File {
            project_id: id,
            id: id * 100,
            name: format!("Mod {id}"),
            filename: format!("mod-{id}.jar"),
            versions: vec!["1.21.1".into(), "Fabric".into()],
            date: String::new(),
            release_type: 1,
            size: 10,
            sha1: "a".repeat(40),
            dependencies: deps
                .iter()
                .map(|(project_id, relation)| Dependency {
                    project_id: *project_id,
                    relation: *relation,
                })
                .collect(),
        }
    }
    fn fake(files: Vec<File>) -> Fake {
        Fake {
            files: files.into_iter().map(|f| (f.project_id, f)).collect(),
            queries: RefCell::default(),
        }
    }
    fn filter() -> Filter {
        Filter {
            minecraft: Some("1.21.1".into()),
            loader: Some("fabric".into()),
        }
    }
    fn run(source: &Fake, root: File, installed: &[Installed]) -> Result<Vec<Entry>> {
        plan(
            source,
            root,
            Side::Both,
            &filter(),
            installed,
            &Control::default(),
        )
    }
    #[test]
    fn diamond_closure_is_deduplicated_and_optional_dependencies_are_ignored() {
        let source = fake(vec![file(2, &[(4, 3)]), file(3, &[(4, 3)]), file(4, &[])]);
        let result = run(&source, file(1, &[(2, 3), (3, 3), (9, 2)]), &[]).unwrap();
        assert_eq!(
            result.iter().map(|e| e.file.project_id).collect::<Vec<_>>(),
            vec![4, 2, 3, 1]
        );
        assert_eq!(*source.queries.borrow(), vec![2, 4, 3]);
        assert!(result.iter().all(|e| e.action == Action::Add));
    }
    #[test]
    fn cycles_and_reverse_incompatibilities_block_planning() {
        assert!(
            run(&fake(vec![file(2, &[(1, 3)])]), file(1, &[(2, 3)]), &[])
                .unwrap_err()
                .detail
                .contains("cycle")
        );
        let installed = [Installed {
            file: file(2, &[(1, 5)]),
            side: Side::Both,
            pinned: false,
        }];
        assert!(
            run(&fake(vec![]), file(1, &[]), &installed)
                .unwrap_err()
                .detail
                .contains("incompatible")
        );
    }
    #[test]
    fn compatible_installed_dependency_is_reused_but_pinned_mismatch_blocks() {
        let mut installed = Installed {
            file: file(2, &[]),
            side: Side::Both,
            pinned: true,
        };
        let source = fake(vec![file(2, &[])]);
        let result = run(&source, file(1, &[(2, 3)]), &[installed.clone()]).unwrap();
        assert_eq!(result[0].action, Action::Reuse);
        assert!(source.queries.borrow().is_empty());
        installed.file.versions = vec!["1.20.1".into()];
        assert!(
            run(&source, file(1, &[(2, 3)]), &[installed.clone()])
                .unwrap_err()
                .detail
                .contains("pinned")
        );
        installed.pinned = false;
        installed.file.id = 99;
        assert_eq!(
            run(&source, file(1, &[(2, 3)]), &[installed]).unwrap()[0].action,
            Action::Update
        );
    }
    #[test]
    fn insufficient_side_and_duplicate_installed_identity_are_not_silently_reused() {
        let installed = Installed {
            file: file(2, &[]),
            side: Side::Server,
            pinned: false,
        };
        assert!(
            run(&fake(vec![]), file(1, &[(2, 3)]), &[installed.clone()])
                .unwrap_err()
                .detail
                .contains("side")
        );
        assert!(
            run(&fake(vec![]), file(1, &[]), &[installed.clone(), installed])
                .unwrap_err()
                .detail
                .contains("duplicate")
        );
    }
    #[test]
    fn disjoint_side_incompatibilities_and_explicit_relaxed_filter() {
        let installed = [Installed {
            file: file(2, &[(1, 5)]),
            side: Side::Server,
            pinned: false,
        }];
        let mut selected = file(1, &[]);
        selected.versions.clear();
        assert!(
            plan(
                &fake(vec![]),
                selected.clone(),
                Side::Client,
                &filter(),
                &installed,
                &Control::default()
            )
            .is_err()
        );
        assert!(
            plan(
                &fake(vec![]),
                selected,
                Side::Client,
                &Filter::default(),
                &installed,
                &Control::default()
            )
            .is_ok()
        );
    }
    #[test]
    fn cancellation_prevents_network_queries() {
        let control = Control::default();
        control.cancel();
        let source = fake(vec![]);
        assert_eq!(
            plan(
                &source,
                file(1, &[(2, 3)]),
                Side::Both,
                &filter(),
                &[],
                &control
            )
            .unwrap_err()
            .code,
            ErrorCode::Cancelled
        );
        assert!(source.queries.borrow().is_empty());
    }
}
