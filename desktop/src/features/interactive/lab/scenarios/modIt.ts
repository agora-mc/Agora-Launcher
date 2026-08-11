/**
 * Mod It scenario: dependencies, conflicts, and a reviewed proposed change.
 *
 * Sol-0 contract: `docs/interactive/LESSON_MAP.md` §3. Installing content is a
 * planned change: requirements, recommendations, conflicts, and a recovery
 * point are reviewed together before current state changes. Everything here
 * is authored, deterministic, and namespaced `lab:mod:*`.
 */

import type {
  VisualContentNode,
  VisualProposal,
  VisualRelationship,
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

const NS = 'mod';

const IDS = {
  betterCaves: simId(NS, 'better-caves'),
  coreLib: simId(NS, 'core-lib'),
  niceTextures: simId(NS, 'nice-textures'),
  terrainOverhaul: simId(NS, 'terrain-overhaul'),
  proposal: simId(NS, 'proposal'),
};

export interface ModItScene extends VisualScene {
  staged: boolean;
  requiredAdded: boolean;
  optionalAdded: boolean;
  optionalSkipped: boolean;
  conflictResolved: boolean;
  conflictVisible: boolean;
  applied: boolean;
}

function relationships(scene: ModItScene): VisualRelationship[] {
  const requires: VisualRelationship = {
    id: simId(NS, 'rel', 'requires-core'),
    fromId: IDS.betterCaves,
    toId: IDS.coreLib,
    kind: 'requires',
    state: scene.requiredAdded ? 'satisfied' : 'missing',
    importance: 'required',
    explanation: scene.requiredAdded ? 'Core Lib satisfies the requirement.' : 'BetterCaves needs Core Lib to work.',
  };
  const recommends: VisualRelationship = {
    id: simId(NS, 'rel', 'recommends-textures'),
    fromId: IDS.betterCaves,
    toId: IDS.niceTextures,
    kind: 'recommends',
    state: scene.optionalAdded ? 'satisfied' : 'indeterminate',
    importance: 'recommended',
    explanation: 'Nice Textures is optional — BetterCaves works without it.',
  };
  const conflict: VisualRelationship = {
    id: simId(NS, 'rel', 'conflicts-terrain'),
    fromId: IDS.betterCaves,
    toId: IDS.terrainOverhaul,
    kind: 'conflicts-with',
    state: scene.conflictResolved ? 'satisfied' : 'conflicting',
    importance: 'required',
    explanation: 'BetterCaves and Terrain Overhaul both change world generation.',
    affectedCount: 1,
  };
  if (!scene.conflictVisible) {
    return [requires, recommends];
  }
  return [requires, recommends, conflict];
}

function proposals(scene: ModItScene): VisualProposal[] {
  if (!scene.staged || scene.applied) return [];
  const proposal: VisualProposal = {
    id: IDS.proposal,
    intent: { kind: 'propose-install', contentId: IDS.betterCaves },
    phase: 'proposed',
    title: 'Install BetterCaves',
    summary: scene.conflictResolved
      ? 'Adds BetterCaves, removes Terrain Overhaul, adds Core Lib' + (scene.optionalAdded ? ' and Nice Textures' : '') + '.'
      : 'Adds BetterCaves, adds Core Lib' + (scene.optionalAdded ? ' and Nice Textures' : '') + ' — conflict with Terrain Overhaul unresolved.',
    destructive: scene.conflictResolved,
  };
  return [proposal];
}

function contentNodes(scene: ModItScene): VisualContentNode[] {
  return [
    {
      id: IDS.betterCaves,
      name: 'BetterCaves',
      kind: 'mod' as const,
      presence: scene.applied
        ? { current: 'installed' }
        : { current: 'not-installed', proposed: scene.staged ? 'installed' : undefined },
      enabled: { current: true },
      health: scene.conflictVisible && !scene.conflictResolved ? 'blocked' : 'healthy',
      relationshipSummary: {
        requiredBy: 0,
        requires: 1,
        conflicts: scene.conflictVisible ? 1 : 0,
      },
      availability: 'available',
    },
    {
      id: IDS.coreLib,
      name: 'Core Lib',
      kind: 'mod' as const,
      presence: scene.applied
        ? { current: 'installed' }
        : { current: 'not-installed', proposed: scene.requiredAdded ? 'installed' : undefined },
      enabled: { current: true },
      health: 'healthy',
      relationshipSummary: { requiredBy: scene.staged ? 1 : 0, requires: 0, conflicts: 0 },
      availability: 'available',
    },
    {
      id: IDS.niceTextures,
      name: 'Nice Textures',
      kind: 'resource-pack' as const,
      presence: scene.applied
        ? { current: scene.optionalAdded ? 'installed' : 'not-installed' }
        : { current: 'not-installed', proposed: scene.optionalAdded ? 'installed' : undefined },
      enabled: { current: true },
      health: 'healthy',
      relationshipSummary: { requiredBy: 0, requires: 0, conflicts: 0 },
      availability: 'available',
    },
    {
      id: IDS.terrainOverhaul,
      name: 'Terrain Overhaul',
      kind: 'mod' as const,
      presence: scene.applied
        ? { current: 'not-installed' }
        : { current: 'installed', proposed: scene.conflictResolved ? 'not-installed' : undefined },
      enabled: { current: true },
      health: scene.conflictVisible && !scene.conflictResolved ? 'blocked' : 'healthy',
      relationshipSummary: { requiredBy: 0, requires: 0, conflicts: scene.conflictVisible ? 1 : 0 },
      availability: 'available',
    },
  ];
}

function canonicalScene(checkpoint: number): ModItScene {
  const scene: ModItScene = {
    source: { kind: 'simulation', scenarioId: NS, scenarioVersion: 1 },
    content: [],
    relationships: [],
    findings: [],
    proposals: [],
    staged: false,
    requiredAdded: false,
    optionalAdded: false,
    optionalSkipped: false,
    conflictResolved: false,
    conflictVisible: false,
    applied: false,
  };
  if (checkpoint >= 1) scene.staged = true;
  if (checkpoint >= 2) {
    // Only decisions REQUIRED to reach this checkpoint are reconstructed. The
    // optional recommendation is deliberately left undecided: it can be either
    // included or skipped at checkpoint 2, and `decisionsFor` re-offers both.
    // Previously this fabricated `optionalAdded = true`, so resuming silently
    // reversed a player's "Skip optional textures" and grew their reviewed plan
    // by an item they had declined (T6-2).
    scene.requiredAdded = true;
    scene.conflictVisible = true;
  }
  if (checkpoint >= 3) {
    scene.conflictResolved = true;
    scene.conflictVisible = true;
  }
  scene.content = contentNodes(scene);
  scene.relationships = relationships(scene);
  scene.proposals = proposals(scene);
  return scene;
}

function decisionsFor(state: LabLessonState<ModItScene>): LabDecision[] {
  const scene = state.scene;
  if (state.checkpoint === 0) {
    return [{ id: 'stage-better-caves', label: 'Stage BetterCaves' }];
  }
  if (state.checkpoint === 1) {
    const decisions: LabDecision[] = [];
    if (!scene.requiredAdded) {
      decisions.push({ id: 'add-core-lib', label: 'Snap required: Core Lib' });
    }
    if (!scene.optionalAdded && !scene.optionalSkipped) {
      decisions.push({ id: 'include-nice-textures', label: 'Include optional: Nice Textures' });
      decisions.push({ id: 'skip-optional', label: 'Skip optional textures' });
    }
    return decisions;
  }
  if (state.checkpoint === 2) {
    const decisions: LabDecision[] = [];
    if (!scene.optionalAdded && !scene.optionalSkipped) {
      decisions.push({ id: 'include-nice-textures', label: 'Include optional: Nice Textures' });
      decisions.push({ id: 'skip-optional', label: 'Skip optional textures' });
    }
    decisions.push({
      id: 'replace-terrain-overhaul',
      label: 'Replace Terrain Overhaul',
      danger: true,
      confirmTitle: 'Confirm replacement',
      // Names the ACTUAL staged change: a static body under-described the plan
      // whenever the optional recommendation had been included.
      confirmBody: `This removes Terrain Overhaul from the simulated instance and installs BetterCaves, Core Lib${
        scene.optionalAdded ? ', and Nice Textures' : ''
      } instead. A simulated return point is created before applying. Worlds are not part of this change.`,
    });
    decisions.push({ id: 'keep-current', label: 'Keep current content' });
    return decisions;
  }
  if (state.checkpoint === 3) {
    return [
      { id: 'apply-plan', label: 'Apply simulated plan' },
      { id: 'reset', label: 'Start over' },
    ];
  }
  return [];
}

const checkpoints: LabCheckpoint<ModItScene>[] = [
  {
    id: 'stage',
    goal: 'Stage the mod you want on the graph.',
    expectedModel: 'A change starts as a proposal, not a direct install.',
    decisionsFor,
  },
  {
    id: 'requirements',
    goal: 'Snap the required dependency and decide on the optional recommendation.',
    expectedModel: 'Required relationships must be satisfied; recommendations are deliberate choices.',
    decisionsFor,
  },
  {
    id: 'conflict',
    goal: 'A conflict surfaced. Choose how to resolve it before applying.',
    expectedModel: 'A blocking conflict must be resolved by keeping current content or changing the proposal.',
    decisionsFor,
  },
  {
    id: 'review',
    goal: 'Review current versus proposed, then apply the simulated plan.',
    expectedModel: 'A planned change is reviewed together before current state changes.',
    decisionsFor,
  },
];

function reduce(
  state: LabLessonState<ModItScene>,
  event: { kind: 'decision'; decisionId: string },
): LabReduction<ModItScene> {
  const scene = state.scene;
  const { decisionId } = event;

  if (decisionId === 'stage-better-caves' && !scene.staged) {
    const next = { ...scene, staged: true };
    next.content = contentNodes(next);
    next.relationships = relationships(next);
    next.proposals = proposals(next);
    return {
      state: { ...state, checkpoint: 1, scene: next, lastFeedback: null },
      feedback: { tone: 'success', message: 'BetterCaves is staged as a proposal. It needs Core Lib.' },
    };
  }

  if (decisionId === 'add-core-lib' && !scene.requiredAdded) {
    const next = { ...scene, requiredAdded: true };
    const done = next.optionalAdded || next.optionalSkipped;
    // Entering the conflict checkpoint must REVEAL the conflict. Without this
    // the goal said "A conflict surfaced" while the graph showed nothing, and
    // the conflict existed only in staging-dock prose (T6-1). Derived arrays
    // are rebuilt AFTER this flag so the relationship is included.
    if (done) next.conflictVisible = true;
    next.content = contentNodes(next);
    next.relationships = relationships(next);
    next.proposals = proposals(next);
    return {
      state: { ...state, checkpoint: done ? 2 : 1, scene: next, lastFeedback: null },
      feedback: { tone: 'success', message: 'Core Lib snaps into the required socket.' },
    };
  }

  if (decisionId === 'include-nice-textures' && !scene.optionalAdded && !scene.optionalSkipped) {
    const next = { ...scene, optionalAdded: true };
    const done = next.requiredAdded;
    if (done) next.conflictVisible = true;
    next.content = contentNodes(next);
    next.relationships = relationships(next);
    next.proposals = proposals(next);
    return {
      state: { ...state, checkpoint: done ? 2 : 1, scene: next, lastFeedback: null },
      feedback: { tone: 'info', message: 'Nice Textures included as a recommendation — optional, not required.' },
    };
  }

  if (decisionId === 'skip-optional' && !scene.optionalAdded && !scene.optionalSkipped) {
    const next = { ...scene, optionalSkipped: true };
    const done = next.requiredAdded;
    if (done) next.conflictVisible = true;
    next.content = contentNodes(next);
    next.relationships = relationships(next);
    next.proposals = proposals(next);
    return {
      state: { ...state, checkpoint: done ? 2 : 1, scene: next, lastFeedback: null },
      feedback: { tone: 'info', message: 'Optional textures skipped. The change still works without them.' },
    };
  }

  if (decisionId === 'replace-terrain-overhaul' && !scene.conflictResolved) {
    const next = { ...scene, conflictResolved: true };
    next.content = contentNodes(next);
    next.relationships = relationships(next);
    next.proposals = proposals(next);
    return {
      state: { ...state, checkpoint: 3, scene: next, lastFeedback: null },
      feedback: {
        tone: 'success',
        message: 'Terrain Overhaul marked for removal. The conflict is resolved — review and apply.',
      },
    };
  }

  if (decisionId === 'keep-current') {
    return {
      state: { ...state, lastFeedback: null },
      feedback: {
        tone: 'caution',
        message: 'Keeping current content leaves the conflict unresolved. Choose Replace to proceed.',
      },
    };
  }

  if (decisionId === 'apply-plan' && scene.conflictResolved && scene.requiredAdded) {
    const next = { ...scene, applied: true };
    next.content = contentNodes(next);
    next.relationships = relationships(next);
    next.proposals = proposals(next);
    return {
      state: { ...state, status: 'complete', scene: next, lastFeedback: null },
      feedback: {
        tone: 'success',
        message: 'Simulated plan applied: BetterCaves installed, dependency satisfied, conflict removed.',
      },
    };
  }

  return { state, feedback: null };
}

export const modItScenario: LabScenario<ModItScene> = {
  id: NS,
  version: 1,
  title: 'Mod It',
  shortTitle: 'Mod It',
  description: 'Plan a change: dependencies, conflicts, and a reviewed proposal.',
  iconLabel: '🧩',
  checkpoints,
  guideTopics: ['install-update', 'modding-foundations'],
  realDestinations: [
    { type: 'tab', tab: 'browse' },
    { type: 'tab', tab: 'instances' },
  ],
  initialScene: canonicalScene,
  reduce,
  intentToDecision(scene, intent: VisualIntent): LabDecisionRequest | null {
    // The review dock proposes the apply decision; the shell gate authorizes it.
    if (intent.kind === 'review-staged-changes' && scene.staged && !scene.applied) {
      return { decisionId: 'apply-plan' };
    }
    if (intent.kind === 'propose-install' && intent.contentId === IDS.betterCaves && !scene.staged) {
      return { decisionId: 'stage-better-caves' };
    }
    if (intent.kind === 'propose-remove' && intent.contentId === IDS.terrainOverhaul && !scene.conflictResolved) {
      return { decisionId: 'replace-terrain-overhaul' };
    }
    return null;
  },
  successPredicate(state) {
    return state.status === 'complete';
  },
  completionMessage: 'You completed the practice: requirements satisfied, conflict resolved, and the plan reviewed before applying.',
};
