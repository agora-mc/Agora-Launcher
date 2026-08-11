/**
 * Take It Offline scenario: per-instance, per-launch-mode offline readiness.
 *
 * Sol-0 contract: `docs/interactive/LESSON_MAP.md` §7. A cached catalog is not
 * readiness; network policy can block a fetch; and unknown stays unknown. This
 * adventure is simulation-only — a truthful live aggregate readiness query does
 * not exist yet (SOL-0 §11). Everything is authored, deterministic, and
 * namespaced `lab:offline:*`.
 */

import type { VisualNetworkReadiness, VisualScene } from '../../domain/models';
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

const NS = 'offline';

const IDS = {
  gameFiles: simId(NS, 'check', 'game-files'),
  loader: simId(NS, 'check', 'loader'),
  content: simId(NS, 'check', 'content'),
  java: simId(NS, 'check', 'java'),
  signIn: simId(NS, 'check', 'sign-in'),
};

export interface OfflineScene extends VisualScene {
  readiness: VisualNetworkReadiness;
  inspected: boolean;
  contentPrepared: boolean;
  launchMode: 'delegated' | 'direct';
  rechecked: boolean;
}

function readinessFor(input: {
  launchMode: 'delegated' | 'direct';
  contentPrepared: boolean;
  inspected: boolean;
}): VisualNetworkReadiness {
  const checks = [
    {
      id: IDS.gameFiles,
      category: 'game-files' as const,
      label: 'Minecraft game files',
      state: 'ready' as const,
      summary: 'The game files for this version are downloaded.',
    },
    {
      id: IDS.loader,
      category: 'loader' as const,
      label: 'Loader',
      state: 'ready' as const,
      summary: 'The loader for this instance is installed.',
    },
    {
      id: IDS.content,
      category: 'content' as const,
      label: 'Mods and packs',
      state: input.contentPrepared ? ('ready' as const) : ('missing' as const),
      summary: input.contentPrepared
        ? 'Every content file this instance needs is downloaded.'
        : 'One content file is not downloaded yet — it would fail without internet.',
    },
    {
      id: IDS.java,
      category: 'java' as const,
      label: 'Java runtime',
      state: 'ready' as const,
      summary: 'A compatible Java runtime is present.',
    },
    {
      id: IDS.signIn,
      category: 'sign-in-and-launch' as const,
      label: 'Sign-in & launch mode',
      state: input.launchMode === 'delegated' ? ('ready' as const) : ('blocked-by-policy' as const),
      summary:
        input.launchMode === 'delegated'
          ? 'Delegated launch needs no Microsoft sign-in here.'
          : 'Direct launch needs a Microsoft sign-in that cannot be verified without internet.',
    },
  ];
  const overall =
    input.inspected && checks.every((check) => check.state === 'ready')
      ? ('ready' as const)
      : input.inspected
        ? ('needs-attention' as const)
        : ('unknown' as const);
  return {
    instanceName: 'My Redstone World',
    launchMode: input.launchMode,
    policy: 'normal',
    overall,
    checkedAt: input.inspected ? 'just now' : undefined,
    checks,
  };
}

function canonicalScene(checkpoint: number): OfflineScene {
  const scene: OfflineScene = {
    source: { kind: 'simulation', scenarioId: NS, scenarioVersion: 1 },
    content: [],
    relationships: [],
    findings: [],
    proposals: [],
    readiness: readinessFor({ launchMode: 'delegated', contentPrepared: false, inspected: false }),
    inspected: false,
    contentPrepared: false,
    launchMode: 'delegated',
    rechecked: false,
  };
  if (checkpoint >= 1) {
    scene.inspected = true;
    scene.readiness = readinessFor({ launchMode: scene.launchMode, contentPrepared: scene.contentPrepared, inspected: true });
  }
  if (checkpoint >= 2) {
    scene.contentPrepared = true;
    scene.readiness = readinessFor({ launchMode: scene.launchMode, contentPrepared: true, inspected: true });
  }
  if (checkpoint >= 3) {
    scene.rechecked = true;
    scene.readiness = readinessFor({ launchMode: scene.launchMode, contentPrepared: scene.contentPrepared, inspected: true });
  }
  return scene;
}

function decisionsFor(state: LabLessonState<OfflineScene>): LabDecision[] {
  if (state.checkpoint === 0) {
    return [{ id: 'inspect-delegated', label: 'Inspect readiness for delegated launch' }];
  }
  if (state.checkpoint === 1) {
    return [
      { id: 'download-content', label: 'Download the missing file now' },
      { id: 'leave-missing', label: 'Leave it missing' },
    ];
  }
  if (state.checkpoint === 2) {
    return [
      { id: 'use-delegated', label: 'Use delegated launch' },
      { id: 'use-direct', label: 'Use direct launch' },
    ];
  }
  if (state.checkpoint === 3) {
    return [{ id: 'recheck', label: 'Re-check readiness' }];
  }
  return [];
}

const checkpoints: LabCheckpoint<OfflineScene>[] = [
  {
    id: 'inspect',
    goal: 'See what this instance needs when the internet is gone.',
    expectedModel: 'Offline readiness is per instance and per launch mode.',
    decisionsFor,
  },
  {
    id: 'content',
    goal: 'A missing content file would fail offline. Prepare it.',
    expectedModel: 'Missing content must be downloaded before leaving — a cached catalog is not the same as ready.',
    decisionsFor,
  },
  {
    id: 'launch-mode',
    goal: 'Direct launch needs a sign-in; delegated launch does not.',
    expectedModel: 'Sign-in/launch needs differ by launch mode and cannot be verified offline for direct.',
    decisionsFor,
  },
  {
    id: 'recheck',
    goal: 'Re-check and tell Ready from Unknown.',
    expectedModel: 'A cached registry never lights every node; Unknown must stay Unknown.',
    decisionsFor,
  },
];

function reduce(
  state: LabLessonState<OfflineScene>,
  event: { kind: 'decision'; decisionId: string },
): LabReduction<OfflineScene> {
  const scene = state.scene;
  const { decisionId } = event;

  if (decisionId === 'inspect-delegated') {
    const next = { ...scene, inspected: true, launchMode: 'delegated' as const };
    next.readiness = readinessFor({ launchMode: next.launchMode, contentPrepared: next.contentPrepared, inspected: true });
    return {
      state: { ...state, checkpoint: 1, scene: next, lastFeedback: null },
      feedback: {
        tone: 'info',
        message: 'Readiness map shown. Content is missing; everything else is ready for delegated launch.',
      },
    };
  }

  if (decisionId === 'download-content') {
    const next = { ...scene, contentPrepared: true };
    next.readiness = readinessFor({ launchMode: next.launchMode, contentPrepared: true, inspected: true });
    return {
      state: { ...state, checkpoint: 2, scene: next, lastFeedback: null },
      feedback: { tone: 'success', message: 'The missing content file is downloaded. Content is now ready.' },
    };
  }

  if (decisionId === 'leave-missing') {
    const next = { ...scene, contentPrepared: false };
    next.readiness = readinessFor({ launchMode: next.launchMode, contentPrepared: false, inspected: true });
    return {
      state: { ...state, checkpoint: 2, scene: next, lastFeedback: null },
      feedback: {
        tone: 'caution',
        message: 'Content stays missing — this instance would not be ready for offline play.',
      },
    };
  }

  if (decisionId === 'use-delegated' || decisionId === 'use-direct') {
    const mode: 'delegated' | 'direct' = decisionId === 'use-delegated' ? 'delegated' : 'direct';
    const next = { ...scene, launchMode: mode };
    next.readiness = readinessFor({ launchMode: mode, contentPrepared: next.contentPrepared, inspected: true });
    return {
      state: { ...state, checkpoint: 3, scene: next, lastFeedback: null },
      feedback: {
        tone: mode === 'delegated' ? 'success' : 'caution',
        message:
          mode === 'delegated'
            ? 'Delegated launch needs no sign-in here — ready.'
            : 'Direct launch is blocked by policy: its Microsoft sign-in cannot be verified offline.',
      },
    };
  }

  if (decisionId === 'recheck') {
    const allReady = readinessFor({ launchMode: scene.launchMode, contentPrepared: scene.contentPrepared, inspected: true }).checks.every(
      (check) => check.state === 'ready',
    );
    const next = { ...scene, rechecked: true };
    next.readiness = readinessFor({ launchMode: next.launchMode, contentPrepared: next.contentPrepared, inspected: true });
    return {
      state: { ...state, status: 'complete', scene: next, lastFeedback: null },
      feedback: {
        tone: allReady ? 'success' : 'caution',
        message: allReady
          ? 'Re-check complete: every verified need is Ready. Remember — a cached catalog alone never makes an instance ready.'
          : 'Re-check complete: something still needs attention. Unknown stays unknown; Agora will not promise what it cannot verify.',
      },
    };
  }

  return { state, feedback: null };
}

export const takeItOfflineScenario: LabScenario<OfflineScene> = {
  id: NS,
  version: 1,
  title: 'Take It Offline',
  shortTitle: 'Take It Offline',
  description: 'Prepare an instance for a trip with no internet.',
  iconLabel: '📡',
  checkpoints,
  guideTopics: ['privacy-offline'],
  realDestinations: [
    { type: 'tab', tab: 'settings' },
  ],
  initialScene: canonicalScene,
  // Checkpoint 2+ depends on branching choices progress does not record
  // ("Download the missing file" vs "Leave it missing"; delegated vs direct
  // launch). Resuming there would reverse them, so resume stops at the
  // unambiguous "inspected" stage (SOL §22 / T6-2).
  safeResumeCheckpoint: (checkpoint: number) => Math.min(checkpoint, 1),
  reduce,
  intentToDecision(_scene, intent: VisualIntent): LabDecisionRequest | null {
    if (intent.kind === 'review-offline-readiness') {
      return { decisionId: 'recheck' };
    }
    return null;
  },
  successPredicate(state) {
    return state.status === 'complete';
  },
  completionMessage: 'You completed the practice: you verified game files, loader, content, Java, and sign-in needs for a launch mode, and you did not turn Unknown into Ready.',
};
