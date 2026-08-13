/**
 * The game API that species/props/eggs logic closes over.
 *
 * In the prototype everything lived in one IIFE scope, so a species' onClick
 * could reach WORLD, findEgg, dropGroundItem, playVoice and the state
 * helpers by name. This interface bundles exactly those callables so the
 * ported handlers can keep their original bodies. It is defined here so
 * `species.ts` / `props.ts` / `eggs.ts` only import this type (erased at
 * runtime) and never create a runtime import cycle with `world.ts`.
 */

import type { Entity, Prop, WorldState } from './types';

export interface VoiceDef {
  /** Flat frequency list, played in sequence (prototype's `seq`). */
  seq?: number[];
  freqs?: number[];
  d?: number;
  t?: OscillatorType;
  v?: number;
  gap?: number;
}

export interface GameApi {
  /** The living world, bound after it is built. */
  world: () => WorldState;
  reactState: (e: Entity, fx: string, dur?: number) => void;
  fleeState: (e: Entity, mul?: number) => void;
  boltState: (e: Entity) => void;
  exitToward: (e: Entity, tx: number) => void;
  dropGroundItem: (itemId: string, x: number) => void;
  playVoice: (v: VoiceDef | null | undefined) => void;
  findEgg: (id: string) => void;
  shakeProp: (p: Prop) => void;
  blip: (f: number, d?: number, t?: OscillatorType, v?: number) => void;
}
