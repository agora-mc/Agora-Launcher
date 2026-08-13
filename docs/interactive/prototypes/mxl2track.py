"""MusicXML (.mxl) -> [pitch, beats] voice arrays for the prototype's WebAudio engine.

Two things make a naive parser wrong on this score, both handled here:

 1. A chord can SPAN STAVES. In Gymnopedie the accompaniment chord is written
    B3 on staff 2 + D4/F#4 on staff 1, all one voice. Grouping chord notes by
    (staff, voice) splits it and attaches the upper notes to the wrong event.
    Chord grouping keys on VOICE ONLY.

 2. Audiveris voice numbers are not stable across the piece. Bars 1-4 have no
    melody, so the accompaniment is voice 1; from bar 5 the melody becomes
    voice 1 and the accompaniment becomes voice 2. Keying output streams by
    voice number therefore interleaves melody and chords. Streams are assigned
    by CONTENT instead: >=2 pitches = chord, single low pitch = bass, else melody.
"""
import zipfile, xml.etree.ElementTree as ET, sys, collections

STEP = {'C':0,'D':2,'E':4,'F':5,'G':7,'A':9,'B':11}
NAMES = ['C','C#','D','D#','E','F','F#','G','G#','A','A#','B']

def load(path):
    z = zipfile.ZipFile(path)
    name = [n for n in z.namelist() if n.endswith('.xml') and 'META-INF' not in n][0]
    return ET.fromstring(z.read(name))

def pitch_name(p):
    step = p.findtext('step'); octv = int(p.findtext('octave'))
    semi = STEP[step] + int(p.findtext('alter') or 0)
    octv += semi // 12
    return NAMES[semi % 12] + str(octv)

def octave_of(n):
    i = 1
    while i < len(n) and not (n[i].isdigit() or n[i] == '-'): i += 1
    return int(n[i:])

BEAT_UNIT = {'whole': 4.0, 'half': 2.0, 'quarter': 1.0, 'eighth': 0.5,
             '16th': 0.25, '32nd': 0.125}

def metronome_qpm(el):
    """Convert a <metronome> marking to quarter-notes-per-minute.

    <per-minute> counts BEAT-UNITS, not quarters: 3/8 marked eighth=144 is
    quarter=72. Reading per-minute raw makes compound-meter pieces play at
    2-3x the intended speed.
    """
    unit = BEAT_UNIT.get(el.findtext('beat-unit') or 'quarter')
    if unit is None: return None
    for _ in el.findall('beat-unit-dot'): unit *= 1.5
    try: return float(el.findtext('per-minute')) * unit
    except (TypeError, ValueError): return None

def ending_numbers(el):
    return [s.strip() for s in (el.get('number') or '').split(',') if s.strip()]

def parse(root, skip_first_endings=True, clamp_bars=False):
    """Linear read-through of the score.

    Repeats are not taken (one pass is what a background loop wants), but a
    1st/2nd ending must still be handled: playing both back to back produces an
    audible stutter right where the music should flow on. Non-final endings are
    skipped so the single pass reads ...bar 7, 2nd ending, bar 10...

    clamp_bars limits each measure to the length its time signature says, for
    sources where a bar overrunning is a mistake rather than the music. Set it
    for OMR output: Audiveris writes an accompaniment chord as a SEQUENTIAL note
    after the melody in the same voice, so Gymnopedie No. 2 bar 59 reads 2+2
    beats in a 3/4 bar and the whole piece gains a beat. Leave it off for
    engraved sources, where a long bar is usually real - Symphony No. 5 bar 269
    is a genuine 10-beat cadenza that clamping would cut off.
    """
    divisions, tempo, total = 1, None, 0.0
    events = []                                  # {t, dur, notes[]}
    bars = []                                    # (measure number, start beat, timesig)
    by_voice = {}                                # voice -> last event (for chord stacking)
    sig, nominal = None, None                    # time signature, and its length in quarters

    last_ending = max((n for e in root.iter('ending') for n in ending_numbers(e)),
                      default=None)

    for part in root.findall('part'):
        cursor = 0.0
        skipping = False
        for meas in part.findall('measure'):
            if skip_first_endings and last_ending:
                for e in meas.iter('ending'):
                    if e.get('type') == 'start':
                        skipping = last_ending not in ending_numbers(e)
                if skipping:
                    if any(e.get('type') in ('stop', 'discontinue') for e in meas.iter('ending')):
                        skipping = False          # this measure is the last skipped one
                    continue
            attrs = meas.find('attributes')
            if attrs is not None and attrs.findtext('divisions'):
                divisions = int(attrs.findtext('divisions'))
            if attrs is not None and attrs.find('time') is not None:
                sig = '%s/%s' % (attrs.findtext('time/beats'), attrs.findtext('time/beat-type'))
                try:
                    nominal = float(attrs.findtext('time/beats')) * 4.0 / float(
                        attrs.findtext('time/beat-type'))
                except (TypeError, ValueError, ZeroDivisionError):
                    nominal = None
            bars.append((meas.get('number'), cursor, sig))
            # FIRST tempo wins. A multi-movement file carries a tempo mark per
            # movement, and overwriting leaves the last one - which is how the
            # Moonlight file reported 155 (movement 3) for a piece that opens at 60.
            if tempo is None:
                for snd in meas.iter('sound'):
                    if snd.get('tempo'):
                        try: tempo = float(snd.get('tempo')); break
                        except ValueError: pass
            if tempo is None:
                for met in meas.iter('metronome'):
                    tempo = metronome_qpm(met)
                    if tempo: break

            start = mmax = cursor
            for el in meas:
                if el.tag == 'note':
                    dur = float(el.findtext('duration') or 0) / divisions
                    voice = el.findtext('voice') or '1'
                    p = el.find('pitch')
                    name = None if (el.find('rest') is not None or p is None) else pitch_name(p)

                    if el.find('chord') is not None:          # stack, do not advance
                        ev = by_voice.get(voice)
                        if ev is not None and name: ev['notes'].append(name)
                        continue

                    tie_stop = any(t.get('type') == 'stop' for t in el.findall('tie'))
                    prev = by_voice.get(voice)
                    if tie_stop and prev and name and name in prev['notes']:
                        prev['dur'] += dur                    # merge tied note
                    else:
                        ev = {'t': cursor, 'dur': dur, 'notes': [name] if name else [],
                              'staff': el.findtext('staff') or '1'}
                        events.append(ev); by_voice[voice] = ev
                    cursor += dur
                    mmax = max(mmax, cursor)
                elif el.tag == 'backup':
                    cursor -= float(el.findtext('duration') or 0) / divisions
                elif el.tag == 'forward':
                    cursor += float(el.findtext('duration') or 0) / divisions
                    mmax = max(mmax, cursor)
            # Advance by how far the measure actually REACHED, not where its
            # last voice happened to stop. A measure whose final voice ends
            # early (backup, then a short voice) otherwise pulls every later
            # bar forward, and bar numbering silently desyncs from the score.
            cursor = max(mmax, start)
            if clamp_bars and nominal and cursor - start > nominal + 1e-6:
                cursor = start + nominal
            total = max(total, cursor)
    return events, divisions, tempo, total, bars

def bar_start(bars, number):
    """Beat offset of a measure, by its printed number."""
    for n, t, _ in bars:
        if n == str(number): return t
    raise KeyError('no measure %s' % number)

def window(events, lo, hi):
    """Events falling in [lo, hi), re-based to 0. Used to lift one movement
    out of a multi-movement file, or to trim a piece to a usable length."""
    out = []
    for e in events:
        if lo - 1e-6 <= e['t'] < hi - 1e-6:
            f = dict(e); f['t'] = e['t'] - lo
            f['dur'] = min(e['dur'], hi - e['t'])
            out.append(f)
    return out

def classify(events, bass_max_octave=3):
    streams = {'mel': [], 'chord': [], 'bass': []}
    for e in events:
        if not e['notes']: continue                          # rests are re-inserted later
        if len(e['notes']) >= 2:            streams['chord'].append(e)
        elif octave_of(e['notes'][0]) <= bass_max_octave: streams['bass'].append(e)
        else:                                streams['mel'].append(e)
    return streams

def classify_by_staff(events):
    """For two-staff piano writing, split right hand / left hand.

    The pitch-based split below is tuned to Gymnopedie, where the melody never
    descends into the bass register. It is wrong for arrangements like Mountain
    King, whose theme STARTS in octave 1-2 and climbs through the piece - a
    fixed octave threshold would file the entire opening melody as bass.
    """
    streams = {'rh': [], 'lh': []}
    for e in events:
        if not e['notes']: continue
        streams['rh' if e.get('staff', '1') == '1' else 'lh'].append(e)
    return streams

def to_seq(events, upto):
    """Sequential playback line. Streams can contain overlapping events (a
    sustained note under a new one); a monophonic line must clip the earlier
    note at the next onset, or every overlap inflates the total length and the
    voices drift out of sync."""
    evs = sorted(events, key=lambda x: (x['t'], -x['dur']))
    seq, t = [], 0.0
    for i, e in enumerate(evs):
        if e['t'] >= upto - 1e-6: break
        start = max(e['t'], t)
        gap = round(start - t, 4)
        if gap > 0.001: seq.append(['R', gap])
        nxt = next((x['t'] for x in evs[i+1:] if x['t'] > start + 1e-6), upto)
        end = min(e['t'] + e['dur'], nxt, upto)
        dur = round(end - start, 4)
        if dur <= 0.001: continue
        seq.append([e['notes'][0] if len(e['notes']) == 1 else sorted(set(e['notes'])), dur])
        t = end
    if upto - t > 0.001: seq.append(['R', round(upto - t, 4)])
    # Per-event rounding accumulates: force the sequence to sum to EXACTLY upto,
    # or voices drift apart a little more on every loop.
    if seq:
        drift = round(upto - sum(e[1] for e in seq), 6)
        if abs(drift) > 1e-9:
            seq[-1][1] = round(seq[-1][1] + drift, 6)
            if seq[-1][1] <= 0: seq.pop()
    return seq

def fmt(seq, per_line=5):
    out, line = [], []
    for ev in seq:
        n = ev[0]
        s = ('"%s"' % n) if isinstance(n, str) else ('[' + ','.join('"%s"' % x for x in n) + ']')
        line.append('[%s,%g]' % (s, ev[1]))
        if len(line) == per_line: out.append(', '.join(line)); line = []
    if line: out.append(', '.join(line))
    return ',\n         '.join(out)

if __name__ == '__main__':
    path = sys.argv[1]
    bars = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    bpb  = float(sys.argv[3]) if len(sys.argv) > 3 else 3.0
    root = load(path)
    events, div, tempo, total, bars = parse(root)
    upto = bars * bpb if bars else total
    streams = classify(events)
    print('# divisions=%d tempo=%s totalBeats=%g (%g bars of %g)  ->  exporting %g bars'
          % (div, tempo, total, total / bpb, bpb, upto / bpb))
    for k in ('mel', 'chord', 'bass'):
        seq = to_seq(streams[k], upto)
        print('\n  %s: [%s],   // %d events, %g beats' % (k, fmt(seq), len(seq), sum(e[1] for e in seq)))
