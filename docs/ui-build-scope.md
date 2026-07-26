# HomeServer Control Center UI build

This scoped branch rebuilds the HomeServer Control Center and adds its product landing page from the approved visual mockups.

The implementation preserves the existing Tauri commands and service boundaries for status, pairing, synchronization, backups, recovery, and signed updates. The page shell, navigation, cards, icon system, responsive layouts, and landing-page presentation are replaced without modifying the Rust service or installer engine.

## Delivered screens

- Dashboard
- Home
- Apps
- Backups
- Integrations & Agents
- Knowledge Vault
- Settings
- Sync Cloud
- System
- Responsive HomeServer product landing page

## Validation

The complete frontend payload was verified by SHA-256 before extraction. The scoped installer then passed:

- `npm ci`
- `npm run check:frontend`
- `npm run build`
- `git diff --cached --check`

Local runtime harnesses also rendered all nine Control Center pages in paired, unpaired, degraded, and offline states, and rendered the standalone landing page.

The temporary payload files and installer workflow removed themselves before the final feature commit. No Rust service, installer, database, pairing protocol, backup engine, or update engine files were changed by this UI build.
