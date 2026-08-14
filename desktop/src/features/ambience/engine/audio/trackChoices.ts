/**
 * Track and instrument names for the settings dropdowns.
 *
 * Deliberately separate from `tracks.ts`. That module carries ~10,000 note
 * events (a 147 kB lazy chunk) and must stay out of any eagerly-loaded page; a
 * settings panel only needs the labels. Importing `tracks.ts` to populate a
 * `<select>` would drag the whole score into the initial bundle.
 *
 * Keep in step with `tracks.ts` — `trackChoices.test.ts` fails if they drift.
 */

export interface TrackChoice {
  id: string;
  name: string;
  mood: string;
}

export const MUSIC_TRACK_CHOICES: TrackChoice[] = [
  { id: 'sugar-plum', name: 'Dance of the Sugar Plum Fairy', mood: 'playful' },
  { id: 'moonlight-allegretto', name: 'Moonlight Sonata (2nd movement)', mood: 'playful' },
  { id: 'bumblebee', name: 'Flight of the Bumblebee', mood: 'exciting' },
  { id: 'mountain-king', name: 'In the Hall of the Mountain King', mood: 'exciting' },
  { id: 'fate', name: 'Symphony No. 5 (1st movement)', mood: 'dramatic' },
  { id: 'fur-elise', name: 'Für Elise', mood: 'moody' },
  { id: 'moonlight', name: 'Moonlight Sonata (1st movement)', mood: 'moody' },
  { id: 'greensleeves', name: 'Greensleeves', mood: 'moody' },
  { id: 'canon-in-d', name: 'Canon in D', mood: 'calm' },
  { id: 'clair-de-lune', name: 'Clair de Lune', mood: 'calm' },
  { id: 'gymnopedie-1', name: 'Gymnopédie No. 1', mood: 'calm' },
  { id: 'gymnopedie-2', name: 'Gymnopédie No. 2', mood: 'calm' },
  { id: 'gymnopedie-3', name: 'Gymnopédie No. 3', mood: 'calm' },
];

export interface InstrumentChoice {
  id: string;
  name: string;
}

export const INSTRUMENT_CHOICES: InstrumentChoice[] = [
  { id: 'chip', name: 'Chiptune' },
  { id: 'musicbox', name: 'Music box' },
  { id: 'rhodes', name: 'Electric piano' },
  { id: 'pluck', name: 'Plucked string' },
  { id: 'strings', name: 'Strings' },
  { id: 'organ', name: 'Organ' },
  { id: 'bell', name: 'Bells' },
];
