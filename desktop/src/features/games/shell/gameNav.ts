/**
 * The navigation a non-Minecraft game shows: its four tabs, and the pages of one instance.
 * Settings stays shared and is not listed here; the sidebar adds it.
 */

import { BookMarked, Boxes, Download, Wrench, type LucideIcon } from 'lucide-react';
import type { GameInstancePage, GameTabId, Tab } from '@/lib/useDestination';

export interface GameShellTab {
  /** The sidebar id. Game ids are `game-*` so they never collide with a Minecraft tab. */
  id: Tab;
  /** The destination tab this sidebar entry opens. */
  tab: GameTabId;
  label: string;
  icon: LucideIcon;
  /** The emoji the command palette shows for this entry, matching the rest of the palette. */
  emoji: string;
}

export const GAME_SHELL_TABS: readonly GameShellTab[] = [
  { id: 'game-instances', tab: 'instances', label: 'Instances', icon: Boxes, emoji: '📦' },
  { id: 'game-library', tab: 'library', label: 'Library', icon: BookMarked, emoji: '📚' },
  { id: 'game-import', tab: 'import', label: 'Import', icon: Download, emoji: '⬇️' },
  { id: 'game-setup', tab: 'setup', label: 'Setup', icon: Wrench, emoji: '🔧' },
];

export interface GameInstancePageTab {
  id: GameInstancePage;
  label: string;
}

export const GAME_INSTANCE_PAGE_TABS: readonly GameInstancePageTab[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'content', label: 'Content' },
  { id: 'plugins', label: 'Plugins' },
  { id: 'ini-saves', label: 'INI & saves' },
  { id: 'tools', label: 'Tools' },
];

/** The game tab a sidebar id stands for, or null for a sidebar id that is not a game tab. */
export function gameTabForSidebarTab(tab: Tab): GameTabId | null {
  return GAME_SHELL_TABS.find((entry) => entry.id === tab)?.tab ?? null;
}

/** The sidebar id a game tab is highlighted under. */
export function sidebarTabForGameTab(tab: GameTabId): Tab {
  const entry = GAME_SHELL_TABS.find((candidate) => candidate.tab === tab);
  if (!entry) throw new Error(`No sidebar entry for game tab ${tab}`);
  return entry.id;
}
