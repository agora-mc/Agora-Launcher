"""Files a game opened from its base under usvfs vs under agvfs (agvfs run with `verbose=1`).

usage: python compare_opened.py <usvfs.log> <agvfs.log>
"""
import re
import sys

usvfs = set()
for line in open(sys.argv[1], encoding='utf-8', errors='replace'):
    m = re.search(r'NtCreateFile\[inPathW=([^\]]*)\]\[rerouter\.fileName\(\)=([^\]]*)\].*?\[res=0\]', line)
    if m and 'mount\\' in m.group(1):
        usvfs.add(m.group(1).split('mount\\', 1)[-1].lower().replace('\\', '/'))

agvfs = set()
for line in open(sys.argv[2], encoding='utf-8', errors='replace'):
    m = re.search(r'opened (.*?) -> ', line)
    if m:
        agvfs.add(m.group(1).lower().replace('\\', '/'))

print('usvfs opened', len(usvfs), '| agvfs opened', len(agvfs))
print('usvfs only:', sorted(usvfs - agvfs))
print('agvfs only:', sorted(agvfs - usvfs))
