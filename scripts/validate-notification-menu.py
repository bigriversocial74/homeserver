from pathlib import Path

root = Path(__file__).resolve().parents[1]
main = (root / "src" / "main.js").read_text(encoding="utf-8")
css = (root / "src" / "styles.css").read_text(encoding="utf-8")

required_main = [
    'let notificationMenuOpen = false;',
    'function notificationItems()',
    'function renderNotificationMenu()',
    'id="notification-toggle"',
    'id="notification-dropdown"',
    'data-notification-page=',
    'event.target.closest(".notification-center")',
    'event.key !== "Escape"',
]
required_css = [
    '.notification-center{position:relative',
    '.notification-dropdown{position:absolute',
    '.notification-item{display:grid',
]
for marker in required_main:
    if marker not in main:
        raise SystemExit(f"Missing notification menu contract: {marker}")
for marker in required_css:
    if marker not in css:
        raise SystemExit(f"Missing notification menu style contract: {marker}")
if 'aria-label="Notifications">${icon("bell", 19)}<span class="notification-count">3</span>' in main:
    raise SystemExit("Decorative dead notification button remains")
print("Header notifications open a bounded, keyboard-accessible dropdown menu.")
