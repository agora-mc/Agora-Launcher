/**
 * The dropdown labels must stay in step with the real score data.
 *
 * `trackChoices.ts` exists so a settings page can list the music WITHOUT pulling
 * in `tracks.ts` (~10,000 note events, a 147 kB lazy chunk). The cost of that
 * split is a second copy of the ids, which can silently drift — a renamed or
 * removed track would leave a dropdown entry that plays nothing at all. This
 * test is the thing that stops that.
 */

import { describe, expect, it } from 'vitest';
import { MUSIC_TRACK_CHOICES, INSTRUMENT_CHOICES } from './trackChoices';
import { MUSIC_TRACKS } from './tracks';

describe('music dropdown choices', () => {
  it('lists exactly the tracks that exist', () => {
    expect(MUSIC_TRACK_CHOICES.map((c) => c.id).sort())
      .toEqual(MUSIC_TRACKS.map((t) => t.id).sort());
  });

  it('labels each track with its real mood', () => {
    const moodById = new Map(MUSIC_TRACKS.map((t) => [t.id, t.mood]));
    for (const choice of MUSIC_TRACK_CHOICES) {
      expect(choice.mood, `mood for ${choice.id}`).toBe(moodById.get(choice.id));
    }
  });

  it('offers every instrument the tracks actually ask for', () => {
    const ids = new Set(INSTRUMENT_CHOICES.map((i) => i.id));
    for (const track of MUSIC_TRACKS) {
      if (track.instrument) {
        expect(ids.has(track.instrument), `instrument ${track.instrument}`).toBe(true);
      }
    }
  });

  it('has no duplicate ids in either list', () => {
    expect(new Set(MUSIC_TRACK_CHOICES.map((c) => c.id)).size).toBe(MUSIC_TRACK_CHOICES.length);
    expect(new Set(INSTRUMENT_CHOICES.map((c) => c.id)).size).toBe(INSTRUMENT_CHOICES.length);
  });
});
