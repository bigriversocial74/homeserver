#!/usr/bin/env python3
import json
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
source_path = root / "crates/homeserver-service/src/inference_governance.rs"
source = source_path.read_text(encoding="utf-8")
old = "connection.unchecked_transaction_with_behavior(TransactionBehavior::Immediate)?"
new = "Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?"
if source.count(old) != 2:
    raise SystemExit(f"expected two immediate transaction compatibility anchors, found {source.count(old)}")
source_path.write_text(source.replace(old, new), encoding="utf-8")

package_path = root / "package.json"
package = json.loads(package_path.read_text(encoding="utf-8"))
if package.get("scripts", {}).get("preinstall") != "python scripts/phase20-final-fix.py":
    raise SystemExit("temporary Phase 20 preinstall hook is missing")
package["scripts"].pop("preinstall")
package_path.write_text(json.dumps(package, indent=2) + "\n", encoding="utf-8")

subprocess.run(["cargo", "fmt", "--all"], cwd=root, check=True)
Path(__file__).unlink()
print("Phase 20 rusqlite immediate transaction compatibility repair applied and removed.")
