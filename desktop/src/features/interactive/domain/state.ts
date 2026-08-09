/**
 * Scene and proposal state helpers.
 *
 * Sol-0 contract: `docs/interactive/DOMAIN_MODELS.md` §3 and
 * `docs/interactive/MASTER_ARCHITECTURE.md` §12 (current / proposed /
 * in-review / committed). These are pure functions over presentation models.
 *
 * This module is pure: it imports nothing from React, Tauri, or the app layer.
 */

import type { ProposalPhase, VisualProposal, VisualScene, VisualValue } from './models';

/** The four explicit scene phases. "Committed" is an outcome event. */
export type ScenePhase = 'current' | 'proposed' | 'applying' | 'committed';

export const SCENE_PHASE_ORDER: Record<ScenePhase, number> = {
  current: 0,
  proposed: 1,
  applying: 2,
  committed: 3,
};

/** Proposal phases that mean an operation surface is active or working. */
export const IN_FLIGHT_PHASES: ReadonlySet<ProposalPhase> = new Set(['in-review', 'applying']);

export function hasProposal<V>(value: VisualValue<V>): boolean {
  return value.proposed !== undefined;
}

/** The value a staged choice would become (falls back to current). */
export function stagedValue<V>(value: VisualValue<V>): V {
  return value.proposed ?? value.current;
}

export function isProposalPhase(phase: ProposalPhase): phase is ProposalPhase {
  return phase === 'proposed' || phase === 'in-review' || phase === 'applying' || phase === 'rejected';
}

/** Add or replace a proposal by id in a scene. Returns a new scene. */
export function stageProposal(scene: VisualScene, proposal: VisualProposal): VisualScene {
  const next = scene.proposals.filter((existing) => existing.id !== proposal.id);
  return { ...scene, proposals: [...next, proposal] };
}

/** Update one proposal's phase. Returns a new scene. */
export function setProposalPhase(scene: VisualScene, proposalId: string, phase: ProposalPhase): VisualScene {
  const found = scene.proposals.some((proposal) => proposal.id === proposalId);
  if (!found) return scene;
  return {
    ...scene,
    proposals: scene.proposals.map((proposal) =>
      proposal.id === proposalId ? { ...proposal, phase } : proposal,
    ),
  };
}

/** Remove a proposal (used after a fresh read / committed outcome). */
export function removeProposal(scene: VisualScene, proposalId: string): VisualScene {
  return { ...scene, proposals: scene.proposals.filter((proposal) => proposal.id !== proposalId) };
}

/** True when any proposal is in review or applying (operation active). */
export function hasProposalInFlight(scene: VisualScene): boolean {
  return scene.proposals.some((proposal) => IN_FLIGHT_PHASES.has(proposal.phase));
}

/** Find the proposal currently driving the scene, if any. */
export function activeProposal(scene: VisualScene): VisualProposal | undefined {
  return scene.proposals.find((proposal) => IN_FLIGHT_PHASES.has(proposal.phase));
}

/** A proposal is staged but not yet accepted by any operation surface. */
export function isStaged(scene: VisualScene, proposalId: string): boolean {
  return scene.proposals.some(
    (proposal) => proposal.id === proposalId && proposal.phase === 'proposed',
  );
}
