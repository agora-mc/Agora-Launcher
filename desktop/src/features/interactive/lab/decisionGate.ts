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
