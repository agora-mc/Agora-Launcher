/**
 * Lab scenario contract and lesson types.
 *
 * Sol-0 contract: `docs/interactive/MASTER_ARCHITECTURE.md` §6 and
 * `docs/interactive/LESSON_MAP.md`. The lesson engine is a deterministic
 * reducer over authored scenario data; animation observes reducer events and
 * never advances the lesson.
 *
 * Lab code must not import Tauri, `lib/tauri`, `live/`, or current operation
 * components — enforced by `scripts/check-interactive-boundaries.mjs`.
 */

import type { VisualIntent, StandardDestination, GuideTopicId } from '../domain/intents';
import type { VisualId, VisualScene } from '../domain/models';

/** A named action the player can take at a checkpoint. */
export interface LabDecision {
  id: string;
  /** Short button label. */
  label: string;
  /** Distinct command name for keyboard/screen readers when different from label. */
  keyboardLabel?: string;
  /** Requires a serious confirmation in the shell before dispatch. */
  danger?: boolean;
  /** When set, the decision is disabled with this persistent reason text. */
  disabledReason?: string;
  /** Accessible name / heading for the serious-confirmation dialog (danger only). */
  confirmTitle?: string;
  /**
   * Player-facing consequence copy for the serious-confirmation dialog
   * (danger only). Must name the exact affected content/consequence. Only a
   * real restore may describe restoring worlds/scope.
   */
  confirmBody?: string;
}

export type FeedbackTone = 'success' | 'caution' | 'info' | 'blocked';

export interface FeedbackEvent {
  tone: FeedbackTone;
  message: string;
}

export type LabStatus = 'in-progress' | 'complete';

export interface LabLessonState<Scene = VisualScene> {
  scenarioId: string;
  scenarioVersion: number;
  /** Current checkpoint index. */
  checkpoint: number;
  /** The simulated scene at this stage. */
  scene: Scene;
  status: LabStatus;
  lastFeedback: FeedbackEvent | null;
}

export interface LabReduction<Scene = VisualScene> {
  state: LabLessonState<Scene>;
  feedback: FeedbackEvent | null;
}

export type LabEvent =
  | { kind: 'decision'; decisionId: string }
  | { kind: 'reset' }
  | { kind: 'exit' };

/**
 * A proposed decision identified by a visual intent. This is NOT an
 * authorization to dispatch — the shell-owned decision gate resolves and
 * authorizes it (SOL-1 BLOCKER 2).
 */
export type LabDecisionRequest = { decisionId: string };

export interface LabCheckpoint<Scene = VisualScene> {
  id: string;
  /** One plain-language goal. */
  goal: string;
  /** The mental model this checkpoint teaches (one short takeaway). */
  expectedModel: string;
  /** Decisions available at this checkpoint (may depend on state). */
  decisionsFor(state: LabLessonState<Scene>): LabDecision[];
}

/**
 * An authored Lab adventure. Scenes are deterministic and IDs are namespaced
 * (`lab:<scenario>:<item>`). `initialScene(checkpoint)` returns the canonical
 * scene for a checkpoint so resume can restore the last safe stage exactly.
 */
export interface LabScenario<Scene = VisualScene> {
  id: string;
  version: number;
  title: string;
  shortTitle: string;
  description: string;
  iconLabel: string;
  checkpoints: LabCheckpoint<Scene>[];
  /** Field Guide destinations (validated GuideTopicIds). */
  guideTopics: GuideTopicId[];
  /** Real Agora destinations shown after leaving the simulation. */
  realDestinations: StandardDestination[];
  initialScene(checkpoint?: number): Scene;
  /**
   * Deterministic reducer for a player decision.
   * @returns the next state and any feedback; may stay on the same checkpoint.
   */
  reduce(
    state: LabLessonState<Scene>,
    event: { kind: 'decision'; decisionId: string },
  ): LabReduction<Scene>;
  /**
   * Map a visual intent to a proposed decision (or null to ignore).
   * Returns a `LabDecisionRequest` — the shell decision gate owns authorization.
   */
  intentToDecision?(scene: Scene, intent: VisualIntent): LabDecisionRequest | null;
  successPredicate(state: LabLessonState<Scene>): boolean;
  completionMessage: string;
}

/** Namespaced simulated id helper: `lab:<scenario>:<part>...`. */
export function simId(scenarioId: string, ...parts: string[]): VisualId {
  return ['lab', scenarioId, ...parts].join(':');
}

export function isNamespacedLabId(id: string, scenarioId: string): boolean {
  return id.startsWith(`lab:${scenarioId}:`);
}
