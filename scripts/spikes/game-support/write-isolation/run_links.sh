#!/bin/bash
# Score link deployment on the write matrix, plain and with ACL-denied lower files. The ACL leaves
# WriteAttributes open, because creating a hardlink needs it.
# Build first: (cd harness && cargo build --release)
cd "$(dirname "$0")"
work="${SPIKE2_WORK:-$(cygpath -u "$LOCALAPPDATA")/AgoraSpike/write-isolation}"
mkdir -p "$work"
export SPIKE2_FILE_DENY='WriteData,AppendData,WriteExtendedAttributes,Delete'
for v in "" "--acl"; do
  sb="$(cygpath -w "$work/sb-links${v}")"
  timeout 120 ./harness/target/release/spike2.exe links --sandbox "$sb" $v 2>&1 | grep -E "listing:|attributes:|probe exit"
  python - "$sb" "${v:-plain}" <<'PY'
import json, os, sys
from collections import Counter
s = json.load(open(os.path.join(sys.argv[1], 'results', 'summary.json')))
rows = s['rows']
leak = lambda r: r['lower'] not in ('orig', '-') or r['store'] not in ('orig', '-')
print(f"  {sys.argv[2]}: ops {len(rows)}, leaks {sum(map(leak, rows))}, game-visible failures {sum(1 for r in rows if not r['ok'])}")
by = Counter((r['layer'], 'LEAK' if leak(r) else ('ok' if r['ok'] else 'refused')) for r in rows if r['who'] == 'direct')
for k in sorted(by):
    print('   ', k, by[k])
PY
done
