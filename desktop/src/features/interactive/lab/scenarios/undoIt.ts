/**
 * Undo It scenario: recovery points, scope, and the world/save boundary.
 *
 * Sol-0 contract: `docs/interactive/LESSON_MAP.md` §6. Snapshots and Last
 * Known Good are scoped return points. Worlds/saves are protected only when
 * the selected snapshot's scope explicitly includes them; restore is a
 * serious, confirmed operation with an undo return point. Everything here is
 * authored, deterministic, and namespaced `lab:undo:*`.
 */

import type {
  VisualId,
  VisualSnapshot,
  VisualScene,
} from '../../domain/models';
import type { VisualIntent } from '../../domain/intents';
import type {
  LabCheckpoint,
  LabDecision,
  LabDecisionRequest,
  LabLessonState,
  LabReduction,
  LabScenario,
} from '../scenarioTypes';
import { simId } from '../scenarioTypes';

const NS = 'undo';

const SNAPSHOTS: VisualSnapshot[] = [
  {
    id: simId(NS, 'snap', 'auto'),
    label: 'Automatic return point',
    createdAt: 'Today, 09:00',
    role: 'automatic',
    sizeLabel: 'Fast',
    changeSummary: { added: 1, changed: 2, removed: 0 },
    protects: ['mods', 'config', 'other-instance-files'],
    worldProtection: 'not-included',
    availability: 'available',
  },
  {
    id: simId(NS, 'snap', 'lkg'),
    label: 'Last known good',
    createdAt: 'Yesterday, 18:30',
    role: 'current-known-good',
    sizeLabel: 'Medium',
    changeSummary: { added: 2, changed: 1, removed: 1 },
    protects: ['mods', 'config', 'other-instance-files'],
    worldProtection: 'not-included',
    availability: 'available',
  },
  {
    id: simId(NS, 'snap', 'manual'),
    label: 'Weekend manual snapshot',
    createdAt: '3 days ago',
    role: 'manual',
    sizeLabel: 'Large',
    changeSummary: { added: 3, changed: 1, removed: 2 },
    protects: ['mods', 'config', 'worlds', 'other-instance-files'],
    worldProtection: 'included',
    availability: 'available',
  },
];

export interface UndoItScene extends VisualScene {
  snapshots: VisualSnapshot[];
  currentLabel: string;
  processRunning: boolean;
  compared: boolean;
  comparedSnapshotId: VisualId | null;
  pendingSnapshotId: VisualId | null;
  restored: boolean;
  undoPointCreated: boolean;
  /** Player-facing restore outcome shown after confirmation. */
  restoredSummary: string | null;
}

function baseUndoScene(): UndoItScene {
  return {
    source: { kind: 'simulation', scenarioId: NS, scenarioVersion: 1 },
    content: [],
    relationships: [],
    findings: [],
    proposals: [],
    snapshots: SNAPSHOTS,
    currentLabel: 'Current state — a change made things worse',
    processRunning: true,
    compared: false,
    comparedSnapshotId: null,
    pendingSnapshotId: null,
    restored: false,
    undoPointCreated: false,
    restoredSummary: null,
  };
}

function canonicalScene(checkpoint: number): UndoItScene {
  const scene = baseUndoScene();
  if (checkpoint >= 1) {
    scene.compared = true;
    scene.comparedSnapshotId = simId(NS, 'snap', 'lkg');
  }
  if (checkpoint >= 2) {
    scene.compared = true;
    scene.comparedSnapshotId = simId(NS, 'snap', 'manual');
    scene.pendingSnapshotId = simId(NS, 'snap', 'manual');
  }
  if (checkpoint >= 3) {
    scene.compared = true;
    scene.comparedSnapshotId = simId(NS, 'snap', 'manual');
    scene.pendingSnapshotId = simId(NS, 'snap', 'manual');
    scene.processRunning = false;
  }
  return scene;
}

function snapshotLabel(scene: UndoItScene, id: VisualId | null): string {
  const snapshot = scene.snapshots.find((candidate) => candidate.id === id);
  return snapshot ? snapshot.label : 'Unknown return point';
}

function decisionsFor(state: LabLessonState<UndoItScene>): LabDecision[] {
  const scene = state.scene;
  if (state.checkpoint === 0) {
    return [
      { id: 'compare-lkg', label: 'Compare Last known good' },
      { id: 'compare-auto', label: 'Compare Automatic return point' },
      { id: 'compare-manual', label: 'Compare Weekend manual snapshot' },
    ];
  }
  if (state.checkpoint === 1) {
    return [
      { id: 'restore-auto', label: 'Restore Automatic return point (worlds not included)' },
      { id: 'restore-manual', label: 'Restore Weekend manual snapshot (worlds included)' },
    ];
  }
  if (state.checkpoint === 2) {
    return [
      { id: 'try-restore-now', label: 'Try restore now' },
      { id: 'stop-process', label: 'Stop the simulated instance' },
    ];
  }
  if (state.checkpoint === 3) {
    const pendingLabel = scene.pendingSnapshotId ? snapshotLabel(scene, scene.pendingSnapshotId) : 'the selected return point';
    const pending = scene.pendingSnapshotId
      ? scene.snapshots.find((candidate) => candidate.id === scene.pendingSnapshotId)
      : undefined;
    const worldsCopy = pending?.worldProtection === 'included'
      ? 'This return point includes worlds/saves, so they are restored with it.'
      : 'This return point does NOT include worlds/saves, so your worlds stay untouched.';
    return [
      {
        id: 'confirm-restore',
        label: `Restore: ${pendingLabel}`,
        danger: true,
        confirmTitle: 'Confirm restore',
        confirmBody: `This restores the simulated instance to "${pendingLabel}" and creates a pre-restore undo return point. ${worldsCopy}`,
      },
      { id: 'cancel-restore', label: 'Cancel restore' },
    ];
  }
  return [];
}

const checkpoints: LabCheckpoint<UndoItScene>[] = [
  {
    id: 'explore',
    goal: 'Explore the return points and see what each protects.',
    expectedModel: 'Return points are scoped; worlds are protected only when the scope includes them.',
    decisionsFor,
  },
  {
    id: 'scope',
    goal: 'Choose the return point that protects what you need.',
    expectedModel: 'A fast automatic point may not protect your worlds — scope matters.',
    decisionsFor,
  },
  {
    id: 'process',
    goal: 'Restore while the instance is running is blocked. Stop it first.',
    expectedModel: 'A running instance cannot be restored.',
    decisionsFor,
  },
  {
    id: 'confirm',
    goal: 'Seriously confirm the restore. A pre-restore undo point is created.',
    expectedModel: 'Restore is consequential: compare, confirm, and keep an undo point.',
    decisionsFor,
  },
];

function reduce(
  state: LabLessonState<UndoItScene>,
  event: { kind: 'decision'; decisionId: string },
): LabReduction<UndoItScene> {
  const scene = state.scene;
  const { decisionId } = event;

  const compareIds = new Set(['compare-lkg', 'compare-auto', 'compare-manual']);
  if (compareIds.has(decisionId)) {
    const targetId =
      decisionId === 'compare-lkg'
        ? simId(NS, 'snap', 'lkg')
        : decisionId === 'compare-auto'
          ? simId(NS, 'snap', 'auto')
          : simId(NS, 'snap', 'manual');
    const snapshot = scene.snapshots.find((candidate) => candidate.id === targetId);
    const next = { ...scene, compared: true, comparedSnapshotId: targetId };
    const worlds = snapshot?.worldProtection === 'included' ? 'Worlds are included.' : 'Worlds are NOT included in this return point.';
    const feedback = scene.compared
      ? { tone: 'info' as const, message: `Now comparing ${snapshot?.label}. ${worlds}` }
      : { tone: 'success' as const, message: `${snapshot?.label} compared. ${worlds}` };
    return {
      state: { ...state, checkpoint: Math.max(state.checkpoint, 1), scene: next, lastFeedback: null },
      feedback,
    };
  }

  if (decisionId === 'restore-auto' || decisionId === 'restore-manual') {
    const targetId = decisionId === 'restore-auto' ? simId(NS, 'snap', 'auto') : simId(NS, 'snap', 'manual');
    const snapshot = scene.snapshots.find((candidate) => candidate.id === targetId);
    const next = { ...scene, pendingSnapshotId: targetId, comparedSnapshotId: targetId, compared: true };
    return {
      state: { ...state, checkpoint: 2, scene: next, lastFeedback: null },
      feedback: {
        tone: snapshot?.worldProtection === 'included' ? 'success' : 'caution',
        message: snapshot?.worldProtection === 'included'
          ? `${snapshot?.label} selected — it includes worlds. Next: the instance is running.`
          : `${snapshot?.label} selected — it does NOT include worlds. Next: the instance is running.`,
      },
    };
  }

  if (decisionId === 'try-restore-now' && scene.processRunning) {
    return {
      state: { ...state, lastFeedback: null },
      feedback: {
        tone: 'blocked',
        message: 'Blocked: the instance is running. Stop the simulated instance before restoring.',
      },
    };
  }

  if (decisionId === 'stop-process' && scene.processRunning) {
    const next = { ...scene, processRunning: false };
    return {
      state: { ...state, checkpoint: 3, scene: next, lastFeedback: null },
      feedback: { tone: 'success', message: 'Simulated instance stopped. Restore is now available.' },
    };
  }

  if (decisionId === 'confirm-restore' && !scene.restored) {
    const restoredLabel = snapshotLabel(scene, scene.pendingSnapshotId);
    const pending = scene.pendingSnapshotId
      ? scene.snapshots.find((candidate) => candidate.id === scene.pendingSnapshotId)
      : undefined;
    const undoSnapshot: VisualSnapshot = {
      id: simId(NS, 'snap', 'undo'),
      label: 'Undo restore point',
      createdAt: 'Just now',
      role: 'undo-restore',
      sizeLabel: 'Small',
      protects: ['mods', 'config', 'other-instance-files'],
      worldProtection: 'not-included',
      availability: 'available',
    };
    const next = {
      ...scene,
      restored: true,
      undoPointCreated: true,
      processRunning: false,
      snapshots: [...scene.snapshots, undoSnapshot],
      currentLabel: `Current state — restored to "${restoredLabel}"`,
      restoredSummary:
        pending?.worldProtection === 'included'
          ? `Instance files and worlds were restored to "${restoredLabel}". A new undo return point now protects the pre-restore state.`
          : `Instance files were restored to "${restoredLabel}". Worlds were NOT included in this return point, so your worlds were not touched.`,
    };
    return {
      state: { ...state, status: 'complete', scene: next, lastFeedback: null },
      feedback: {
        tone: 'success',
        message: `Simulated restore complete with a new undo return point. ${restoredLabel} was restored.`,
      },
    };
  }

  if (decisionId === 'cancel-restore') {
    return {
      state: { ...state, lastFeedback: null },
      feedback: { tone: 'info', message: 'Restore cancelled. Nothing changed; current state stays as-is.' },
    };
  }

  return { state, feedback: null };
}

export const undoItScenario: LabScenario<UndoItScene> = {
  id: NS,
  version: 1,
  title: 'Undo It',
  shortTitle: 'Undo It',
  description: 'Pick a safe return point and confirm a restore.',
  iconLabel: '↩️',
  checkpoints,
  guideTopics: ['snapshots-loadouts'],
  realDestinations: [
    { type: 'tab', tab: 'instances' },
  ],
  initialScene: canonicalScene,
  reduce,
  intentToDecision(_scene, intent: VisualIntent): LabDecisionRequest | null {
    if (intent.kind === 'preview-snapshot') {
      const map: Record<string, string> = {
        [simId(NS, 'snap', 'auto')]: 'compare-auto',
        [simId(NS, 'snap', 'lkg')]: 'compare-lkg',
        [simId(NS, 'snap', 'manual')]: 'compare-manual',
      };
      const decisionId = map[intent.snapshotId];
      if (decisionId) return { decisionId };
    }
    if (intent.kind === 'request-snapshot-restore') {
      return { decisionId: 'try-restore-now' };
    }
    return null;
  },
  successPredicate(state) {
    return state.status === 'complete';
  },
  completionMessage: 'You completed the practice: you chose a scoped return point, stopped the process, confirmed seriously, and kept an undo point.',
};
