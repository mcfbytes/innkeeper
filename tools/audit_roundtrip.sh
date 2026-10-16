#!/usr/bin/env bash
# Show that the public key in the server vault and the RSA key in the escrow vault match.
# Run as the operator. Needs az and innkeeperd (override with INNKEEPERD). See docs/server/privacy.md.
set -euo pipefail

usage() {
  cat <<'EOF'
usage: audit_roundtrip.sh --server-vault NAME --escrow-vault NAME --key-name KEY
                          [--public-secret audit-public-key] [--kid-secret audit-key-id]

Read the audit public key and its key version URL from the server vault, seal a probe with
`innkeeperd audit seal`, unwrap the data key in the escrow vault (tools/audit_unwrap.sh),
open the row with `innkeeperd audit open --data-keys`, and compare the plaintext to the probe.
EOF
}

server=""
escrow=""
key=""
public_secret="audit-public-key"
kid_secret="audit-key-id"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --server-vault) server=${2:?}; shift 2 ;;
    --escrow-vault) escrow=${2:?}; shift 2 ;;
    --key-name) key=${2:?}; shift 2 ;;
    --public-secret) public_secret=${2:?}; shift 2 ;;
    --kid-secret) kid_secret=${2:?}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ -z "$server" || -z "$escrow" || -z "$key" ]]; then
  echo "pass --server-vault, --escrow-vault and --key-name" >&2
  usage >&2
  exit 2
fi

inn="${INNKEEPERD:-innkeeperd}"
if ! command -v az >/dev/null 2>&1; then
  echo "az is not on PATH" >&2
  exit 1
fi
if ! command -v "$inn" >/dev/null 2>&1 && [[ ! -x "$inn" ]]; then
  echo "innkeeperd not found; set INNKEEPERD" >&2
  exit 1
fi

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

az keyvault secret show \
  --vault-name "$server" \
  --name "$public_secret" \
  --query value \
  -o tsv >"$work/audit-public.pem"
kid=$(az keyvault secret show \
  --vault-name "$server" \
  --name "$kid_secret" \
  --query value \
  -o tsv)
kid=${kid//$'\r'/}

record='{"probe":"audit-roundtrip"}'
"$inn" audit seal \
  --public-key "$work/audit-public.pem" \
  --key-id "$kid" \
  --record "$record" >"$work/row.jsonl"
"$here/audit_unwrap.sh" \
  --vault-name "$escrow" \
  --key-name "$key" \
  --in "$work/row.jsonl" \
  --out "$work/keys.jsonl"
opened=$("$inn" audit open --in "$work/row.jsonl" --data-keys "$work/keys.jsonl")
if [[ "$opened" != "$record" ]]; then
  echo "round trip failed: the opened record does not match the probe" >&2
  exit 1
fi
echo "round trip ok: the escrow vault unwrapped a record sealed to the server vault public key"
