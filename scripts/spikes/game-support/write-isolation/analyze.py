"""Summarise a usvfs debug log from `spike2 launch`: what the game opened, how, and where it resolved.

usage: python analyze.py <usvfs.log> <base dir> <overwrite dir>
"""
import os, re, sys
from collections import defaultdict

log, base, overwrite = sys.argv[1], os.path.normcase(sys.argv[2]), os.path.normcase(sys.argv[3])

WRITE_BITS = 0x2 | 0x4 | 0x40000000 | 0x10000000  # WRITE_DATA, APPEND_DATA, GENERIC_WRITE, GENERIC_ALL
DELETE = 0x10000
DISPOSITIONS = {0: "supersede", 1: "open", 2: "create", 3: "open_if", 4: "overwrite", 5: "overwrite_if"}

create_re = re.compile(
    r"NtCreateFile\[inPathW=([^\]]*)\]\[rerouter\.fileName\(\)=([^\]]*)\]\[DesiredAccess=([0-9a-fA-F]+)\]"
    r"\[originalDisposition=(\d+)\]\[CreateDisposition=(\d+)\].*?\[res=([0-9a-fA-F]+)\]"
)
other_re = re.compile(r"^(hook_\w+)\[(.*)$")

opens = defaultdict(lambda: {"read": 0, "write": 0, "delete": 0, "dispositions": set(), "fail": 0, "real": ""})
others = defaultdict(list)
for line in open(log, encoding="utf-8", errors="replace"):
    m = create_re.search(line)
    if m:
        virt, real, access, _, disp, res = m.groups()
        access, disp = int(access, 16), int(disp)
        o = opens[virt.replace("\\??\\", "").lower()]
        o["real"] = real
        if int(res, 16) != 0:
            o["fail"] += 1
            continue
        if access & WRITE_BITS or disp in (0, 4, 5):
            o["write"] += 1
        else:
            o["read"] += 1
        if access & DELETE:
            o["delete"] += 1
        o["dispositions"].add(DISPOSITIONS.get(disp, str(disp)))
        continue
    m = other_re.match(line.strip())
    if m and m.group(1) not in ("hook_GetCurrentDirectoryW", "hook_GetFileAttributesW", "hook_GetFileAttributesExW",
                                "hook_GetFullPathNameW", "hook_GetModuleFileNameW", "hook_FindFirstFileExW"):
        others[m.group(1)].append(m.group(2)[:220])


def layer(real):
    r = os.path.normcase(real)
    if r.startswith(overwrite):
        return "overwrite"
    if r.startswith(base):
        return "base"
    return "outside"


rows = [(v, o, layer(o["real"])) for v, o in opens.items() if o["real"]]
print(f"files opened through the mount: {len(rows)}")
for name in ("base", "overwrite", "outside"):
    print(f"  resolved to {name}: {sum(1 for r in rows if r[2] == name)}")

print("\nWRITE-access opens of existing lower (base) files -- what copy-on-open would copy:")
total = 0
for v, o, l in sorted(rows):
    if l == "base" and o["write"]:
        size = os.path.getsize(o["real"]) if os.path.exists(o["real"]) else 0
        total += size
        print(f"  {size/1e6:10.2f} MB  w{o['write']} r{o['read']} del{o['delete']} {sorted(o['dispositions'])}  {v}")
print(f"  total {total/1e6:.2f} MB")

print("\nOpens that resolved to overwrite (new files the game made):")
for v, o, l in sorted(rows):
    if l == "overwrite":
        print(f"  w{o['write']} r{o['read']} {sorted(o['dispositions'])}  {v}")

print("\nDELETE-access opens:")
for v, o, l in sorted(rows):
    if o["delete"]:
        print(f"  [{l}] del{o['delete']}  {v}")

print("\nOther mutating hooks:")
for name, calls in sorted(others.items()):
    print(f"  {name}: {len(calls)}")
    for c in calls[:8]:
        print(f"    {c}")
