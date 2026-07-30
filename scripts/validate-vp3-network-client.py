from pathlib import Path

root = Path(__file__).resolve().parents[1]
client = (root / 'crates/homeserver-service/src/vp3_client.rs').read_text()
authority = (root / 'crates/homeserver-service/src/software_authority.rs').read_text()
migration = (root / 'database/migrations/0018_vp3_network_client.sql').read_text()
update = (root / 'crates/homeserver-service/src/update.rs').read_text()

required = [
    'Policy::none()',
    'host == "vp3.me" || host.ends_with(".vp3.me")',
    'ACTIVATE VP3',
    'verify_authority_document',
    'VP3_HOMESERVER_LEASE_PUBLIC_KEY_B64',
    'installer_download_path',
    'bearer_auth',
    'verify_authenticode',
    'vp3_authority_outbox',
    'vp3_release_authorizations',
    'Operating-system credential vault',
]
for marker in required[:-1]:
    if marker not in client and marker not in migration and marker not in update:
        raise SystemExit(f'missing VP3 client boundary: {marker}')
if 'credential TEXT' in migration or 'grant_token TEXT' in migration:
    raise SystemExit('plaintext VP3 credentials are forbidden in SQLite')
for route in ['fingerprint', 'activate', 'refresh', 'heartbeat']:
    if f'/v1/software-authority/{route}' not in authority:
        raise SystemExit(f'missing local VP3 authority route: {route}')
print('VP3 network client contract passed.')
