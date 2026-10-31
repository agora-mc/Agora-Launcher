#!/bin/bash
# Run the write matrix (plain and ACL-protected lowers) under agvfs; print leaks and the consistency checks.
# Build first: (cd harness && cargo build --release) && (cd agvfs && cargo build --release)
cd "$(dirname "$0")"
work="${SPIKE2_WORK:-$(cygpath -u "$LOCALAPPDATA")/AgoraSpike/write-isolation}"
mkdir -p "$work"
dll="$(cygpath -w "$PWD/agvfs/target/release/agvfs.dll")"
for v in "" "--acl"; do
  sb="$(cygpath -w "$work/sb-agvfs${v}")"
  timeout 120 ./harness/target/release/spike2.exe agvfs --dll "$dll" --sandbox "$sb" $v 2>&1 | grep -E "listing:|attributes:|probe exit"
  python - "$sb" "${v:-plain}" <<'PY'
import json, os, sys
s = json.load(open(os.path.join(sys.argv[1], 'results', 'summary.json')))
leaks = [r for r in s['rows'] if r['lower'] not in ('orig', '-') or r['store'] not in ('orig', '-')]
print(f"  {sys.argv[2]}: ops {len(s['rows'])}, leaks {len(leaks)}, game-visible failures {sum(1 for r in s['rows'] if not r['ok'])}")
PY
done
