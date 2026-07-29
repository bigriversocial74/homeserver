from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"Missing repair anchor in {path}: {old[:120]!r}")
    if text.count(old) != 1:
        raise SystemExit(f"Expected one repair anchor in {path}, found {text.count(old)}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


Path("src-tauri/src/main.rs").write_text(
    '''#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    microgifter_homeserver_control_center_lib::run();
}
''',
    encoding="utf-8",
)

replace_once(
    "src-tauri/Cargo.toml",
    'tauri = { version = "2", features = [] }\n',
    'tauri = { version = "2", features = ["tray-icon"] }\ntauri-plugin-autostart = "2"\n',
)

lib = Path("src-tauri/src/lib.rs")
text = lib.read_text(encoding="utf-8")
text = text.replace(
    "use std::time::Duration;\n",
    '''use std::time::Duration;
#[cfg(desktop)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
''',
    1,
)
marker = 'const PASSPHRASE_HEADER: &str = "x-mg-recovery-passphrase";\n'
desktop_code = r'''

#[cfg(desktop)]
struct DesktopUiState {
    quitting: AtomicBool,
}

#[cfg(desktop)]
fn show_control_center(app: &tauri::AppHandle, route: Option<&str>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        if let Some(route) = route {
            let script = match route {
                "dashboard" => "window.location.hash = '#dashboard';",
                "agent" => "window.location.hash = '#agent';",
                "system" => "window.location.hash = '#system';",
                _ => "",
            };
            if !script.is_empty() {
                let _ = window.eval(script);
            }
        }
    }
}

#[cfg(desktop)]
fn run_tray_action(app: &tauri::AppHandle, action: &str) {
    match action {
        "open-dashboard" => show_control_center(app, Some("dashboard")),
        "open-agent" => show_control_center(app, Some("agent")),
        "open-status" => show_control_center(app, Some("system")),
        "check-updates" => {
            show_control_center(app, Some("system"));
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval(
                    "window.dispatchEvent(new CustomEvent('homeserver-tray-action', { detail: { action: 'check-updates' } }));",
                );
            }
        }
        "quit-control-center" => {
            app.state::<DesktopUiState>()
                .quitting
                .store(true, Ordering::SeqCst);
            app.exit(0);
        }
        _ => {}
    }
}

#[tauri::command]
fn control_center_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        return app.autolaunch().is_enabled().map_err(|error| error.to_string());
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Ok(false)
    }
}

#[tauri::command]
fn control_center_set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        if enabled {
            manager.enable().map_err(|error| error.to_string())?;
        } else {
            manager.disable().map_err(|error| error.to_string())?;
        }
        return manager.is_enabled().map_err(|error| error.to_string());
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, enabled);
        Ok(false)
    }
}
'''
if marker not in text:
    raise SystemExit("Missing lib.rs desktop insertion anchor")
text = text.replace(marker, marker + desktop_code, 1)

builder_anchor = '''    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
'''
builder_replacement = '''    tauri::Builder::default()
        .setup(|app| {
            #[cfg(desktop)]
            {
                use tauri_plugin_autostart::MacosLauncher;

                app.handle().plugin(tauri_plugin_autostart::init(
                    MacosLauncher::LaunchAgent,
                    Some(vec!["--hidden"]),
                ))?;
                app.manage(DesktopUiState {
                    quitting: AtomicBool::new(false),
                });

                let dashboard = MenuItem::with_id(app, "open-dashboard", "Open Dashboard", true, None::<&str>)?;
                let agent = MenuItem::with_id(app, "open-agent", "Open Agent Chat", true, None::<&str>)?;
                let status = MenuItem::with_id(app, "open-status", "HomeServer Status", true, None::<&str>)?;
                let updates = MenuItem::with_id(app, "check-updates", "Check for Updates", true, None::<&str>)?;
                let separator = PredefinedMenuItem::separator(app)?;
                let quit = MenuItem::with_id(app, "quit-control-center", "Quit Control Center", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&dashboard, &agent, &status, &updates, &separator, &quit])?;
                let tray_icon = app
                    .default_window_icon()
                    .cloned()
                    .ok_or("HomeServer application icon is unavailable")?;

                TrayIconBuilder::with_id("homeserver-control-center")
                    .icon(tray_icon)
                    .tooltip("Microgifter HomeServer")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| run_tray_action(app, event.id.as_ref()))
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            show_control_center(tray.app_handle(), None);
                        }
                    })
                    .build(app)?;

                if std::env::args().any(|argument| argument == "--hidden") {
                    if let Some(window) = app.get_webview_window("main") {
                        window.hide()?;
                    }
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            if let WindowEvent::CloseRequested { api, .. } = event {
                let quitting = window
                    .app_handle()
                    .state::<DesktopUiState>()
                    .quitting
                    .load(Ordering::SeqCst);
                if !quitting {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
'''
if builder_anchor not in text:
    raise SystemExit("Missing Tauri builder anchor")
text = text.replace(builder_anchor, builder_replacement, 1)

command_anchor = "            homeserver_status,\n"
command_replacement = '''            homeserver_status,
            control_center_autostart_enabled,
            control_center_set_autostart,
'''
if command_anchor not in text:
    raise SystemExit("Missing command registration anchor")
text = text.replace(command_anchor, command_replacement, 1)
lib.write_text(text, encoding="utf-8")

main = Path("src/main.js")
text = main.read_text(encoding="utf-8")
text = text.replace(
    "let notificationMenuOpen = false;\n",
    "let notificationMenuOpen = false;\nlet desktopAutostartEnabled = false;\n",
    1,
)
general = '''      ${settingsSection("dashboard", "General", "Basic server display preferences.", `<label><span>Server Name</span><input id="setting-server-name" type="text" value="${escapeHtml(prefs.serverName)}" maxlength="64"></label><label><span>Time Zone</span><select id="setting-time-zone"><option value="local" ${prefs.timeZone === "local" ? "selected" : ""}>Use Windows local time</option><option value="utc" ${prefs.timeZone === "utc" ? "selected" : ""}>UTC</option></select></label><button class="button primary" data-save-setting="general">Save</button>`)}
'''
desktop = general + '''      ${settingsSection("system", "Windows Desktop", "Keep the Control Center available without leaving a terminal or taskbar window open.", `<label class="toggle-field"><span>Start Control Center with Windows</span>${toggle("setting-start-with-windows", desktopAutostartEnabled)}</label><p class="settings-note">Closing the window hides the Control Center to the Windows system tray. The HomeServer service continues running independently.</p><button class="button primary" data-save-setting="desktop">Save</button>`)}
'''
if general not in text:
    raise SystemExit("Missing settings General anchor")
text = text.replace(general, desktop, 1)
summary_old = '<div><dt>Notifications</dt><dd>${prefs.notifications ? "Enabled" : "Disabled"}</dd></div><div><dt>Backups</dt>'
summary_new = '<div><dt>Notifications</dt><dd>${prefs.notifications ? "Enabled" : "Disabled"}</dd></div><div><dt>Start with Windows</dt><dd>${desktopAutostartEnabled ? "Enabled" : "Disabled"}</dd></div><div><dt>Close Button</dt><dd>Hide to tray</dd></div><div><dt>Backups</dt>'
if summary_old not in text:
    raise SystemExit("Missing settings summary anchor")
text = text.replace(summary_old, summary_new, 1)
text = text.replace("function savePreferences() {\n", "async function savePreferences() {\n", 1)
save_anchor = '''  prefs.autoRefresh = Boolean(document.querySelector("#setting-auto-refresh")?.checked);
  localStorage.setItem("homeserver-ui-preferences", JSON.stringify(prefs));
  notice = { kind: "success", message: "Control Center preferences saved locally." };
  render();
'''
save_replacement = '''  prefs.autoRefresh = Boolean(document.querySelector("#setting-auto-refresh")?.checked);
  const requestedAutostart = Boolean(document.querySelector("#setting-start-with-windows")?.checked);
  if (requestedAutostart !== desktopAutostartEnabled) {
    try {
      desktopAutostartEnabled = Boolean(await invoke("control_center_set_autostart", { enabled: requestedAutostart }));
    } catch (error) {
      notice = { kind: "warning", message: `Unable to update Windows startup: ${String(error)}` };
      render();
      return;
    }
  }
  localStorage.setItem("homeserver-ui-preferences", JSON.stringify(prefs));
  notice = { kind: "success", message: "Control Center and Windows desktop preferences saved." };
  render();
'''
if save_anchor not in text:
    raise SystemExit("Missing savePreferences anchor")
text = text.replace(save_anchor, save_replacement, 1)
promise_anchor = '''    invoke("homeserver_mcp_bridge_path"),
  ]);
  if (results[0].status === "rejected") {
'''
promise_replacement = '''    invoke("homeserver_mcp_bridge_path"),
    invoke("control_center_autostart_enabled"),
  ]);
  if (results[9].status === "fulfilled") desktopAutostartEnabled = Boolean(results[9].value);
  if (results[0].status === "rejected") {
'''
if promise_anchor not in text:
    raise SystemExit("Missing loadAll promise anchor")
text = text.replace(promise_anchor, promise_replacement, 1)
tray_listener_anchor = 'document.addEventListener("click", (event) => {\n'
tray_listener = '''window.addEventListener("homeserver-tray-action", (event) => {
  if (event.detail?.action !== "check-updates" || busy) return;
  navigate("system");
  void checkUpdates();
});

''' + tray_listener_anchor
if tray_listener_anchor not in text:
    raise SystemExit("Missing tray listener anchor")
text = text.replace(tray_listener_anchor, tray_listener, 1)
main.write_text(text, encoding="utf-8")

styles = Path("src/styles.css")
css = styles.read_text(encoding="utf-8")
css += '''

.settings-note {
  margin: 0;
  color: var(--text-muted, #687386);
  font-size: 0.82rem;
  line-height: 1.5;
}
'''
styles.write_text(css, encoding="utf-8")

validator = r'''from pathlib import Path

main_rs = Path("src-tauri/src/main.rs").read_text(encoding="utf-8")
lib_rs = Path("src-tauri/src/lib.rs").read_text(encoding="utf-8")
cargo = Path("src-tauri/Cargo.toml").read_text(encoding="utf-8")
frontend = Path("src/main.js").read_text(encoding="utf-8")

checks = {
    "production GUI subsystem": 'windows_subsystem = "windows"' in main_rs,
    "tray feature": 'features = ["tray-icon"]' in cargo,
    "autostart plugin": 'tauri-plugin-autostart = "2"' in cargo,
    "native tray builder": 'TrayIconBuilder::with_id("homeserver-control-center")' in lib_rs,
    "dashboard tray action": '"open-dashboard"' in lib_rs,
    "agent tray action": '"open-agent"' in lib_rs,
    "status tray action": '"open-status"' in lib_rs,
    "update tray action": '"check-updates"' in lib_rs,
    "explicit UI quit": '"quit-control-center"' in lib_rs,
    "close request intercepted": 'WindowEvent::CloseRequested' in lib_rs and 'api.prevent_close();' in lib_rs,
    "close hides window": 'let _ = window.hide();' in lib_rs,
    "autostart launches hidden": 'Some(vec!["--hidden"])' in lib_rs,
    "startup command": 'control_center_set_autostart' in lib_rs and 'control_center_autostart_enabled' in lib_rs,
    "settings toggle": 'setting-start-with-windows' in frontend,
    "tray update listener": 'homeserver-tray-action' in frontend,
    "service isolation": 'stop' not in lib_rs[lib_rs.find('"quit-control-center"'):lib_rs.find('"quit-control-center"') + 400].lower(),
}

failed = [name for name, passed in checks.items() if not passed]
if failed:
    raise SystemExit("Windows desktop validation failed: " + ", ".join(failed))
print("Windows tray, close-to-tray, autostart, and GUI-subsystem validation passed.")
'''
Path("scripts/validate-windows-desktop.py").write_text(validator, encoding="utf-8")

package = Path("package.json")
package_text = package.read_text(encoding="utf-8")
package_text = package_text.replace(
    ' && python scripts/validate-multi-cloud-connections.py"',
    ' && python scripts/validate-multi-cloud-connections.py && python scripts/validate-windows-desktop.py"',
    1,
)
package.write_text(package_text, encoding="utf-8")

Path("docs/windows-tray-background-ui.md").write_text(
    '''# Windows Tray and Background Control Center

## Behavior

- Production Control Center launches without a terminal window.
- A Microgifter HomeServer icon remains in the Windows system tray while the Control Center UI is running.
- Left-clicking the tray icon restores and focuses the current Control Center page.
- The tray menu opens Dashboard, Agent Chat, HomeServer Status, checks for signed updates, or explicitly exits the Control Center UI.
- Closing the main window hides it to the system tray instead of terminating the UI process.
- The LocalSystem HomeServer service continues independently when the Control Center is hidden or explicitly exited.
- Settings includes an optional **Start Control Center with Windows** toggle.
- Windows autostart launches the Control Center hidden so the tray icon is available without opening the dashboard.

No HomeServer service, pairing, provider, model, backup, MCP, or local-data behavior is changed.
''',
    encoding="utf-8",
)
