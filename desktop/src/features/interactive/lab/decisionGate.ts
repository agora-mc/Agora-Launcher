/**
 * Single shell-owned Lab decision gate.
 *
 * SOL-1 BLOCKER 2: action-list buttons and visual-intent routes must reach the
 * same safety gate. The gate resolves a decision id against the CURRENT
 * checkpoint's `decisionsFor(state)` at request time, rejects unavailable or
 * disabled decisions, and returns `confirm` for dangerous ones. Authorization
 * lives here (plus the confirm-time revalidation in the shell), never inside
 * `intentToDecision`.
 *
 * This module is pure: no React, Tauri, or app-layer imports.
 */

import type { LabDecision, LabLessonState, LabScenario } from './scenarioTypes';

export type DecisionGateResult =
  | { status: 'dispatch'; decision: LabDecision }
  | { status: 'confirm'; decision: LabDecision }
  | { status: 'rejected'; reason: string };

export function resolveDecisionGate<Scene>(
  scenario: LabScenario<Scene>,
  state: LabLessonState<Scene>,
  decisionId: string,
): DecisionGateResult {
  const checkpoint = scenario.checkpoints[state.checkpoint];
  const decisions = checkpoint ? checkpoint.decisionsFor(state) : [];
  const decision = decisions.find((candidate) => candidate.id === decisionId);
  if (!decision) {
    return { status: 'rejected', reason: 'That action is not available at this step.' };
  }
  if (decision.disabledReason) {
    return { status: 'rejected', reason: decision.disabledReason };
  }
  if (decision.danger) {
    return { status: 'confirm', decision };
  }
  return { status: 'dispatch', decision };
}

/**
 * The consequence the player reviewed when the confirmation dialog opened.
 * Confirm-time revalidation compares the CURRENT decision's consequence against
 * this snapshot so the player confirms exactly what they were shown.
 */
export interface DecisionConsequence {
  danger: boolean;
  confirmTitle?: string;
  confirmBody?: string;
}

/**
 * Revalidate an already-confirmed decision immediately before dispatch
 * (SOL-1 BLOCKER 3 / residual BLOCKER B). Danger is already accepted at this
 * point — the player is confirming — but the decision must still exist for the
 * CURRENT checkpoint, must not be disabled, and its consequence (danger /
 * confirmTitle / confirmBody) must be unchanged from what the player reviewed.
 */
export function revalidateDecisionForConfirm<Scene>(
  scenario: LabScenario<Scene>,
  state: LabLessonState<Scene>,
  decisionId: string,
  reviewed: DecisionConsequence,
): { valid: true } | { valid: false; reason: string } {
  const checkpoint = scenario.checkpoints[state.checkpoint];
  const decision = checkpoint?.decisionsFor(state).find((candidate) => candidate.id === decisionId);
  if (!decision) {
    return { valid: false, reason: 'That action is no longer available at this step.' };
  }
  if (decision.disabledReason) {
    return { valid: false, reason: decision.disabledReason };
  }
  const current: DecisionConsequence = {
    danger: decision.danger === true,
    confirmTitle: decision.confirmTitle,
    confirmBody: decision.confirmBody,
  };
  if (
    current.danger !== reviewed.danger
    || current.confirmTitle !== reviewed.confirmTitle
    || current.confirmBody !== reviewed.confirmBody
  ) {
    return { valid: false, reason: 'The consequences of this action changed. Please review it again.' };
  }
  return { valid: true };
}
