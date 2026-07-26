# HomeServer Control Center UI build

This scoped branch rebuilds the HomeServer Control Center and adds its product landing page from the approved visual mockups.

The implementation preserves the existing Tauri commands and service boundaries for status, pairing, synchronization, backups, recovery, and signed updates. The page shell, navigation, cards, icon system, responsive layouts, and landing-page presentation are replaced without modifying the Rust service or installer engine.

The temporary payload workflow validates and installs the complete frontend file set, then removes itself before the final branch commit.
