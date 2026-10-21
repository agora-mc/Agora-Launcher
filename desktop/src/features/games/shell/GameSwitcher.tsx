/**
 * The game switcher at the top of the sidebar. Minecraft plus each non-Minecraft game found.
 * It is rendered only when there is another game to switch to, so a Minecraft-only user never
 * sees it. Every choice is a `<button>` so the controller can reach it.
 */

import { useState } from 'react';
import { ChevronDown, ChevronUp, Gamepad2 } from 'lucide-react';
import { MINECRAFT_GAME_ID } from '../shared/types';

export interface GameSwitcherOption {
  gameId: string;
  label: string;
}

interface GameSwitcherProps {
  options: GameSwitcherOption[];
  shownGame: string;
  onSwitch: (gameId: string) => void;
  collapsed: boolean;
}

export function GameSwitcher({ options, shownGame, onSwitch, collapsed }: GameSwitcherProps) {
  const [open, setOpen] = useState(false);
  const current = options.find((option) => option.gameId === shownGame) ?? options[0];
  if (!current) return null;

  return (
    <div className="space-y-1" data-testid="game-switcher">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        aria-label={`Game: ${current.label}. Choose a game`}
        title={collapsed ? current.label : undefined}
        className={[
          'flex w-full items-center gap-2 rounded-lg border border-border px-2.5 py-2 text-left text-sm font-medium hover:bg-accent',
          collapsed ? 'justify-center px-0' : '',
        ].join(' ')}
      >
        <Gamepad2 className="h-4 w-4 shrink-0" aria-hidden="true" />
        {!collapsed && <span className="min-w-0 flex-1 truncate">{current.label}</span>}
        {!collapsed && (open ? <ChevronUp className="h-4 w-4 shrink-0" aria-hidden="true" /> : <ChevronDown className="h-4 w-4 shrink-0" aria-hidden="true" />)}
      </button>
      {open && (
        <div role="group" aria-label="Games" className="space-y-1">
          {options.map((option) => {
            const isCurrent = option.gameId === current.gameId;
            return (
              <button
                key={option.gameId}
                type="button"
                aria-pressed={isCurrent}
                title={collapsed ? option.label : undefined}
                onClick={() => {
                  setOpen(false);
                  if (!isCurrent) onSwitch(option.gameId);
                }}
                className={[
                  'flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-sm',
                  collapsed ? 'justify-center px-0' : '',
                  isCurrent
                    ? 'bg-primary text-primary-foreground shadow-sm'
                    : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                ].join(' ')}
              >
                {!collapsed && <span className="min-w-0 flex-1 truncate">{option.label}</span>}
                {collapsed && <span aria-hidden="true">{option.gameId === MINECRAFT_GAME_ID ? 'M' : option.label.charAt(0)}</span>}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
