//! hcm: move Herdr TUI workspaces into herdr-core (hc).

mod migrate;
mod snapshot;

use std::path::PathBuf;
use std::process::ExitCode;

use hc::{Client, Env};

use crate::migrate::State;
use crate::snapshot::Workspace;

const USAGE: &str = "usage: hcm [plan|apply|resume] [--snapshot <file>]

Move Herdr TUI workspaces into herdr-core (hc), so rodeo and the Herdr app show
them. Running programs cannot move; each tab gets a new shell in its folder.

  plan    print what apply would do (the default; changes nothing)
  apply   start a shell in hc for every Herdr tab not moved yet, and add the
          spaces and tabs to the herdr-app doc; a folder that already has a
          space gets the new tabs
  resume  type each agent's resume command into its new pane, once; run it
          only after the Herdr copies of those agents have stopped

The snapshot defaults to the newest in ~/.config/herdr/session-snapshots.
HC_SESSION picks the hc session, as for hc itself.";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hcm: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_plan(workspaces: &[Workspace], state: &State) {
    let tabs: usize = workspaces.iter().map(|w| w.tabs.len()).sum();
    let agents = workspaces
        .iter()
        .flat_map(|w| &w.tabs)
        .filter(|t| t.resume.is_some())
        .count();
    println!(
        "{} workspaces, {tabs} tabs, {agents} agents to resume",
        workspaces.len()
    );
    for workspace in workspaces {
        let name = if workspace.name.is_empty() {
            "(folder name)"
        } else {
            &workspace.name
        };
        let note = if state.spaces.contains_key(&workspace.folder) {
            "  (adds to the space hc already has here)"
        } else {
            ""
        };
        println!("\n{name}  {}{note}", workspace.folder);
        for (n, tab) in workspace.tabs.iter().enumerate() {
            let label = if tab.name.is_empty() {
                format!("tab {}", n + 1)
            } else {
                tab.name.clone()
            };
            let cwd = if tab.cwd == workspace.folder {
                String::new()
            } else {
                format!("  in {}", tab.cwd)
            };
            let resume = tab
                .resume
                .as_deref()
                .map(|r| format!("  → {r}"))
                .unwrap_or_default();
            let done = if state.moved.contains(&tab.from) {
                "  (moved already)"
            } else {
                ""
            };
            println!("  {label}{cwd}{resume}{done}");
        }
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut mode = String::from("plan");
    let mut file = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "plan" | "apply" | "resume" => mode = arg,
            "--snapshot" => file = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => return Err(USAGE.to_owned()),
        }
    }
    let client = Client::connect_env(&Env::current())
        .await
        .map_err(|e| format!("connecting to hc: {e}"))?;
    if mode == "resume" {
        let typed = migrate::resume(&client).await?;
        if typed.is_empty() {
            println!("no panes are waiting to resume");
        }
        for (address, line) in typed {
            println!("{address}: {line}");
        }
        return Ok(());
    }
    let path = if let Some(path) = file {
        path
    } else {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        snapshot::latest(&snapshot::default_dir(&PathBuf::from(home)))?
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let workspaces = snapshot::read(&value)?;
    let state = migrate::state(&client).await?;
    println!("snapshot: {}", path.display());
    print_plan(&workspaces, &state);
    if mode != "apply" {
        println!("\nplan only: run `hcm apply` to do it");
        return Ok(());
    }
    let added = migrate::apply(&client, &workspaces, &state).await?;
    println!();
    if added.is_empty() {
        println!("nothing to add: every tab was moved before");
    }
    for a in added {
        let how = if a.fresh {
            "new space"
        } else {
            "existing space"
        };
        println!("added {} tab(s) to the {how} for {}", a.tabs, a.folder);
    }
    Ok(())
}
