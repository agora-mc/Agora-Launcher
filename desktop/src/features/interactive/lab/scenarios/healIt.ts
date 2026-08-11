/**
 * Heal It scenario: health blockers, warnings, and recommendations.
 *
 * Sol-0 contract: `docs/interactive/LESSON_MAP.md` §4. Agora's pre-launch health
 * check separates blockers, warnings, and recommendations; loader and Java
 * choices are based on structured compatibility, not guesswork. Everything here
 * is authored, deterministic, and namespaced `lab:heal:*`. No `javafml`,
 * capability metadata, or resolver internals are exposed.
 */

import type {
  VisualHealthFinding,
  VisualLoaderCandidate,
  VisualRuntimeState,
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

const NS = 'heal';

const IDS = {
  findingLoader: simId(NS, 'finding', 'loader'),
  findingJava: simId(NS, 'finding', 'java'),
  findingMemory: simId(NS, 'finding', 'memory'),
  loaderCurrent: simId(NS, 'loader', 'current'),
  loaderFabric: simId(NS, 'loader', 'fabric'),
  loaderQuilt: simId(NS, 'loader', 'quilt'),
};

export interface HealItScene extends VisualScene {
  validated: boolean;
  blockerCleared: boolean;
  javaDecided: boolean;
  javaResolved: boolean;
  memoryDecided: boolean;
  memoryManual: boolean;
  revalidated: boolean;
  findings: VisualHealthFinding[];
  loaderCandidates: VisualLoaderCandidate[];
  runtime: VisualRuntimeState;
}

function loaderCandidates(blockerCleared: boolean): VisualLoaderCandidate[] {
  return [
    {
      id: IDS.loaderCurrent,
      family: 'Forge',
      version: '47.1',
      channel: 'stable',
      role: 'current',
      compatibility: 'incompatible',
      requirementSummary: blockerCleared ? { satisfied: 3, indeterminate: 0, failed: 0 } : { satisfied: 1, indeterminate: 0, failed: 2 },
      affectedContent: { visibleNames: ['Tweakeroo', 'Sodium'], total: 2 },
      explanation: 'This loader does not fit the mods this instance needs.',
    },
    {
      id: IDS.loaderFabric,
      family: 'Fabric',
      version: '0.15.11',
      channel: 'stable',
      role: 'recommended',
      compatibility: 'compatible',
      requirementSummary: { satisfied: 3, indeterminate: 0, failed: 0 },
      affectedContent: { visibleNames: [], total: 0 },
      explanation: 'Proven compatible with the installed mods.',
    },
    {
      id: IDS.loaderQuilt,
      family: 'Quilt',
      version: '0.25.0',
      channel: 'stable',
      role: 'alternative',
      compatibility: 'indeterminate',
      requirementSummary: { satisfied: 1, indeterminate: 2, failed: 0 },
      affectedContent: { visibleNames: ['Sodium'], total: 1 },
      explanation: 'Needs review — compatibility is not proven for this setup.',
    },
  ];
}

function findingsFor(scene: Omit<HealItScene, 'findings'>): VisualHealthFinding[] {
  const findings: VisualHealthFinding[] = [];
  if (!scene.blockerCleared) {
    findings.push({
      id: IDS.findingLoader,
      severity: 'blocker',
      title: 'The loader does not fit this instance',
      summary: 'The current loader is incompatible with two installed mods.',
      affectedIds: [],
      suggestedAction: 'Choose a proven-compatible loader.',
      structuredKind: 'loader-compatibility',
      compatibility: 'incompatible',
      reviewIntent: { kind: 'review-loader' },
    });
  }
  if (!scene.javaResolved) {
    findings.push({
      id: IDS.findingJava,
      severity: 'warning',
      title: 'Java version needs attention',
      summary: 'The selected Java is too old for this Minecraft version.',
      affectedIds: [],
      suggestedAction: 'Let Agora manage a compatible runtime, or choose one in Settings.',
      structuredKind: 'runtime',
      compatibility: 'incompatible',
    });
  }
  if (!scene.memoryDecided) {
    findings.push({
      id: IDS.findingMemory,
      severity: 'recommendation',
      title: 'Memory: automatic is recommended here',
      summary: 'Automatic memory is fine for this instance; more is not always better.',
      affectedIds: [],
      suggestedAction: 'Keep automatic, or stage a manual choice and review it.',
      structuredKind: 'runtime',
    });
  }
  return findings;
}

function runtimeFor(scene: { javaResolved: boolean; memoryDecided: boolean; memoryManual: boolean }): VisualRuntimeState {
  return {
    runtime: {
      currentLabel: scene.javaResolved ? 'Java 17 (managed)' : 'Java 8 (installed)',
      requiredJavaMajor: 17,
      compatibility: scene.javaResolved ? 'compatible' : 'incompatible',
      managedByAgora: scene.javaResolved,
    },
    memory: {
      mode: {
        current: 'automatic',
        ...(scene.memoryDecided && scene.memoryManual ? { proposed: 'manual' } : {}),
      },
      currentMiB: 2048,
      ...(scene.memoryManual ? { proposedMiB: 4096 } : {}),
      recommendedMiB: 4096,
      safeHeadroomLabel: '12 GB free of 16 GB',
      explanation: 'Agora chooses memory automatically. Manual is fine, but more memory is not always better.',
    },
    garbageCollector: { current: { mode: 'automatic' } },
    availability: 'available',
  };
}

function baseHealScene(): Omit<HealItScene, 'source' | 'content' | 'relationships' | 'proposals' | 'findings' | 'loaderCandidates' | 'runtime'> {
  return {
    validated: false,
    blockerCleared: false,
    javaDecided: false,
    javaResolved: false,
    memoryDecided: false,
    memoryManual: false,
    revalidated: false,
  };
}

function canonicalScene(checkpoint: number): HealItScene {
  const base = baseHealScene();
  const scene: HealItScene = {
    ...base,
    source: { kind: 'simulation', scenarioId: NS, scenarioVersion: 1 },
    content: [],
    relationships: [],
    proposals: [],
    findings: [],
    loaderCandidates: [],
    runtime: runtimeFor(base),
  };
  if (checkpoint >= 1) scene.validated = true;
  if (checkpoint >= 2) {
    scene.validated = true;
    scene.blockerCleared = true;
  }
  if (checkpoint >= 3) {
    scene.validated = true;
    scene.blockerCleared = true;
    scene.javaDecided = true;
    scene.javaResolved = true;
    scene.memoryDecided = true;
  }
  scene.loaderCandidates = loaderCandidates(scene.blockerCleared);
  scene.findings = findingsFor(scene);
  scene.runtime = runtimeFor(scene);
  return scene;
}

function decisionsFor(state: LabLessonState<HealItScene>): LabDecision[] {
  const scene = state.scene;
  if (state.checkpoint === 0) {
    return [{ id: 'run-validation', label: 'Run validation check' }];
  }
  if (state.checkpoint === 1) {
    return [
      { id: 'choose-fabric', label: 'Choose Fabric 0.15' },
      { id: 'choose-forge', label: 'Keep Forge 47.1' },
      { id: 'choose-quilt', label: 'Choose Quilt 0.25' },
    ];
  }
  if (state.checkpoint === 2) {
    const decisions: LabDecision[] = [];
    if (!scene.javaDecided) {
      decisions.push({ id: 'use-agora-java', label: 'Let Agora manage Java 17' });
      decisions.push({ id: 'keep-java-8', label: 'Keep Java 8' });
    }
    if (!scene.memoryDecided) {
      decisions.push({ id: 'keep-memory-auto', label: 'Keep automatic memory' });
      decisions.push({ id: 'stage-memory-manual', label: 'Stage manual 4 GB' });
    }
    return decisions;
  }
  if (state.checkpoint === 3) {
    return [{ id: 'revalidate', label: 'Re-run validation check' }];
  }
  return [];
}

const checkpoints: LabCheckpoint<HealItScene>[] = [
  {
    id: 'scan',
    goal: 'Run the pre-launch health check to see what is going on.',
    expectedModel: 'A health check separates blockers, warnings, and recommendations.',
    decisionsFor,
  },
  {
    id: 'loader',
    goal: 'A blocker stops launch. Choose a loader that fits the installed mods.',
    expectedModel: 'Proven-compatible is different from needs-review; a blocker must be resolved to launch.',
    decisionsFor,
  },
  {
    id: 'warnings',
    goal: 'Warnings need review; recommendations never block. Decide on Java and memory.',
    expectedModel: 'Warnings require a choice, recommendations are advice, and automatic memory is fine here.',
    decisionsFor,
  },
  {
    id: 'recheck',
    goal: 'Re-run the check to confirm the simulated instance is green.',
    expectedModel: 'After fixing the blocker and reviewing warnings, health is green again.',
    decisionsFor,
  },
];

function reduce(
  state: LabLessonState<HealItScene>,
  event: { kind: 'decision'; decisionId: string },
): LabReduction<HealItScene> {
  const scene = state.scene;
  const { decisionId } = event;

  if (decisionId === 'run-validation') {
    const next = { ...scene, validated: true };
    next.findings = findingsFor(next);
    next.runtime = runtimeFor(next);
    return {
      state: { ...state, checkpoint: 1, scene: next, lastFeedback: null },
      feedback: {
        tone: 'info',
        message: 'Scan complete: 1 blocker, 1 warning, 1 recommendation.',
      },
    };
  }

  if (decisionId === 'revalidate' && scene.validated && !scene.revalidated) {
    const next = { ...scene, revalidated: true };
    next.findings = findingsFor(next);
    next.loaderCandidates = loaderCandidates(next.blockerCleared);
    next.runtime = runtimeFor(next);
    const green = next.findings.length === 0;
    return {
      state: { ...state, checkpoint: 3, status: 'complete', scene: next, lastFeedback: null },
      feedback: {
        tone: green ? 'success' : 'caution',
        message: green
          ? 'Validation green: no blockers, no warnings, no unresolved recommendations.'
          : 'Validation complete: the blocker is gone, but warnings and recommendations still need your review.',
      },
    };
  }

  if (decisionId === 'choose-fabric' && !scene.blockerCleared) {
    const next = { ...scene, blockerCleared: true };
    next.findings = findingsFor(next);
    next.loaderCandidates = loaderCandidates(true);
    next.runtime = runtimeFor(next);
    return {
      state: { ...state, checkpoint: 2, scene: next, lastFeedback: null },
      feedback: { tone: 'success', message: 'Fabric is proven compatible — the blocker is cleared.' },
    };
  }

  if (decisionId === 'choose-forge') {
    return {
      state: { ...state, lastFeedback: null },
      feedback: {
        tone: 'blocked',
        message: 'Blocked: Forge does not fit the installed mods. Choose a proven-compatible loader.',
      },
    };
  }

  if (decisionId === 'choose-quilt') {
    return {
      state: { ...state, lastFeedback: null },
      feedback: {
        tone: 'caution',
        // SOL-3 P2 #2: name what "review" actually means, so an indeterminate
        // result is a next action rather than a dead end.
        message:
          'Quilt needs review — compatibility is not proven, so it cannot clear the blocker. To review it, check which loaders each installed mod supports; Fabric is already proven for all of them.',
      },
    };
  }

  if (decisionId === 'use-agora-java' && !scene.javaDecided) {
    const next = { ...scene, javaDecided: true, javaResolved: true };
    next.findings = findingsFor(next);
    next.runtime = runtimeFor(next);
    const done = next.memoryDecided;
    return {
      state: { ...state, checkpoint: done ? 3 : 2, scene: next, lastFeedback: null },
      feedback: { tone: 'success', message: 'Java 17 is managed by Agora — the warning is resolved.' },
    };
  }

  if (decisionId === 'keep-java-8' && !scene.javaDecided) {
    const next = { ...scene, javaDecided: true, javaResolved: false };
    next.findings = findingsFor(next);
    next.runtime = runtimeFor(next);
    const done = next.memoryDecided;
    return {
      state: { ...state, checkpoint: done ? 3 : 2, scene: next, lastFeedback: null },
      feedback: {
        tone: 'caution',
        message: 'Java 8 stays — the warning remains. You can proceed, but launch may fail.',
      },
    };
  }

  if (decisionId === 'keep-memory-auto' && !scene.memoryDecided) {
    const next = { ...scene, memoryDecided: true, memoryManual: false };
    next.findings = findingsFor(next);
    next.runtime = runtimeFor(next);
    const done = next.javaDecided;
    return {
      state: { ...state, checkpoint: done ? 3 : 2, scene: next, lastFeedback: null },
      feedback: { tone: 'info', message: 'Automatic memory kept — this is the recommended choice here.' },
    };
  }

  if (decisionId === 'stage-memory-manual' && !scene.memoryDecided) {
    const next = { ...scene, memoryDecided: true, memoryManual: true };
    next.findings = findingsFor(next);
    next.runtime = runtimeFor(next);
    const done = next.javaDecided;
    return {
      state: { ...state, checkpoint: done ? 3 : 2, scene: next, lastFeedback: null },
      feedback: {
        tone: 'info',
        message: 'Manual 4 GB staged with 12 GB headroom. Manual is fine; it does not beat automatic here.',
      },
    };
  }

  return { state, feedback: null };
}

export const healItScenario: LabScenario<HealItScene> = {
  id: NS,
  version: 1,
  title: 'Heal It',
  shortTitle: 'Heal It',
  description: 'Run a health check, fix the blocker, and review warnings.',
  iconLabel: '🩹',
  checkpoints,
  guideTopics: ['launching', 'java-performance'],
  realDestinations: [
    { type: 'tab', tab: 'instances' },
    { type: 'tab', tab: 'settings' },
  ],
  initialScene: canonicalScene,
  reduce,
  intentToDecision(_scene, intent: VisualIntent): LabDecisionRequest | null {
    if (intent.kind === 'review-loader') {
      const map: Record<string, string> = {
        [IDS.loaderFabric]: 'choose-fabric',
        [IDS.loaderQuilt]: 'choose-quilt',
        [IDS.loaderCurrent]: 'choose-forge',
      };
      const decisionId = intent.candidateId ? map[intent.candidateId] : undefined;
      if (decisionId) return { decisionId };
    }
    if (intent.kind === 'propose-memory') {
      return { decisionId: intent.mode === 'manual' ? 'stage-memory-manual' : 'keep-memory-auto' };
    }
    return null;
  },
  successPredicate(state) {
    return state.status === 'complete';
  },
  // Must not claim "green": keeping Java 8 is a valid ending, and the warning
  // deliberately persists. Asserting a clean check would contradict the screen
  // and undo the lesson that a kept warning stays (T6-6).
  completionMessage:
    'You completed the practice: the blocker was cleared with a proven choice, and you decided what to do about the warning and the recommendation. Blockers stop launch; a warning you keep stays until you resolve it.',
};
