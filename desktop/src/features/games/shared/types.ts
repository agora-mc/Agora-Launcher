/**
 * Shared shapes for the multi-game desktop UI (MASTER_SPEC §26).
 *
 * Mirrors `desktop/src-tauri/src/games/dto.rs` and `error.rs`. A slice that needs a
 * shape only it uses declares it in its own `api.ts`; anything two slices read goes here.
 */

/** Every `games_*` command rejects with this. `code` is stable; branch on it, not on `message`. */
export interface GamesError {
  code: string;
  message: string;
  /** The agora-core `ERR_...` code, when the failure came from core. */
  coreCode?: string;
}

/** The game id Minecraft is listed under. It is never "found" by discovery. */
export const MINECRAFT_GAME_ID = 'minecraft';

export type RuntimeResolution =
  | {
      status: 'identified';
      runtime: { game: string; store: string; version: string; build: string | null };
      source: string;
    }
  | { status: 'unidentified'; reasons: string[] };

export interface InstallSummary {
  installId: string;
  store: string;
  name: string;
  location: string;
  storeVersion: string | null;
  storeBuild: string | null;
  addOnCount: number;
  runtime: RuntimeResolution;
}

export interface GameListing {
  gameId: string;
  name: string;
  installs: InstallSummary[];
}

export interface GamesList {
  games: GameListing[];
}

export function isGamesError(value: unknown): value is GamesError {
  return (
    typeof value === 'object'
    && value !== null
    && typeof (value as { code?: unknown }).code === 'string'
    && typeof (value as { message?: unknown }).message === 'string'
  );
}
