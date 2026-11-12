//! Herdr TUI session snapshots, read as the workspaces to move.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// One Herdr workspace: the folder it is tied to and its tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub name: String,
    pub folder: String,
    pub tabs: Vec<TabPlan>,
}

/// One Herdr tab with its single pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabPlan {
    /// `<workspace id>/<tab index>`, so a moved tab is never moved twice.
    pub from: String,
    pub name: String,
    pub cwd: String,
    /// The command that resumes the pane's agent conversation, when it had one.
    pub resume: Option<String>,
}

/// Where Herdr keeps its session snapshots.
pub fn default_dir(home: &Path) -> PathBuf {
    home.join(".config/herdr/session-snapshots")
}

/// The newest `.json` snapshot in `dir`.
pub fn latest(dir: &Path) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max()
        .map(|(_, path)| path)
        .ok_or_else(|| format!("no snapshot in {}", dir.display()))
}

/// The command that resumes an agent conversation, or none for an agent without one.
pub fn resume_line(agent: &str, id: &str) -> Result<Option<String>, String> {
    let safe = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if id.is_empty() || !id.chars().all(safe) {
        return Err(format!("odd {agent} session id {id:?}"));
    }
    Ok(match agent {
        "claude" => Some(format!("claude --resume {id}")),
        "codex" => Some(format!("codex resume {id}")),
        _ => None,
    })
}

fn name_of(value: &Value) -> String {
    value.as_str().unwrap_or_default().trim().to_owned()
}

fn tab(id: &str, folder: &str, index: usize, tab: &Value) -> Result<TabPlan, String> {
    let Some(pane) = tab["layout"]["Pane"].as_u64() else {
        return Err(format!(
            "{folder}: a tab with split panes ({}); only one pane per tab can move",
            tab["layout"]
        ));
    };
    let pane = &tab["panes"][pane.to_string()];
    let session = &pane["agent_session"];
    let resume = match (session["agent"].as_str(), session["value"].as_str()) {
        (Some(agent), Some(value)) => resume_line(agent, value)?,
        _ => None,
    };
    Ok(TabPlan {
        from: format!("{id}/{index}"),
        name: name_of(&tab["custom_name"]),
        cwd: pane["cwd"].as_str().unwrap_or(folder).to_owned(),
        resume,
    })
}

/// Every workspace in a snapshot, in Herdr's order.
pub fn read(snapshot: &Value) -> Result<Vec<Workspace>, String> {
    let workspaces = snapshot["workspaces"]
        .as_array()
        .ok_or("no workspaces in the snapshot")?;
    workspaces
        .iter()
        .map(|ws| {
            let folder = ws["identity_cwd"]
                .as_str()
                .ok_or("a workspace has no identity_cwd")?;
            let id = ws["id"].as_str().ok_or("a workspace has no id")?;
            let tabs = ws["tabs"]
                .as_array()
                .ok_or("a workspace has no tabs")?
                .iter()
                .enumerate()
                .map(|(index, t)| tab(id, folder, index, t))
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Workspace {
                name: name_of(&ws["custom_name"]),
                folder: folder.to_owned(),
                tabs,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> Value {
        json!({"workspaces": [
            {"id": "w1", "custom_name": null, "identity_cwd": "/w/api", "tabs": [
                {"custom_name": "Dataflare", "layout": {"Pane": 1}, "panes": {"1": {
                    "cwd": "/w/api/sub",
                    "agent_session": {"agent": "claude", "kind": "id", "value": "6ce658b4-a87a"}
                }}},
                {"custom_name": null, "layout": {"Pane": 3}, "panes": {"3": {"cwd": "/w/api"}}}
            ]},
            {"id": "w2", "custom_name": " notes ", "identity_cwd": "/w/notes", "tabs": [
                {"layout": {"Pane": 1}, "panes": {"1": {
                    "agent_session": {"agent": "codex", "value": "019a"}
                }}}
            ]}
        ]})
    }

    #[test]
    fn workspaces_keep_their_order_names_folders_and_agents() {
        let read = read(&snapshot()).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].name, "");
        assert_eq!(read[0].folder, "/w/api");
        assert_eq!(
            read[0].tabs[0],
            TabPlan {
                from: "w1/0".into(),
                name: "Dataflare".into(),
                cwd: "/w/api/sub".into(),
                resume: Some("claude --resume 6ce658b4-a87a".into()),
            }
        );
        assert_eq!(read[0].tabs[1].from, "w1/1");
        assert_eq!(read[0].tabs[1].resume, None);
        assert_eq!(read[1].name, "notes");
        assert_eq!(read[1].tabs[0].cwd, "/w/notes");
        assert_eq!(read[1].tabs[0].resume.as_deref(), Some("codex resume 019a"));
    }

    #[test]
    fn a_split_tab_or_an_odd_session_id_stops_the_read() {
        let mut split = snapshot();
        split["workspaces"][0]["tabs"][0]["layout"] = json!({"Split": {}});
        assert!(read(&split).unwrap_err().contains("split panes"));
        let mut odd = snapshot();
        odd["workspaces"][1]["tabs"][0]["panes"]["1"]["agent_session"]["value"] =
            json!("x; rm -rf ~");
        assert!(read(&odd).unwrap_err().contains("odd codex session id"));
        assert_eq!(resume_line("aider", "abc"), Ok(None));
    }

    #[test]
    fn the_newest_snapshot_wins() {
        let dir = tempfile::tempdir().unwrap();
        assert!(latest(dir.path()).is_err());
        std::fs::write(dir.path().join("old.json"), "{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.path().join("new.json"), "{}").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "").unwrap();
        assert_eq!(latest(dir.path()).unwrap(), dir.path().join("new.json"));
    }
}
