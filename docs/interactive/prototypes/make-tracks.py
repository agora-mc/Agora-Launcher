"""Regenerate music-tracks.js from the MusicXML sources.

    python make-tracks.py <dir-with-mxl-files> [--check]

Every track in music-tracks.js is produced here; none is hand-written. This file
is the record of WHICH source and WHICH settings produced each track, so the
output is reproducible rather than a one-off artifact.

--check prints the structure and the first few events of every voice without
writing anything, which is the fastest way to catch a bad conversion: if the
opening of a piece you know does not look right in that dump, it will not sound
right either.
"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mxl2track as M

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'music-tracks.js')

# wave/gain per voice role
ROLE = {
    'mel':   ('melody',     'triangle', 0.20),
    'chord': ('chord',      'sine',     0.13),
    'bass':  ('bass',       'sine',     0.17),
    'rh':    ('right hand', 'triangle', 0.18),
    'lh':    ('left hand',  'sine',     0.16),
}

# split: "pitch" -> classify() (melody/chord/bass by register, right for Satie)
#        "staff" -> classify_by_staff() (right/left hand, right for piano writing)
# clamp: OMR sources only - see mxl2track.parse
# window: (lo, hi) in quarter-note beats, to lift one movement out of a file
# pad:    leading rest that completes a pickup bar, so `bars` stays a whole number
TRACKS = [
 dict(id="gymnopedie-1", instrument="rhodes", name="Gymnopedie No. 1", composer="Erik Satie", year=1888,
      marking="Lent et douloureux", mood="calm", file="IMSLP935937.mvt1.mxl",
      bpb=3, bpm=76, split="pitch", clamp=True,
      source="IMSLP score -> Audiveris OMR -> MusicXML"),
 dict(id="gymnopedie-2", instrument="rhodes", name="Gymnopedie No. 2", composer="Erik Satie", year=1888,
      marking="Lent et triste", mood="calm", file="IMSLP935937.mvt2.mxl",
      bpb=3, bpm=76, split="pitch", clamp=True,
      source="IMSLP score -> Audiveris OMR -> MusicXML"),
 dict(id="gymnopedie-3", instrument="rhodes", name="Gymnopedie No. 3", composer="Erik Satie", year=1888,
      marking="Lent et grave", mood="calm", file="IMSLP935937.mvt3.mxl",
      bpb=3, bpm=76, split="pitch", clamp=True,
      source="IMSLP score -> Audiveris OMR -> MusicXML"),
 dict(id="mountain-king", instrument="pluck", name="In the Hall of the Mountain King", composer="Edvard Grieg",
      year=1875, marking="Alla marcia e molto marcato", mood="exciting",
      file="In the Hall of the Mountain King - Ivory.mxl", bpb=4, bpm=138, split="staff",
      source="MusicXML, rights: Public Domain"),

 dict(id="fur-elise", instrument="musicbox", name="Fur Elise", composer="Ludwig van Beethoven", year=1810,
      marking="Poco moto", mood="moody", file="Fur_Elise.mxl",
      bpb=1.5, bpm=72, split="staff", pad=1.0,
      source="MusicXML (MuseScore), transcription of the piano original"),
 dict(id="moonlight", instrument="rhodes", name="Moonlight Sonata (1st movement)", composer="Ludwig van Beethoven",
      year=1801, marking="Adagio sostenuto", mood="moody",
      file="Moonlight Sonata - Ivory.mxl", bpb=4, bpm=60, split="staff", window=(0, 276),
      source="OpenScore (CC0) MusicXML"),
 dict(id="moonlight-allegretto", instrument="rhodes", name="Moonlight Sonata (2nd movement)",
      composer="Ludwig van Beethoven", year=1801, marking="Allegretto", mood="playful",
      file="Moonlight Sonata - Ivory.mxl", bpb=3, bpm=168, split="staff", window=(276, 456),
      tempo_note="file marks 210, outside any performed tempo for this movement",
      source="OpenScore (CC0) MusicXML"),
 dict(id="fate", instrument="strings", name="Symphony No. 5, 1st movement", composer="Ludwig van Beethoven",
      year=1808, marking="Allegro con brio", mood="dramatic",
      file="Beethoven_Symphony_No._5_1st_movement_Piano_solo.mxl", bpb=2, bpm=164,
      split="staff", source="MusicXML (MuseScore), piano reduction"),
 dict(id="clair-de-lune", instrument="rhodes", name="Clair de Lune", composer="Claude Debussy", year=1905,
      marking="Andante tres expressif", mood="calm", file="Clair_de_Lune__Debussy.mxl",
      bpb=4.5, bpm=66, split="staff", upto=324.0,
      tempo_note="file marks 48, a dotted-quarter pulse of 32 - slower than any recording",
      source="MusicXML (MuseScore), transcription of the piano original"),
 dict(id="bumblebee", instrument="chip", name="Flight of the Bumblebee", composer="Nikolai Rimsky-Korsakov",
      year=1900, marking="Presto", mood="exciting", file="Flight_of_the_Bumblebee.mxl",
      bpb=2, bpm=144, split="staff", source="MusicXML (MuseScore), piano arrangement"),
 dict(id="sugar-plum", instrument="musicbox", name="Dance of the Sugar Plum Fairy", composer="Pyotr Ilyich Tchaikovsky",
      year=1892, marking="Andante non troppo", mood="playful",
      file="Dance_of_the_sugar_plum_fairy.mxl", bpb=2, bpm=70, split="staff",
      source="MusicXML (MuseScore), piano arrangement"),
 dict(id="canon-in-d", instrument="pluck", name="Canon in D", composer="Johann Pachelbel", year=1680,
      marking="Andante", mood="calm", file="Canon_in_D.mxl", bpb=4, bpm=100, split="staff",
      source="MusicXML (MuseScore), piano arrangement"),
 dict(id="greensleeves", instrument="pluck", name="Greensleeves", composer="Traditional (English)", year=1580,
      marking="Andante", mood="moody", file="Greensleeves_for_Piano_easy_and_beautiful.mxl",
      bpb=3, bpm=120, split="staff", pad=2.0,
      source="MusicXML (MuseScore), piano arrangement of the traditional tune"),
]


def build(cfg, srcdir):
    root = M.load(os.path.join(srcdir, cfg['file']))
    events, div, tempo, total, bars = M.parse(root, clamp_bars=cfg.get('clamp', False))
    if cfg.get('window'):
        lo, hi = cfg['window']
        events = M.window(events, lo, hi)
        total = hi - lo
    pad = cfg.get('pad', 0.0)
    if pad:
        events = [dict(e, t=e['t'] + pad) for e in events]
        total += pad
    upto = cfg.get('upto', round(total, 4))

    streams = (M.classify(events) if cfg['split'] == 'pitch' else M.classify_by_staff(events))
    order = ('mel', 'chord', 'bass') if cfg['split'] == 'pitch' else ('rh', 'lh')
    return [(ROLE[k], M.to_seq(streams[k], upto)) for k in order], upto, tempo


def js(cfg, voices, upto):
    bars = upto / cfg['bpb']
    bars_s = ('%d' % round(bars)) if abs(bars - round(bars)) < 1e-6 else ('%.4f' % bars)
    L = ['  {',
         '    id: "%s", name: "%s", composer: "%s", year: %d,'
         % (cfg['id'], cfg['name'], cfg['composer'], cfg['year']),
         '    marking: "%s", mood: "%s", bpm: %g, beatsPerBar: %g, bars: %s, beats: %g,'
         % (cfg['marking'], cfg['mood'], cfg['bpm'], cfg['bpb'], bars_s, upto),
         '    source: "%s",' % cfg['source']]
    if cfg.get('tempo_note'):
        L.append('    tempoNote: "%s",' % cfg['tempo_note'])
    L.append('    instrument: "%s",' % cfg.get('instrument', 'chip'))
    L.append('    voices: [')
    for i, ((nm, wave, gain), seq) in enumerate(voices):
        L.append('      { name: "%s", wave: "%s", gain: %.2f, seq: [' % (nm, wave, gain))
        L.append('         ' + M.fmt(seq) + '] }' + (',' if i < len(voices) - 1 else ''))
    L += ['    ]', '  },']
    return '\n'.join(L)


HEADER = '''/**
 * music-tracks.js - public-domain music for the prototype, from real scores.
 *
 * GENERATED FILE. Do not edit by hand: run `python make-tracks.py <src-dir>`,
 * which records which source and which settings produced each track.
 *
 * Nothing here is transcribed from memory. Every track is generated by
 * mxl2track.py from a MusicXML source; see `source` on each entry. To add one,
 * find a MusicXML file (MuseScore, IMSLP) rather than decoding a scan.
 *
 * Complete pieces, not loop edits - they repeat from the top the way the music
 * does. Repeats are played once through; 1st endings are skipped so a single
 * pass flows. Every voice is verified to sum to exactly `beats`, and this file
 * throws on load if that stops being true (voices that disagree by a fraction
 * of a beat drift further apart on every repeat).
 *
 * Format: each voice is a sequential list of [pitch, beats], where pitch is a
 * note name, an array of names for a chord, or "R" for a rest. `beats` counts
 * quarter notes, and `bpm` is quarter notes per minute, so a beat lasts
 * 60 / bpm seconds. Durations are NOT on a fixed grid - triplets are 0.3333 -
 * so a player must walk each voice's list, not sample a 16th-note grid.
 *
 * LICENSING. Every composition here is public domain: Pachelbel d. 1706,
 * Beethoven d. 1827, Tchaikovsky d. 1893, Rimsky-Korsakov d. 1908, Debussy
 * d. 1918, Satie d. 1925, Grieg d. 1907, Greensleeves traditional (c. 1580).
 * Sources differ in what they declare, and that difference matters:
 *   - Moonlight is OpenScore (CC0) - an explicit public-domain dedication.
 *   - Mountain King declares rights: Public Domain.
 *   - The Gymnopedies are our own OMR of an IMSLP scan.
 *   - The rest are MuseScore community uploads with no rights statement. The
 *     NOTES are public domain, but a community upload that is a genuine
 *     ARRANGEMENT (an orchestral work reduced for piano, or an "easy piano"
 *     rewrite) reflects the arranger's own choices. That applies to Symphony
 *     No. 5, Flight of the Bumblebee, Sugar Plum Fairy, Canon in D and
 *     Greensleeves. Fur Elise and Clair de Lune were written for piano, so
 *     those files are transcriptions of a public-domain original.
 *   Flagged, not resolved - see the licensing note in V4-FIX-PLAN.md A1.
 *
 * TEMPO. Taken from each file's first tempo mark unless `tempoNote` says
 * otherwise, which flags the two marks that fall outside any performed tempo.
 */
window.MUSIC_TRACKS = [
'''

FOOTER = '''];

/* Every voice must sum to the track's beat count. Throw loudly rather than let
   a track quietly fall apart as it loops. */
window.MUSIC_TRACKS.forEach(function (t) {
  t.voices.forEach(function (v) {
    var got = v.seq.reduce(function (s, e) { return s + e[1]; }, 0);
    if (Math.abs(got - t.beats) > 0.001) {
      throw new Error(t.id + " voice " + v.name + ": " + got + " beats, expected " + t.beats);
    }
  });
});
'''

if __name__ == '__main__':
    args = [a for a in sys.argv[1:] if not a.startswith('--')]
    if not args:
        sys.exit(__doc__)
    srcdir, check = args[0], '--check' in sys.argv
    parts = []
    for cfg in TRACKS:
        voices, upto, tempo = build(cfg, srcdir)
        parts.append(js(cfg, voices, upto))
        secs, bars = upto / cfg['bpm'] * 60, upto / cfg['bpb']
        print('%-22s beats=%-7g bars=%-8.3f %d:%02d  file_tempo=%-7s used=%-5g %s%s'
              % (cfg['id'], upto, bars, secs // 60, secs % 60, tempo, cfg['bpm'],
                 ' '.join('%s:%d' % (v[0][0][:3], len(v[1])) for v in voices),
                 '' if abs(bars - round(bars)) < 1e-6 else '  <-- fractional bars'))
        if check:
            for (nm, _, _), seq in voices:
                print('     %-12s %s' % (nm, ' '.join(
                    '%s:%g' % (e[0] if isinstance(e[0], str) else '[' + '+'.join(e[0]) + ']', e[1])
                    for e in seq[:9])))
    if not check:
        with open(OUT, 'w', encoding='utf-8') as f:
            f.write(HEADER + '\n'.join(parts) + '\n' + FOOTER)
        print('\nwrote %s (%d bytes, %d tracks)' % (OUT, os.path.getsize(OUT), len(TRACKS)))
