from pathlib import Path

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
