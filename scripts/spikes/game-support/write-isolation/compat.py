"""Run a few installed games under agvfs with their store install as the read-only lower layer.

For each game: snapshot the install (relative path, size, mtime), launch it under agvfs, stop it
after SECONDS, then re-snapshot and report whether anything in the install changed.

usage: python compat.py <name> [<name> ...]   (names from GAMES below)
"""
import json, os, subprocess, sys, time
from pathlib import Path

SPIKE = Path(__file__).resolve().parent
WORK = Path(os.environ.get("SPIKE2_WORK") or os.path.expandvars(r"%LOCALAPPDATA%\AgoraSpike\write-isolation"))
SECONDS = 45
GAMES = {
    # name: (install folder, exe relative to it, Steam app id)
    "rimworld": (r"C:\Program Files (x86)\Steam\steamapps\common\RimWorld", "RimWorldWin64.exe", "294100"),
    "slay-the-spire": (r"C:\Program Files (x86)\Steam\steamapps\common\SlayTheSpire", "SlayTheSpire.exe", "646570"),
    "balatro": (r"C:\Program Files (x86)\Steam\steamapps\common\Balatro", "Balatro.exe", "2379780"),
    "satisfactory": (r"C:\Program Files (x86)\Steam\steamapps\common\Satisfactory", "FactoryGameSteam.exe", "526870"),
}


def snapshot(root):
    out = {}
    for dirpath, _, files in os.walk(root):
        for f in files:
            p = os.path.join(dirpath, f)
            try:
                st = os.stat(p)
                out[os.path.relpath(p, root)] = (st.st_size, st.st_mtime_ns)
            except OSError:
                pass
    return out


def stop_processes_under(*folders):
    paths = ",".join(f"'{f}'" for f in folders)
    script = (
        f"$roots = @({paths}); Get-Process | Where-Object {{ $p = $_.Path; $p -and ($roots | Where-Object {{ $p.StartsWith($_, 'OrdinalIgnoreCase') }}) }}"
        " | ForEach-Object { $_.ProcessName + ' ' + [int]($_.WorkingSet64/1MB) + 'MB'; Stop-Process -Id $_.Id -Force -Confirm:$false }"
    )
    return subprocess.run(["powershell", "-NoProfile", "-Command", script], capture_output=True, text=True).stdout.strip()


def run(name):
    install, exe, appid = GAMES[name]
    work = WORK / "compat" / name
    if work.exists():
        subprocess.run(["cmd", "/c", "rmdir", "/s", "/q", str(work)])
    work.mkdir(parents=True)
    before = snapshot(install)
    env = dict(os.environ, SteamAppId=appid, SteamGameId=appid, SPIKE2_COPY_BINARIES="1")
    proc = subprocess.Popen(
        [str(SPIKE / "harness/target/release/spike2.exe"), "launch-agvfs",
         "--dll", str(SPIKE / "agvfs/target/release/agvfs.dll"), "--base", install,
         "--mount", str(work / "mount"), "--overwrite", str(work / "overwrite"),
         "--exe", exe, "--log", str(work / "agvfs.log")],
        env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    deadline = time.time() + SECONDS
    while time.time() < deadline and proc.poll() is None:
        time.sleep(1)
    running = stop_processes_under(str(work / "mount"), install) if proc.poll() is None else ""
    out = proc.communicate(timeout=60)[0]
    after = snapshot(install)
    changed = sorted(k for k in before.keys() & after.keys() if before[k] != after[k])
    log = (work / "agvfs.log").read_text(encoding="utf-8", errors="replace") if (work / "agvfs.log").exists() else ""
    result = {
        "game": name,
        "still_running_at_deadline": running or None,
        "harness_output": out.strip().splitlines()[-3:],
        "install_files": len(before),
        "install_changed": changed,
        "install_added": sorted(after.keys() - before.keys()),
        "install_removed": sorted(before.keys() - after.keys()),
        "files_opened": log.count(" opened "),
        "unmerged_listings": sorted({l.split("listing class ")[1].split(" not")[0] for l in log.splitlines() if "listing class" in l}),
        "copy_ups": log.count("copy-up "),
        "whiteouts": log.count("whiteout "),
        "upper_files": [str(p.relative_to(work / "overwrite")) for p in (work / "overwrite").rglob("*") if p.is_file()][:20],
    }
    (work / "result.json").write_text(json.dumps(result, indent=1))
    print(json.dumps(result, indent=1))


for n in sys.argv[1:]:
    run(n)
