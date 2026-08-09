/**
 * Build It scenario: instance isolation.
 *
 * Sol-0 contract: `docs/interactive/LESSON_MAP.md` §2. A simulated instance is
 * an isolated home for a Minecraft version, loader choice, and content. The
 * lesson: choose a foundation first, and an incompatible tile cannot become a
 * valid current state. Everything here is authored, deterministic, and
 * namespaced `lab:build:*`.
 */

import type {
  Compatibility,
  ContentKind,
  VisualId,
  VisualInstance,
  VisualScene,
} from '../../domain/models';
import type {
  LabCheckpoint,
  LabDecision,
  LabLessonState,
  LabReduction,
  LabScenario,
} from '../scenarioTypes';
import { simId } from '../scenarioTypes';

const NS = 'build';

export interface TrayTile {
  id: VisualId;
  name: string;
  kind: ContentKind;
  needs: string;
}

export interface BuildItScene extends VisualScene {
  tray: TrayTile[];
  placedTileIds: VisualId[];
  versionChosen: boolean;
  loaderChosen: boolean;
  loaderFamily: string;
  loaderCompat: Compatibility;
  named: boolean;
  modPlaced: boolean;
  finishChosen: boolean;
  /** A separate example instance shown to teach isolation. */
  sibling: VisualInstance;
}

const VERSION_TILE: TrayTile = {
  id: simId(NS, 'tile', 'version-1-20-1'),
  name: 'Minecraft 1.20.1',
  kind: 'world',
  needs: '—',
};

const LOADER_TILES: TrayTile[] = [
  { id: simId(NS, 'tile', 'fabric'), name: 'Fabric loader', kind: 'mod', needs: '1.20.1' },
  { id: simId(NS, 'tile', 'vanilla'), name: 'Vanilla (no loader)', kind: 'mod', needs: '1.20.1' },
  { id: simId(NS, 'tile', 'forge'), name: 'Forge loader', kind: 'mod', needs: '1.21' },
];

const MOD_TILE: TrayTile = {
  id: simId(NS, 'tile', 'notebot'),
  name: 'Notebot Mod',
  kind: 'mod',
  needs: 'Fabric + 1.20.1',
};

function siblingInstance(): VisualInstance {
  return {
    id: simId(NS, 'instance', 'friend'),
    name: "Your friend's world",
    gameVersion: '1.21',
    loader: { current: { family: 'Forge', compatibility: 'compatible' } },
    lockState: 'editable',
    recoveryReadiness: 'unknown',
    launchState: 'idle',
    contentSummary: { enabled: 2, disabled: 0, needsAttention: 0 },
  };
}

function buildInstance(scene: BuildItScene): VisualInstance {
  return {
    id: simId(NS, 'instance', 'mine'),
    name: scene.named ? 'My Redstone World' : 'Untitled',
    gameVersion: scene.versionChosen ? '1.20.1' : '—',
    loader: {
      current: scene.loaderChosen
        ? {
            family: scene.loaderFamily,
            compatibility: scene.loaderCompat as Compatibility,
          }
        : { family: '—', compatibility: 'unknown' },
    },
    lockState: 'editable',
    recoveryReadiness: 'unknown',
    launchState: 'idle',
    contentSummary: {
      enabled: scene.modPlaced ? 1 : 0,
      disabled: 0,
      needsAttention: scene.loaderChosen && scene.loaderFamily === 'Forge' ? 1 : 0,
    },
  };
}

function baseScene(): Omit<BuildItScene, 'source' | 'instance' | 'content' | 'relationships' | 'findings' | 'proposals' | 'sibling'> {
  return {
    tray: [],
    placedTileIds: [],
    versionChosen: false,
    loaderChosen: false,
    loaderFamily: '',
    loaderCompat: 'unknown',
    named: false,
    modPlaced: false,
    finishChosen: false,
  };
}

function canonicalScene(checkpoint: number): BuildItScene {
  const base = baseScene();
  const scene: BuildItScene = {
    ...base,
    source: { kind: 'simulation', scenarioId: NS, scenarioVersion: 1 },
    content: [],
    relationships: [],
    findings: [],
    proposals: [],
    sibling: siblingInstance(),
  };
  if (checkpoint >= 1) {
    scene.tray = LOADER_TILES;
    scene.versionChosen = true;
  } else {
    scene.tray = [VERSION_TILE];
  }
  if (checkpoint >= 2) {
    scene.tray = [MOD_TILE];
    scene.loaderChosen = true;
    scene.loaderFamily = 'Fabric';
    scene.loaderCompat = 'compatible';
  }
  scene.instance = buildInstance(scene);
  return scene;
}

function decisionsFor(state: LabLessonState<BuildItScene>): LabDecision[] {
  const scene = state.scene;
  if (state.checkpoint === 0) {
    return [
      { id: 'place-version', label: 'Place Minecraft 1.20.1' },
    ];
  }
  if (state.checkpoint === 1) {
    return [
      { id: 'choose-fabric', label: 'Choose Fabric' },
      { id: 'choose-vanilla', label: 'Choose Vanilla' },
      { id: 'choose-forge', label: 'Choose Forge' },
    ];
  }
  // Checkpoint 2: name, then place content (or finish).
  const decisions: LabDecision[] = [];
  if (!scene.named) {
    decisions.push({ id: 'name-it', label: 'Name it "My Redstone World"' });
  }
  if (scene.named && !scene.modPlaced && !scene.finishChosen) {
    if (scene.loaderFamily !== 'Fabric') {
      decisions.push({ id: 'switch-fabric', label: 'Switch to Fabric' });
      decisions.push({
        id: 'place-mod',
        label: 'Place Notebot Mod',
        disabledReason: 'Notebot Mod needs Fabric — this instance is not on Fabric yet.',
      });
    } else {
      decisions.push({ id: 'place-mod', label: 'Place Notebot Mod' });
    }
    decisions.push({ id: 'finish', label: 'Finish (no mod)' });
  }
  return decisions;
}

const checkpoints: LabCheckpoint<BuildItScene>[] = [
  {
    id: 'choose-version',
    goal: 'Give your instance a foundation: pick a Minecraft version.',
    expectedModel: 'An instance is an isolated home; the version is its first foundation.',
    decisionsFor,
  },
  {
    id: 'choose-loader',
    goal: 'Choose a loader that fits the content you want — or vanilla when none is needed.',
    expectedModel: 'The loader choice belongs to this instance and should fit its content.',
    decisionsFor,
  },
  {
    id: 'name-and-place',
    goal: 'Name it and try one content tile. An incompatible tile cannot become current state.',
    expectedModel: 'A tile that does not fit stays in the tray — it cannot force its way in.',
    decisionsFor,
  },
];

function reduce(
  state: LabLessonState<BuildItScene>,
  event: { kind: 'decision'; decisionId: string },
): LabReduction<BuildItScene> {
  const scene = state.scene;
  const { decisionId } = event;

  if (decisionId === 'place-version' && !scene.versionChosen) {
    const next = { ...scene, versionChosen: true, tray: LOADER_TILES, instance: buildInstance({ ...scene, versionChosen: true }) };
    return {
      state: { ...state, checkpoint: 1, scene: next, lastFeedback: null },
      feedback: { tone: 'success', message: 'Minecraft 1.20.1 is placed on the bench.' },
    };
  }

  if (decisionId === 'choose-fabric' || decisionId === 'choose-vanilla' || decisionId === 'choose-forge') {
    const family = decisionId === 'choose-fabric' ? 'Fabric' : decisionId === 'choose-vanilla' ? 'Vanilla' : 'Forge';
    const compatibility: Compatibility = family === 'Forge' ? 'incompatible' : 'compatible';
    if (family === 'Forge') {
      const next = { ...scene, loaderChosen: true, loaderFamily: family, loaderCompat: compatibility, instance: buildInstance({ ...scene, loaderChosen: true, loaderFamily: family, loaderCompat: compatibility }) };
      return {
        state: { ...state, scene: next, lastFeedback: null },
        feedback: {
          tone: 'caution',
          message: 'Forge does not fit 1.20.1 for this mod. Choose Fabric or Vanilla instead.',
        },
      };
    }
    const next = { ...scene, loaderChosen: true, loaderFamily: family, loaderCompat: compatibility, tray: [MOD_TILE], instance: buildInstance({ ...scene, loaderChosen: true, loaderFamily: family, loaderCompat: compatibility }) };
    const feedback = family === 'Vanilla'
      ? { tone: 'info' as const, message: 'Vanilla works when no mods are needed. Your mod tile will still need Fabric.' }
      : { tone: 'success' as const, message: 'Fabric fits this setup. Now name it and place content.' };
    return {
      state: { ...state, checkpoint: 2, scene: next, lastFeedback: null },
      feedback,
    };
  }

  if (decisionId === 'name-it' && !scene.named) {
    const next = { ...scene, named: true, instance: buildInstance({ ...scene, named: true }) };
    return {
      state: { ...state, scene: next, lastFeedback: null },
      feedback: { tone: 'info', message: 'This instance has a purpose: My Redstone World.' },
    };
  }

  if (decisionId === 'switch-fabric' && scene.loaderFamily !== 'Fabric') {
    const next: BuildItScene = { ...scene, loaderFamily: 'Fabric', loaderCompat: 'compatible', instance: buildInstance({ ...scene, loaderFamily: 'Fabric', loaderCompat: 'compatible' }) };
    return {
      state: { ...state, scene: next, lastFeedback: null },
      feedback: { tone: 'success', message: 'Switched to Fabric — the mod tile fits now.' },
    };
  }

  if (decisionId === 'place-mod') {
    if (scene.loaderFamily !== 'Fabric') {
      return {
        state: { ...state, lastFeedback: null },
        feedback: { tone: 'blocked', message: 'Blocked: Notebot Mod needs Fabric + 1.20.1. It stays in the tray.' },
      };
    }
    if (scene.modPlaced) return { state, feedback: null };
    const next = {
      ...scene,
      modPlaced: true,
      placedTileIds: [...scene.placedTileIds, MOD_TILE.id],
      instance: buildInstance({ ...scene, modPlaced: true }),
    };
    return {
      state: { ...state, status: 'complete', scene: next, lastFeedback: null },
      feedback: { tone: 'success', message: 'The mod fits and is placed inside this instance only.' },
    };
  }

  if (decisionId === 'finish' && !scene.finishChosen && !scene.modPlaced) {
    const next = { ...scene, finishChosen: true };
    return {
      state: { ...state, status: 'complete', scene: next, lastFeedback: null },
      feedback: { tone: 'success', message: 'You built a separate instance with no mod — a valid current state.' },
    };
  }

  return { state, feedback: null };
}

export const buildItScenario: LabScenario<BuildItScene> = {
  id: NS,
  version: 1,
  title: 'Build It',
  shortTitle: 'Build It',
  description: 'Make a separate instance: version, loader, and a place for content.',
  iconLabel: '🧱',
  checkpoints,
  guideTopics: ['instances', 'modding-foundations'],
  realDestinations: [
    { type: 'tab', tab: 'instances' },
  ],
  initialScene: canonicalScene,
  reduce,
  successPredicate(state) {
    return state.status === 'complete';
  },
  completionMessage: 'You completed the practice: your simulated instance stays separate from the example beside it.',
};
