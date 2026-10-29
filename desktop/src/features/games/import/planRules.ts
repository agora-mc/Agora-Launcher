/**
 * The checks the Import screen makes on a plan before it offers "Import". Kept apart from the
 * component so they can be tested without a window.
 */

import type { Mo2PlanDto } from './api';

/**
 * Why this plan cannot be imported into `gameId`, or `null` when it can. Importing a setup into
 * the wrong game would store its mods under that game, so a plan for another game is refused by
 * name, never imported quietly.
 */
export function planBlockReason(plan: Mo2PlanDto, gameId: string): string | null {
  if (plan.gameId !== gameId) {
    return `This setup is for ${plan.mo2GameName} (${plan.gameId}), not this game. Open that game's Import tab to bring it in.`;
  }
  if (plan.mods.length === 0) {
    return 'The profile lists no mods Agora can import, so there is nothing to bring in.';
  }
  return null;
}

/** The mods a plan imports, enabled and disabled, in the order MO2 lists them. */
export function plannedModsLabel(plan: Mo2PlanDto): string {
  const enabled = plan.modsEnabled === 1 ? '1 enabled mod' : `${plan.modsEnabled} enabled mods`;
  const disabled = plan.modsDisabled === 1 ? '1 disabled mod' : `${plan.modsDisabled} disabled mods`;
  return `${enabled}, ${disabled}`;
}
