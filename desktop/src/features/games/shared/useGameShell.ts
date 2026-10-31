import { useCallback, useEffect, useState } from 'react';
import { getSetting, setSetting } from '@/lib/tauri';
import { listInstalledGames } from '../discovery/api';
import { MINECRAFT_GAME_ID, type GameListing } from './types';

/** The settings key the active game persists under (the app's own settings store). */
export const ACTIVE_GAME_SETTING = 'active_game';

export interface GameShell {
  /** The persisted choice. Minecraft until the stored value has loaded. */
  activeGame: string;
  /**
   * What the shell shows. A stored game that discovery no longer finds, or a discovery that
   * failed, falls back to Minecraft rather than showing a game the user cannot reach.
   */
  shownGame: string;
  /** Non-Minecraft games found on this machine. Empty until discovery has answered. */
  foundGames: GameListing[];
  /** The header switcher is shown only when there is another game to switch to. */
  switcherVisible: boolean;
  /** True once the stored choice and discovery have both settled (answered or failed). */
  ready: boolean;
  /** Choose a game and remember the choice across restarts. */
  setActiveGame: (gameId: string) => void;
}

type Discovery =
  | { status: 'loading' }
  | { status: 'failed' }
  | { status: 'loaded'; games: GameListing[] };

/** Anything that is not a plausible game id is treated as "no choice made", not as a game. */
export function parseActiveGame(value: unknown): string {
  if (typeof value !== 'string') return MINECRAFT_GAME_ID;
  if (value.length === 0 || value.length > 64) return MINECRAFT_GAME_ID;
  return value;
}

export function useGameShell(): GameShell {
  // null = the stored value has not come back yet.
  const [stored, setStored] = useState<string | null>(null);
  const [discovery, setDiscovery] = useState<Discovery>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    getSetting(ACTIVE_GAME_SETTING)
      .then((value) => {
        if (!cancelled) setStored(parseActiveGame(value));
      })
      .catch(() => {
        // An unreadable choice means Minecraft, the same as a fresh install.
        if (!cancelled) setStored(MINECRAFT_GAME_ID);
      });
    listInstalledGames()
      .then((list) => {
        if (!cancelled) {
          const games = list.games.filter((game) => game.gameId !== MINECRAFT_GAME_ID && game.installs.length > 0);
          setDiscovery({ status: 'loaded', games });
        }
      })
      .catch(() => {
        // Not a list (or no discovery at all): nothing is found, so nothing is offered.
        if (!cancelled) setDiscovery({ status: 'failed' });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setActiveGame = useCallback((gameId: string) => {
    setStored(gameId);
    // A choice that fails to save costs the user one click on the next start; it is not
    // worth interrupting them for.
    setSetting(ACTIVE_GAME_SETTING, gameId).catch(() => undefined);
  }, []);

  const foundGames = discovery.status === 'loaded' ? discovery.games : [];
  const activeGame = stored ?? MINECRAFT_GAME_ID;
  const ready = stored !== null && discovery.status !== 'loading';
  const isFound = foundGames.some((game) => game.gameId === activeGame);
  const shownGame = !ready || activeGame === MINECRAFT_GAME_ID
    ? MINECRAFT_GAME_ID
    : isFound
      ? activeGame
      : MINECRAFT_GAME_ID;

  return {
    activeGame,
    shownGame,
    foundGames,
    switcherVisible: foundGames.length > 0,
    ready,
    setActiveGame,
  };
}
