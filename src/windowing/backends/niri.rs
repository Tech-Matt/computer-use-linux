use crate::command_runner;
use crate::terminal::enrich_terminal_windows;
use crate::windowing::registry::BackendProbe;
use crate::windowing::types::WindowInfo;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::process::Command as StdCommand;
use tokio::process::Command;

pub const NIRI_BACKEND: &str = "niri";

pub fn probe() -> BackendProbe {
    match niri_windows_command().output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let ok = matches!(
                serde_json::from_str::<serde_json::Value>(&stdout),
                Ok(serde_json::Value::Array(_))
            );
            BackendProbe {
                id: NIRI_BACKEND,
                ok,
                can_list_windows: ok,
                can_focus_apps: ok,
                can_focus_windows: ok,
                detail: if ok {
                    "niri msg --json windows returned a JSON array".to_string()
                } else {
                    "niri msg --json windows did not return a JSON array".to_string()
                },
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            BackendProbe {
                id: NIRI_BACKEND,
                ok: false,
                can_list_windows: false,
                can_focus_apps: false,
                can_focus_windows: false,
                detail: if stderr.is_empty() { stdout } else { stderr },
            }
        }
        Err(error) => BackendProbe {
            id: NIRI_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: error.to_string(),
        },
    }
}

pub async fn list_windows() -> Result<Vec<WindowInfo>> {
    let output =
        command_runner::output(niri_windows_command_async(), "run niri msg --json windows").await?;
    if !output.status.success() {
        bail!(
            "niri msg --json windows failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut windows = parse_niri_windows(&String::from_utf8_lossy(&output.stdout))?;
    enrich_terminal_windows(&mut windows);
    Ok(windows)
}

pub(crate) fn parse_niri_windows(json: &str) -> Result<Vec<WindowInfo>> {
    let niri_windows: Vec<NiriWindow> =
        serde_json::from_str(json).context("failed to parse niri msg --json windows output")?;
    let mut windows = niri_windows
        .into_iter()
        .map(|window| WindowInfo {
            window_id: window.id,
            title: window.title,
            app_id: window.app_id,
            wm_class: None,
            pid: window.pid.and_then(|pid| u32::try_from(pid).ok()),
            // Niri reports a window's tile position relative to its workspace,
            // not a desktop-global origin suitable for screenshot cropping.
            bounds: None,
            workspace: window.workspace_id.and_then(|id| i32::try_from(id).ok()),
            focused: window.is_focused,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: NIRI_BACKEND.to_string(),
            terminal: None,
        })
        .collect::<Vec<_>>();
    windows.sort_by_key(|window| window.window_id);
    Ok(windows)
}

pub async fn activate_window(window_id: u64) -> Result<()> {
    let output = command_runner::output(
        niri_action_command_async(window_id),
        &format!("run niri msg action focus-window --id {window_id}"),
    )
    .await?;
    if !output.status.success() {
        bail!(
            "niri msg action focus-window --id {window_id} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn niri_windows_command() -> StdCommand {
    let mut command = StdCommand::new("niri");
    command.args(["msg", "--json", "windows"]);
    command
}

fn niri_windows_command_async() -> Command {
    let mut command = Command::new("niri");
    command.args(["msg", "--json", "windows"]);
    command
}

fn niri_action_command_async(window_id: u64) -> Command {
    let mut command = Command::new("niri");
    command.args(["msg", "action", "focus-window", "--id"]);
    command.arg(window_id.to_string());
    command
}

#[derive(Debug, Deserialize)]
struct NiriWindow {
    id: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    pid: Option<i32>,
    #[serde(default)]
    workspace_id: Option<u64>,
    #[serde(default)]
    is_focused: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_niri_windows_and_sorts_by_id() {
        let windows = parse_niri_windows(
            r#"[
                {"id": 9, "title": "Editor", "app_id": "org.example.Editor", "pid": 123, "workspace_id": 2, "is_focused": true, "layout": {"tile_size": [640.0, 480.0]}},
                {"id": 3, "title": null, "app_id": null, "pid": -1, "workspace_id": 9223372036854775808, "is_focused": false}
            ]"#,
        )
        .unwrap();

        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].window_id, 3);
        assert_eq!(windows[0].pid, None);
        assert_eq!(windows[0].workspace, None);
        assert_eq!(windows[1].title.as_deref(), Some("Editor"));
        assert_eq!(windows[1].app_id.as_deref(), Some("org.example.Editor"));
        assert_eq!(windows[1].pid, Some(123));
        assert_eq!(windows[1].workspace, Some(2));
        assert!(windows[1].focused);
        assert!(windows[1].bounds.is_none());
        assert_eq!(windows[1].backend, NIRI_BACKEND);
    }
}
