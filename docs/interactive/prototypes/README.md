# High Interaction — design prototypes

Standalone HTML studies for the rebuilt High Interaction mode. Each is self-contained (no build,
no network) — open directly in a browser. They are **design artifacts, not production code**; the
production surface is React under `desktop/src/features/interactive/`.

Context: TERRA-6b found the shipped High Interaction surface was a two-column card list with no
icons, no real diagram, and false mass warnings — strictly less usable than the Standard view it
was meant to make friendlier. These prototypes explore the replacement.

## Versions

| File | What it establishes |
|---|---|
| `v1-baseline.html` | The structural reset: Play as hero, inventory shelf instead of a card list, bounded neighbourhood diagram (real SVG curves), everything advanced behind a closed drawer. |
| `v2-juiced.html` | Adds the animated pre-flight health check (the centrepiece), living world thumbnail, 3D tilt tiles, particles, optional synthesized sound. |
| `v3-maximalist.html` | **Current checkpoint.** Full-page living terrain background, real pack logo, rarity tiers, achievements, cursor companion, drag-to-rearrange, graphical Crash Doctor. Deliberately over-built so it can be trimmed down rather than built up. |
| `v4-world.html` | **The instance editor.** The living world (30 species, 43 props, 54 easter eggs, Field Journal, music). Remaining fixes in `V4-FIX-PLAN.md`. |
| `v5-lab.html` | **The Lab, revamped.** Six benches as physical workstations, each four steps deep (try it → guess first → transfer → why). Build it / Add stuff / Something broke are fully playable, three are sketches. |
| `v5-browse.html` | **Browse, revamped.** Market stalls, a legible taste model, a "surprise me" machine, collectible shelf. |
| `music-preview.html` | Auditions the 13 tracks in `music-tracks.js`. Also the reference WebAudio engine for A1. |

**[`V5-PORT-PLAN.md`](V5-PORT-PLAN.md) is the handoff** for rebuilding all of this in the real app:
the ambience/foreground split, the living background as an option on every page, Simple mode, Browse
and the Lab. Its prime directive is *port, don't rewrite*.

`v4-world.html` and `music-preview.html` load `music-tracks.js` by `src`, so **serve this folder**
rather than opening them from disk — `.claude/launch.json` has a `prototypes` entry on port 5180.
Everything except music still works from disk; the music controls disable themselves if the data
is missing.

## Plans

| File | What it covers |
|---|---|
| `V4-WORLD-PLAN.md` | The spec v4 was built from. |
| `V4-FIX-PLAN.md` | F1–F13 defects + A1 music. F2 and F4 are done. |
| `V5-PORT-PLAN.md` | Reconstructing the prototype in the real app, split into a global ambience layer and the High Interaction UI. |

`cobbleverse-icon.png` is the real pack logo (from the instance's `agora_pack_icon_url`), embedded
as a data URI in v3.

## Design rules these encode

- **Play is the hero.** The launcher's main screen should read as a save-select, not a control panel.
- **The health check is core, not a speed bump** — so it was made the most fun moment in the flow.
  A check nobody resents sitting through is a check that actually gets run.
- **Limited by intention.** Loader, Java, memory, snapshots, suggestions, and raw identifiers live
  behind Advanced or stay in Standard. The primary audience does not care about Minecraft or Agora
  internals.
- **Plain language.** "One mod is missing its file", "likely / maybe / unlikely" — never
  "1 blocker · 24 recommendations" or confidence percentages.
- **The safety spine is unchanged.** Gestures still create intent; the reviewed operations remain
  authoritative. Only the presentation is being rebuilt.
- **Reduced motion is honoured everywhere.** Ambient motion is flavour and always removable; state
  changes stay legible without it.

## Known deliberate exception

v2 onward knowingly breaks the coordination rule *"animation communicates causality, not
decoration"* — decorative motion is the point of this mode. The **Lab keeps the strict rule**, where
motion genuinely carries the teaching. Recorded here so the split is a decision, not drift.

## Data note

Mod names are the real 136-mod contents of "Copy of COBBLEVERSE"; item art is a generated
placeholder monogram. Production uses each mod's real `icon_url`, which the Standard view already
renders and the old live adapter discarded.

## V4 status

`v4-world.html` implements `V4-WORLD-PLAN.md`. Review found the structure sound (54/54 eggs wired,
tier-3 chains are real state machines, all accessibility/performance requirements met) but the
rendering and physics layer needs a fix pass — see `V4-FIX-PLAN.md`.

Known-broken as of review: nothing is tethered to the terrain under mouse parallax (entities keep a
fixed screen x while the ridges slide, and re-sample a different ridge column as the mouse moves, so
they slide and bob over the landscape); terrain alignment off by one ridge segment (33% of positions,
caused by `Math.round` in the original plan — should be `Math.floor`); quadruped legs anchored to the
ground instead of the body and only 2 of 4 drawn; no fish species at all (the pond spawns a frog with
a no-op effect); every quadruped sharing one generic box silhouette; ambient spawning stopping
permanently after a few minutes (perched, pond and sleeping animals never despawn, so they fill the
12-entity cap — measured climbing 1→10 in 97s and never falling); and the pond being a smooth
ellipse pasted onto blocky pixel art at a single ground sample, so it floats over sloped terrain.

## Verifying egg reachability

`verify-eggs.js` drives all 54 easter eggs through the same code paths a real click takes and prints
a pass/blocked/fail table. Expose the world object as described in its header, open the prototype,
paste the file into the console, and run `await verifyEggs()`.

Baseline at the time of writing: **47 / 54 reachable**, 7 blocked (4 by the missing fish species,
3 by missing item sources), 0 unexpected failures. After F3 and F10 land it must read 54 / 54.

Run it after every fix in `V4-FIX-PLAN.md` — it is the fastest way to catch an F7 coordinate-refactor
regression, which otherwise breaks chains silently.

## Music

`music-tracks.js` holds **13 complete public-domain pieces** — about 37 minutes — in the WebAudio
engine's `[pitch, beats]` format. Nothing is transcribed from memory: every track is generated by
`mxl2track.py` from real MusicXML, and the file self-checks on load that each voice sums to exactly
the track's `beats`.

| Mood | Tracks |
|---|---|
| playful | Sugar Plum Fairy · Moonlight mvt II |
| exciting | Flight of the Bumblebee · In the Hall of the Mountain King |
| dramatic | Symphony No. 5 mvt I |
| moody | Für Elise · Moonlight mvt I · Greensleeves |
| calm | Canon in D · Clair de Lune · Gymnopédies 1–3 |

**Audition them** in `music-preview.html` — a card per track, click to play, per-voice mute, loop
toggle, nothing autoplays. It needs to be served rather than opened from disk (the page loads
`music-tracks.js` via a script tag); `.claude/launch.json` has a `prototypes` entry that serves this
folder on port 5180. Its player is also the reference engine for the fix plan's A1.

### Instruments

Seven synth techniques, switchable while a track plays. They are different *methods*, not different
oscillator shapes, so they do not all fail in the same way at the extremes of a piece's range:

| Instrument | Technique |
|---|---|
| Chiptune | subtractive — square through a pitch-tracking lowpass |
| Music box | additive — fundamental + 2× + 3.9× partials, fast decay |
| Electric piano | FM, 2:1 ratio with a decaying modulation index |
| Plucked string | Karplus–Strong — noise burst in a period-length delay line |
| Strings | two detuned sawtooths, lowpass, slow attack |
| Organ | additive via `PeriodicWave` (drawbar partials), one oscillator |
| Bells | FM at an inharmonic 1.41 ratio, long decay |

Each track carries an `instrument` default chosen to suit the piece — Sugar Plum opens on the music
box because the original is a celesta, Mountain King and Greensleeves on plucked string (pizzicato,
lute), Symphony No. 5 on strings. Picking one by hand overrides the default for the rest of the
session.

### Why high notes were harsh, and what fixes it

The tuning was never wrong: A4 = 440, so middle C = C4 = 261.6 Hz. The problem is register. Sugar
Plum tops out at **B7 (3951 Hz) with 27.5% of its notes above C6** — by far the highest in the set —
and equal gain across the range is not equal loudness, because the ear peaks around 2–5 kHz. Two
things address it, both on by default:

- **Per-note gain rolloff** above 600 Hz (−3.4 dB at C6, −7.6 dB at C7, −10.5 dB at B7, floored so
  the top stays present). Real instruments roll off up there; nothing did before.
- **A master high shelf** of −7 dB from 2.6 kHz. A shelf rather than a lowpass, so it pulls down the
  band the ear is most sensitive to without dulling everything underneath.

The **Tame high notes** checkbox disables both, which is the quickest way to hear what the
difference actually buys. An **Octave** control (−2 … +1) is there too if a piece still sits too
high — at −1, Sugar Plum's top note lands at 1975 Hz, clear of the band entirely.

**To add a track:** find a MusicXML file for it (MuseScore, IMSLP) rather than decoding a scan, then
run `python mxl2track.py <file.mxl> <bars> <beatsPerBar>`. Use `classify()` for pitch-separated
writing like Satie, `classify_by_staff()` for two-staff piano. The script's docstring records the
traps, all of which produce plausible-sounding but wrong output if unhandled: chords spanning staves,
unstable voice numbers, overlapping notes, rounding drift, per-movement tempo marks, `<per-minute>`
counting beat-units rather than quarters, and 1st/2nd endings.

**Licensing: settled.** Every composition is public domain (latest death: Satie, 1925). The sources
are too — Moonlight is OpenScore CC0, Mountain King declares Public Domain, the Gymnopédies were
OMR'd from an IMSLP scan, and the remainder came from a GitHub library that publishes MusicXML
explicitly as public domain. Recorded in `V4-FIX-PLAN.md` A1 Licensing.
