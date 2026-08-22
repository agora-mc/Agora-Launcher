/**
 * MUSIC_TRACKS — WEB STUB.
 *
 * The desktop ships the full public-domain score here (~190 KB of generated
 * note data, see `desktop/src/features/ambience/engine/audio/tracks.ts`). The
 * site deliberately does NOT: the diorama mounts with `musicOn: false` and
 * `soundOn: false`, so the engine's lazy `import('./audio/tracks')` is never
 * executed. This file exists only so that import resolves at build time (and
 * so `music.ts`'s type imports keep their shape). The types mirror the
 * desktop module exactly; the track list is empty.
 *
 * Keep the types in step with the desktop copy — a future shared-package
 * extraction replaces this file with the real one.
 */

export type MusicMood = 'calm' | 'moody' | 'playful' | 'exciting' | 'dramatic' | 'peaceful' | 'wistful' | 'familiar' | 'bright';
export type MusicPitch = string | string[];

export interface MusicVoice {
  name: string;
  wave: OscillatorType;
  gain: number;
  seq: Array<[MusicPitch, number]>;
}

export interface MusicTrack {
  id: string;
  name: string;
  composer: string;
  year: number;
  marking: string;
  mood: MusicMood;
  bpm: number;
  beatsPerBar: number;
  bars: number;
  beats: number;
  source: string;
  instrument?: string;
  tempoNote?: string;
  /** Linear crescendo: gain multiplier at the very start of the piece,
   *  ramping to `crescendoPeak`× voice gain by the end of the pass
   *  (e.g. 0.35 → 1.5 = pp → ff, louder than 1×). */
  crescendo?: number;
  crescendoPeak?: number;
  voices: MusicVoice[];
}

export const MUSIC_TRACKS: MusicTrack[] = [];
