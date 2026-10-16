#!/usr/bin/env bash
# Unwrap sealed connection records with the operator's login on the escrow vault.
# The server identity has no role there. See docs/server/privacy.md.
set -euo pipefail

usage() {
  cat <<'EOF'
usage: audit_unwrap.sh --vault-name ESCROW --key-name KEY [--in rows.jsonl] [--out keys.jsonl]

Read JSON lines from `innkeeperd audit export` (or stdin) and unwrap each data key:

  az keyvault key decrypt --algorithm RSA-OAEP-256 \
    --vault-name ESCROW --name KEY [--version VERSION] --value WRAPPED

VERSION is the last path segment of `kid` when `kid` is an escrow key version URL.
Each output line is {"id","data_key_b64","wrapped_key_b64"} for
`innkeeperd audit open --data-keys`. The az JSON may call the key "result" or "plaintext".
EOF
}

vault=""
key=""
infile=""
outfile=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --vault-name) vault=${2:?}; shift 2 ;;
    --key-name) key=${2:?}; shift 2 ;;
    --in) infile=${2:?}; shift 2 ;;
    --out) outfile=${2:?}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ -z "$vault" || -z "$key" ]]; then
  echo "pass --vault-name and --key-name" >&2
  usage >&2
  exit 2
fi

export AUDIT_VAULT="$vault" AUDIT_KEY="$key"
python3 - "$infile" "$outfile" <<'PY'
import json
import os
import subprocess
import sys

infile, outfile = sys.argv[1], sys.argv[2]
vault = os.environ["AUDIT_VAULT"]
name = os.environ["AUDIT_KEY"]
source = open(infile, encoding="utf-8") if infile else sys.stdin
dest = open(outfile, "w", encoding="utf-8") if outfile else sys.stdout


def version_of(kid):
    if not isinstance(kid, str) or not kid.startswith("https://"):
        return None
    path = kid.split("://", 1)[1].split("?", 1)[0].rstrip("/")
    parts = path.split("/")
    if "keys" not in parts:
        return None
    tail = parts[parts.index("keys") + 1 :]
    if len(tail) >= 2 and tail[-1]:
        return tail[-1]
    return None


try:
    for raw in source:
        line = raw.strip()
        if not line:
            continue
        row = json.loads(line)
        wrapped = row["wrapped_key_b64"]
        cmd = [
            "az",
            "keyvault",
            "key",
            "decrypt",
            "--algorithm",
            "RSA-OAEP-256",
            "--vault-name",
            vault,
            "--name",
            name,
            "--value",
            wrapped,
            "-o",
            "json",
        ]
        version = version_of(row.get("kid"))
        if version:
            cmd.extend(["--version", version])
        payload = json.loads(subprocess.check_output(cmd, text=True))
        data = payload.get("result") or payload.get("plaintext")
        if not data:
            sys.stderr.write("az decrypt returned neither result nor plaintext\n")
            sys.exit(1)
        dest.write(
            json.dumps(
                {
                    "id": row.get("id"),
                    "data_key_b64": data,
                    "wrapped_key_b64": wrapped,
                }
            )
            + "\n"
        )
        dest.flush()
finally:
    if infile:
        source.close()
    if outfile:
        dest.close()
PY
