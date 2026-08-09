/**
 * Scenario views: compose shared visual components from scenario scenes.
 *
 * These views are Lab-only: they render simulation scenes through the shared
 * visuals and translate visual intents back into scenario decisions via
 * `scenario.intentToDecision`. No view here calls Tauri or a live controller.
 */

import type { VisualId } from '../domain/models';
import type { CapabilityFlags } from '../domain/models';
import { NO_CAPABILITIES as NO_CAPS } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import type { LabLessonState, LabScenario } from './scenarioTypes';
import { InstanceBench } from '../visual/InstanceBench';
import { ContentGraph } from '../visual/ContentGraph';
import { ChangeStaging } from '../visual/ChangeStaging';
import { RecoveryTimeline } from '../visual/RecoveryTimeline';
import { SeverityBadge, StatusChip } from '../visual/primitives/statusChips';
import type { BuildItScene } from './scenarios/buildIt';
import type { ModItScene } from './scenarios/modIt';
import type { UndoItScene } from './scenarios/undoIt';

/** Capabilities that enable the simulated gestures the Lab teaches. */
export function labCapabilities(): CapabilityFlags {
  return {
    canProposeInstall: true,
    canProposeUpdate: false,
    canProposeRemove: true,
    canProposeEnabled: false,
    canReviewHealth: false,
    canReviewLoader: false,
    canOpenCrashDoctor: false,
    canPreviewSnapshot: true,
    canRequestSnapshotRestore: true,
    canProposeMemory: false,
    canReviewOfflineReadiness: false,
  };
}

export interface ScenarioViewProps<Scene> {
  scenario: LabScenario<Scene>;
  state: LabLessonState<Scene>;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  reducedMotion: boolean;
}

function BuildItView({
  state,
  selection,
  onSelect,
  reducedMotion,
}: ScenarioViewProps<BuildItScene>) {
  const scene = state.scene;
  if (!scene.instance) return null;
  return (
    <div className="space-y-4">
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <InstanceBench
          instance={scene.instance}
          source={scene.source}
          selection={selection}
          onSelect={onSelect}
          onIntent={() => undefined}
          capabilities={NO_CAPS}
          roleLabel="Your new instance"
          highlight
          reducedMotion={reducedMotion}
        />
        <InstanceBench
          instance={scene.sibling}
          source={scene.source}
          selection={selection}
          onSelect={onSelect}
          onIntent={() => undefined}
          capabilities={NO_CAPS}
          roleLabel="Existing example"
          statusLabel="Separate · unchanged"
          reducedMotion={reducedMotion}
        />
      </div>
      <section aria-label="Tile tray" className="rounded-xl border border-border bg-card p-3">
        <h4 className="text-xs font-bold uppercase tracking-wide text-muted-foreground">Tile tray</h4>
        <ul className="mt-2 space-y-1.5">
          {scene.tray.map((tile) => {
            const placed = scene.placedTileIds.includes(tile.id);
            return (
              <li key={tile.id} className="flex flex-wrap items-center gap-2 text-sm">
                <span className={placed ? 'text-foreground line-through opacity-70' : 'text-foreground'}>
                  {tile.name}
                </span>
                <span className="text-xs text-muted-foreground">needs {tile.needs}</span>
                {placed ? <StatusChip label="Placed" /> : null}
                {!placed && scene.named && tile.id.includes('notebot') && scene.loaderFamily !== 'Fabric' ? (
                  <SeverityBadge severity="blocker" />
                ) : null}
              </li>
            );
          })}
        </ul>
      </section>
    </div>
  );
}

function ModItView({
  state,
  selection,
  onSelect,
  onIntent,
  reducedMotion,
}: ScenarioViewProps<ModItScene>) {
  const scene = state.scene;
  const canApply = scene.conflictResolved && scene.requiredAdded && scene.staged;
  return (
    <div className="space-y-4">
      <ContentGraph
        scene={scene}
        source={scene.source}
        selection={selection}
        onSelect={onSelect}
        onIntent={onIntent}
        capabilities={labCapabilities()}
        reducedMotion={reducedMotion}
      />
      <ChangeStaging
        scene={scene}
        source={scene.source}
        onIntent={onIntent}
        capabilities={labCapabilities()}
        reviewLabel="Apply simulated plan"
        reviewAvailable={canApply}
        outcome={scene.applied
          ? {
              title: 'Plan applied',
              summary: `BetterCaves and Core Lib installed${scene.optionalAdded ? '; Nice Textures installed' : ''}. Terrain Overhaul removed.`,
              recoveryPoint: 'A simulated return point was created before applying.',
            }
          : null}
        reducedMotion={reducedMotion}
      />
    </div>
  );
}

function UndoItView({
  state,
  selection,
  onSelect,
  onIntent,
  reducedMotion,
}: ScenarioViewProps<UndoItScene>) {
  const scene = state.scene;
  const compared = scene.snapshots.find((snapshot) => snapshot.id === scene.comparedSnapshotId);
  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-2">
        <StatusChip label={scene.processRunning ? 'Running' : 'Stopped'} />
        {scene.restored ? <StatusChip label="Restored" /> : null}
        {scene.undoPointCreated ? <StatusChip label="Undo point created" /> : null}
      </div>
      <RecoveryTimeline
        snapshots={scene.snapshots}
        currentStateLabel={scene.currentLabel}
        source={scene.source}
        selection={selection}
        onSelect={onSelect}
        onIntent={onIntent}
        capabilities={labCapabilities()}
        reducedMotion={reducedMotion}
      />
      {compared ? (
        <div className="rounded-lg border border-dashed border-border bg-muted/30 p-3" role="region" aria-label="Recovery ghost comparison">
          <h4 className="text-sm font-bold text-foreground">Return point comparison: {compared.label}</h4>
          {compared.changeSummary ? (
            <p className="mt-1 text-xs text-muted-foreground">
              Added {compared.changeSummary.added} · Changed {compared.changeSummary.changed} · Removed {compared.changeSummary.removed}
            </p>
          ) : null}
          <p className="mt-1 text-xs font-semibold text-foreground">
            {compared.worldProtection === 'included'
              ? 'Worlds/saves ARE included in this return point.'
              : 'Worlds/saves are NOT included in this return point.'}
          </p>
        </div>
      ) : null}
      {scene.restored && scene.restoredSummary ? (
        <div
          className="rounded-lg border border-emerald-600/40 bg-emerald-600/5 p-3"
          role="region"
          aria-label="Restore outcome"
          data-outcome="restored"
        >
          <h4 className="text-sm font-bold text-foreground">Restored</h4>
          <p className="mt-1 text-xs text-muted-foreground">{scene.restoredSummary}</p>
        </div>
      ) : null}
    </div>
  );
}

export function ScenarioView<Scene>(props: ScenarioViewProps<Scene>) {
  const { scenario } = props;
  switch (scenario.id) {
    case 'build':
      return <BuildItView {...(props as ScenarioViewProps<BuildItScene>)} />;
    case 'mod':
      return <ModItView {...(props as ScenarioViewProps<ModItScene>)} />;
    case 'undo':
      return <UndoItView {...(props as ScenarioViewProps<UndoItScene>)} />;
    default:
      return null;
  }
}
