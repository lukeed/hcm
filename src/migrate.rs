//! Move snapshot workspaces into hc: shells in their folders, spaces and tabs in the `herdr-app` doc.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use doc::{Document, Node, Prefix, Tab};
use hc::{Client, HcError, ListParams, RunParams};
use serde_json::json;

use crate::snapshot::Workspace;

/// Label on a moved terminal: the Herdr tab it came from.
pub const FROM: &str = "migrate.from";
/// Label on a moved terminal: its agent's resume command, emptied once typed.
pub const RESUME: &str = "migrate.resume";

const SEND_TIMEOUT: Duration = Duration::from_secs(90);

/// What hc already has: a space per folder, and the Herdr tabs moved before.
pub struct State {
    pub spaces: BTreeMap<String, String>,
    pub moved: BTreeSet<String>,
}

/// One workspace that `apply` added: to a new space, or to the space its folder already had.
#[derive(Debug, PartialEq, Eq)]
pub struct Added {
    pub folder: String,
    pub fresh: bool,
    pub tabs: usize,
}

struct Started {
    space: String,
    fresh: bool,
    workspace: usize,
    tabs: Vec<(String, String, String, usize)>,
}

async fn current(client: &Client) -> Result<(Document, u64), String> {
    let got = client
        .doc_get(doc::NAME, None)
        .await
        .map_err(|e| format!("reading the herdr-app doc: {e}"))?;
    if got.value.is_null() {
        return Ok((Document::default(), got.version));
    }
    let document =
        Document::parse(&got.value).map_err(|e| format!("reading the herdr-app doc: {e}"))?;
    Ok((document, got.version))
}

/// The spaces and moved tabs hc has now.
pub async fn state(client: &Client) -> Result<State, String> {
    let (document, _) = current(client).await?;
    let spaces = document
        .spaces
        .iter()
        .map(|s| (s.folder.clone(), s.id.clone()))
        .collect();
    let moved = client
        .list(&ListParams::default())
        .await
        .map_err(|e| format!("listing terminals: {e}"))?
        .terminals
        .iter()
        .filter_map(|t| t.labels.get(FROM).cloned())
        .collect();
    Ok(State { spaces, moved })
}

async fn start(
    client: &Client,
    workspaces: &[Workspace],
    state: &State,
) -> Result<Vec<Started>, String> {
    let mut started = Vec::new();
    for (index, workspace) in workspaces.iter().enumerate() {
        let todo: Vec<usize> = (0..workspace.tabs.len())
            .filter(|&n| !state.moved.contains(&workspace.tabs[n].from))
            .collect();
        if todo.is_empty() {
            continue;
        }
        let (space, fresh) = state.spaces.get(&workspace.folder).map_or_else(
            || (doc::fresh_id(Prefix::Space), true),
            |id| (id.clone(), false),
        );
        let mut tabs = Vec::new();
        for n in todo {
            let tab = &workspace.tabs[n];
            let tab_id = doc::fresh_id(Prefix::Tab);
            let mut labels = BTreeMap::from([
                ("herdr.space".to_owned(), space.clone()),
                ("herdr.tab".to_owned(), tab_id.clone()),
                ("herdr.seen".to_owned(), "0".to_owned()),
                (FROM.to_owned(), tab.from.clone()),
            ]);
            if let Some(resume) = &tab.resume {
                labels.insert(RESUME.to_owned(), resume.clone());
            }
            let params = RunParams {
                cwd: Some(tab.cwd.clone()),
                labels,
                keep: true,
                no_parent: true,
                no_notify: true,
                ..RunParams::default()
            };
            let terminal = client
                .run(&params)
                .await
                .map_err(|e| format!("starting a shell in {}: {e}", tab.cwd))?;
            tabs.push((tab_id, doc::fresh_id(Prefix::Pane), terminal.address, n));
        }
        started.push(Started {
            space,
            fresh,
            workspace: index,
            tabs,
        });
    }
    Ok(started)
}

fn place(
    document: &mut Document,
    workspaces: &[Workspace],
    started: &[Started],
) -> Result<(), String> {
    for s in started {
        let workspace = &workspaces[s.workspace];
        for (k, (tab_id, pane_id, address, n)) in s.tabs.iter().enumerate() {
            let node = Node::terminal(pane_id.clone(), address.clone());
            let tab = Tab::new(tab_id.clone(), workspace.tabs[*n].name.clone(), node);
            let edit = if s.fresh && k == 0 {
                document.new_space(&s.space, &workspace.name, &workspace.folder, tab)
            } else {
                document.new_tab(&s.space, tab, None)
            };
            edit.map_err(|e| format!("{}: {e:?}", workspace.folder))?;
        }
    }
    Ok(())
}

/// Start a shell for every tab not moved yet, then add them to the `herdr-app` doc in one write.
pub async fn apply(
    client: &Client,
    workspaces: &[Workspace],
    state: &State,
) -> Result<Vec<Added>, String> {
    let started = start(client, workspaces, state).await?;
    if started.is_empty() {
        return Ok(Vec::new());
    }
    for _ in 0..5 {
        let (mut document, version) = current(client).await?;
        place(&mut document, workspaces, &started)?;
        match client
            .doc_set(doc::NAME, &document.to_value(), version, None)
            .await
        {
            Ok(_) => {
                return Ok(started
                    .iter()
                    .map(|s| Added {
                        folder: workspaces[s.workspace].folder.clone(),
                        fresh: s.fresh,
                        tabs: s.tabs.len(),
                    })
                    .collect());
            }
            Err(HcError::Conflict(_)) => {}
            Err(e) => return Err(format!("writing the herdr-app doc: {e}")),
        }
    }
    let shells: Vec<&str> = started
        .iter()
        .flat_map(|s| s.tabs.iter().map(|t| t.2.as_str()))
        .collect();
    Err(format!(
        "the herdr-app doc kept changing, so the new shells are not in it: hc kill {}",
        shells.join(" ")
    ))
}

/// Type each waiting agent's resume command into its pane, once.
pub async fn resume(client: &Client) -> Result<Vec<(String, String)>, String> {
    let list = client
        .list(&ListParams::default())
        .await
        .map_err(|e| format!("listing terminals: {e}"))?;
    let waiting: Vec<(String, String)> = list
        .terminals
        .iter()
        .filter_map(|t| Some((t.address.clone(), t.labels.get(RESUME)?.clone())))
        .filter(|(_, line)| !line.is_empty())
        .collect();
    for (address, line) in &waiting {
        let _: serde_json::Value = client
            .call_within(
                "terminal.send",
                json!({"terminal": address, "text": line}),
                SEND_TIMEOUT,
            )
            .await
            .map_err(|e| format!("typing into {address}: {e}"))?;
        client
            .label(address, [(RESUME, "")])
            .await
            .map_err(|e| format!("labelling {address}: {e}"))?;
    }
    Ok(waiting)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use hc::TestCore;
    use serde_json::{Value, json};

    use super::*;
    use crate::snapshot;

    fn herdr(root: &Path) -> Value {
        let folder = |name: &str| {
            let path = root.join(name);
            std::fs::create_dir_all(&path).unwrap();
            path.display().to_string()
        };
        let (api, notes) = (folder("api"), folder("notes"));
        json!({"workspaces": [
            {"id": "w1", "custom_name": null, "identity_cwd": api, "tabs": [
                {"custom_name": "Dataflare", "layout": {"Pane": 1}, "panes": {"1": {
                    "cwd": api, "agent_session": {"agent": "claude", "value": "abc-123"}
                }}},
                {"custom_name": null, "layout": {"Pane": 2}, "panes": {"2": {"cwd": api}}}
            ]},
            {"id": "w2", "custom_name": "notes", "identity_cwd": notes, "tabs": [
                {"custom_name": null, "layout": {"Pane": 1}, "panes": {"1": {"cwd": notes}}}
            ]}
        ]})
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apply_merges_into_hc_once_and_resume_types_once() {
        let core = TestCore::start_with(|root, env| {
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let fake = bin.join("claude");
            let log = root.join("claude.log");
            std::fs::write(
                &fake,
                format!("#!/bin/sh\necho \"$@\" >> '{}'\n", log.display()),
            )
            .unwrap();
            let mut perms = std::fs::metadata(&fake).unwrap().permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
            std::fs::set_permissions(&fake, perms).unwrap();
            let path = format!(
                "{}:{}",
                bin.display(),
                env.get("PATH").unwrap_or_default().to_string_lossy()
            );
            *env = env.clone().with("PATH", path);
        })
        .await;
        let client = core.client().await;
        let workspaces = snapshot::read(&herdr(core.root())).unwrap();
        let notes = workspaces[1].folder.clone();

        let (mut document, version) = current(&client).await.unwrap();
        let shell = client
            .run(&RunParams {
                cwd: Some(notes.clone()),
                ..RunParams::default()
            })
            .await
            .unwrap();
        let tab = Tab::new(
            doc::fresh_id(Prefix::Tab),
            "",
            Node::terminal(doc::fresh_id(Prefix::Pane), shell.address),
        );
        document
            .new_space(&doc::fresh_id(Prefix::Space), "", &notes, tab)
            .unwrap();
        client
            .doc_set(doc::NAME, &document.to_value(), version, None)
            .await
            .unwrap();

        let added = apply(&client, &workspaces, &state(&client).await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            added,
            vec![
                Added {
                    folder: workspaces[0].folder.clone(),
                    fresh: true,
                    tabs: 2
                },
                Added {
                    folder: notes.clone(),
                    fresh: false,
                    tabs: 1
                },
            ]
        );
        let (document, _) = current(&client).await.unwrap();
        assert_eq!(
            document.spaces.len(),
            2,
            "the notes tab joins the space it already had"
        );
        let api = document
            .spaces
            .iter()
            .find(|s| s.folder == workspaces[0].folder)
            .unwrap();
        assert_eq!(api.tabs.len(), 2);
        assert_eq!(api.tabs[0].name, "Dataflare");
        let notes_space = document.spaces.iter().find(|s| s.folder == notes).unwrap();
        assert_eq!(notes_space.tabs.len(), 2);

        let again = apply(&client, &workspaces, &state(&client).await.unwrap())
            .await
            .unwrap();
        assert!(again.is_empty(), "a second apply moves nothing: {again:?}");
        assert_eq!(current(&client).await.unwrap().0.spaces.len(), 2);

        let typed = resume(&client).await.unwrap();
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].1, "claude --resume abc-123");
        let log = core.root().join("claude.log");
        until("the fake claude to run", || {
            std::fs::read_to_string(&log).is_ok_and(|l| !l.is_empty())
        })
        .await;
        assert!(
            resume(&client).await.unwrap().is_empty(),
            "resume types once"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "--resume abc-123\n");
    }
}
