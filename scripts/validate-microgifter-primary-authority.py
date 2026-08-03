#!/usr/bin/env python3
"""Permanent contract validation for Microgifter primary HomeServer authority."""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def require(path: str, needles: list[str]) -> None:
    content = (ROOT / path).read_text(encoding="utf-8")
    for needle in needles:
        if needle not in content:
            raise SystemExit(f"{path}: missing required contract text: {needle}")


def forbid(path: str, needles: list[str]) -> None:
    content = (ROOT / path).read_text(encoding="utf-8")
    for needle in needles:
        if needle in content:
            raise SystemExit(f"{path}: forbidden contract text remains: {needle}")


def main() -> None:
    require(
        "database/migrations/0033_microgifter_primary_software_authority.sql",
        [
            "provider_key TEXT NOT NULL DEFAULT 'microgifter'",
            "No HomeServer identity, credential, grant or local data is replaced",
            "0033_microgifter_primary_software_authority",
        ],
    )
    require(
        "crates/homeserver-service/src/config.rs",
        [
            'https://microgifter.com/api/homeserver/update-manifest-stable.php',
            'DEFAULT_VP3_BASE_URL',
        ],
    )
    require(
        "crates/homeserver-service/src/software_authority.rs",
        [
            'const MICROGIFTER_AUTHORITY: &str = "microgifter";',
            "microgifter_connection::ensure_update_download_allowed(connection, update_id)",
            "microgifter_connection::record_update_result_receipt(",
            "microgifter_primary_active",
            "vp3_optional",
        ],
    )
    forbid(
        "crates/homeserver-service/src/software_authority.rs",
        [
            'match snapshot.current_authority.as_str()',
            '"VP3 software authority is not active"',
            '"VP3 license does not permit this update"',
            '"pending_vp3_submission"',
        ],
    )
    require(
        "crates/homeserver-service/src/microgifter_connection.rs",
        [
            'const UPDATE_AUTHORIZATION_PATH: &str = "/api/homeserver/v1/updates/authorize";',
            'const UPDATE_RECEIPT_PATH: &str = "/api/homeserver/v1/updates/receipts";',
            '"signed-updates.v1"',
            '"update-authorization.v1"',
            '"update-receipts.v1"',
            "pub(crate) fn ensure_update_download_allowed(",
            "pub(crate) fn record_update_result_receipt(",
        ],
    )
    print("Microgifter primary HomeServer authority contract passed.")


if __name__ == "__main__":
    main()
