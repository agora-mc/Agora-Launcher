import { describe, expect, it } from 'vitest';
import type { Mo2PlanDto } from './api';
import { planBlockReason, plannedModsLabel } from './planRules';

function plan(overrides: Partial<Mo2PlanDto> = {}): Mo2PlanDto {
  return {
    setup: 'D:/Setups/MO2/ModOrganizer.ini',
    profile: 'Default',
    mo2GameName: 'Skyrim Special Edition',
    gameId: 'skyrim-se',
    installLocation: 'D:/Games/Skyrim Special Edition',
    store: 'steam',
    instanceName: 'Default (MO2 import)',
    mods: [
      {
        line: 1,
        folder: 'SkyUI',
        enabled: true,
        files: 12,
        bytes: 1024,
        provenance: 'claims Nexus mod 3863, not verified',
        pluginFiles: ['SkyUI_SE.esp'],
        metaIniExcluded: true,
      },
    ],
    modsEnabled: 1,
    modsDisabled: 0,
    files: 12,
    bytes: 1024,
    metaIniExcluded: 1,
    separators: 0,
    unmanaged: [],
    problems: [],
    overwrite: { present: false, files: 0, bytes: 0, generated: [], restFiles: 0 },
    plugins: { kept: true, listed: 1, active: 1, locked: [], implicitSkipped: [], notes: [] },
    localSettings: false,
    inis: [],
    saves: { local: false, files: 0, bytes: 0, note: '' },
    warnings: [],
    ...overrides,
  };
}

describe('planBlockReason', () => {
  it('lets a plan for this game through', () => {
    expect(planBlockReason(plan(), 'skyrim-se')).toBeNull();
  });

  it('refuses a plan for another game by naming both games', () => {
    const reason = planBlockReason(plan({ gameId: 'fallout-4', mo2GameName: 'Fallout 4' }), 'skyrim-se');
    expect(reason).toContain('Fallout 4');
    expect(reason).toContain('fallout-4');
  });

  it('refuses a plan with no mods, so an empty import is never offered', () => {
    const reason = planBlockReason(plan({ mods: [] }), 'skyrim-se');
    expect(reason).toContain('nothing to bring in');
  });

  it('treats a game id that differs only by case as another game', () => {
    expect(planBlockReason(plan(), 'Skyrim-SE')).not.toBeNull();
  });
});

describe('plannedModsLabel', () => {
  it('uses the singular for one mod and the plural otherwise', () => {
    expect(plannedModsLabel(plan({ modsEnabled: 1, modsDisabled: 0 }))).toBe('1 enabled mod, 0 disabled mods');
    expect(plannedModsLabel(plan({ modsEnabled: 3, modsDisabled: 1 }))).toBe('3 enabled mods, 1 disabled mod');
  });
});
