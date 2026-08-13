/**
 * Live Scene View: renders the shared visuals from live data.
 *
 * A High Interaction presentation surface. Every fragment is labelled with its
 * own availability: a failed read renders as unavailable / unknown and is never
 * shown as an ordinary empty value or "ready" state (SOL-2 BLOCKER 1).
 * Review/navigation intents are forwarded to the host, which routes them to the
 * STANDARD Agora surface — approved reviews open Standard (remove uses the
 * reviewed InstallFlow); nothing here executes a mutation directly.
 */

import type { VisualId, VisualScene } from '../domain/models';
import type { VisualSnapshot, VisualCrashEvidence, VisualRuntimeState, CapabilityFlags } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import type { Fragment } from './liveScene';
import { InstanceBench } from '../visual/InstanceBench';
import { ContentGraph } from '../visual/ContentGraph';
import { HealthLens } from '../visual/HealthLens';
import { RuntimeWorkbench } from '../visual/RuntimeWorkbench';
import { CrashEvidenceBoard } from '../visual/CrashEvidenceBoard';
import { RecoveryTimeline } from '../visual/RecoveryTimeline';
import { StatusChip } from '../visual/primitives/statusChips';

export interface LiveHostData {
  scene: VisualScene;
  /** Health verification fragment (ok = a fresh health report was read). */
  health: Fragment<boolean>;
  snapshots: Fragment<VisualSnapshot[]>;
  crashEvidence: Fragment<VisualCrashEvidence | null>;
  runtime: Fragment<VisualRuntimeState | null>;
}

export interface LiveSceneViewProps {
  data: LiveHostData;
  capabilities: CapabilityFlags;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  reducedMotion?: boolean;
  /** Presentation: `simple` hides the decorative flourish. */
  presentation?: 'standard' | 'simple' | 'high-interaction';
}

function UnavailableNote({ label }: { label: string }) {
  return (
    <p className="text-sm font-medium text-muted-foreground" role="status">
      {label}
    </p>
  );
}

export function LiveSceneView({ data, capabilities, selection, onSelect, onIntent, reducedMotion, presentation = 'high-interaction' }: LiveSceneViewProps) {
  const { scene } = data;
  const source = scene.source;
  const healthOk = data.health.status === 'ok';
  const snapshotsOk = data.snapshots.status === 'ok' ? data.snapshots.value : null;
  const crashOk = data.crashEvidence.status === 'ok' ? data.crashEvidence.value : null;
  const runtimeOk = data.runtime.status === 'ok' ? data.runtime.value : null;
  const modeLabel = presentation === 'simple' ? 'Simple' : 'High Interaction';

  return (
    <div className="space-y-5" data-testid="live-scene-view" data-source={source.kind} data-presentation={presentation}>
      <div className="flex flex-wrap items-center gap-2">
        <StatusChip label={modeLabel} />
        {source.kind === 'live' ? (
          <StatusChip label={source.freshness === 'fresh' ? 'Live' : source.freshness === 'refreshing' ? 'Refreshing…' : 'Live (degraded)'} />
        ) : null}
        <span className="text-xs text-muted-foreground">
          Reviews open the Standard surface; content changes use the reviewed InstallFlow.
        </span>
      </div>

      {scene.instance ? (
        <InstanceBench
          instance={scene.instance}
          source={source}
          selection={selection}
          onSelect={onSelect}
          onIntent={onIntent}
          capabilities={capabilities}
          roleLabel={scene.instance.name}
          statusLabel={
            scene.instance.launchState === 'running'
              ? 'Running'
              : scene.instance.lockState === 'busy'
                ? 'Busy'
                : scene.instance.lockState === 'locked-by-player'
                  ? 'Locked'
                  : undefined
          }
          reducedMotion={reducedMotion}
        />
      ) : null}

      {scene.content.length > 0 || scene.relationships.length > 0 ? (
        <ContentGraph
          scene={scene}
          selection={selection}
          onSelect={onSelect}
          onIntent={onIntent}
          capabilities={capabilities}
          reducedMotion={reducedMotion}
        />
      ) : null}

      {healthOk ? (
        <HealthLens
          findings={scene.findings}
          source={source}
          validated
          selection={selection}
          onSelect={onSelect}
          onIntent={onIntent}
          capabilities={capabilities}
          reducedMotion={reducedMotion}
        />
      ) : (
        <HealthLens
          findings={[]}
          source={source}
          validated={false}
          unavailable
          selection={selection}
          onSelect={onSelect}
          onIntent={onIntent}
          capabilities={capabilities}
          reducedMotion={reducedMotion}
        />
      )}

      {runtimeOk ? (
        <RuntimeWorkbench
          runtime={runtimeOk}
          source={source}
          selection={selection}
          onSelect={onSelect}
          onIntent={onIntent}
          capabilities={capabilities}
          reducedMotion={reducedMotion}
        />
      ) : (
        <UnavailableNote label="Runtime data could not be verified — nothing here is treated as current." />
      )}

      {snapshotsOk && snapshotsOk.length > 0 ? (
        <RecoveryTimeline
          snapshots={snapshotsOk}
          currentStateLabel="Current state"
          source={source}
          selection={selection}
          onSelect={onSelect}
          onIntent={onIntent}
          capabilities={capabilities}
          reducedMotion={reducedMotion}
        />
      ) : snapshotsOk === null ? (
        <UnavailableNote label="Return points could not be read — none are shown." />
      ) : null}

      {crashOk ? (
        <CrashEvidenceBoard
          evidence={crashOk}
          source={source}
          selection={selection}
          onSelect={onSelect}
          onIntent={onIntent}
          capabilities={capabilities}
          reducedMotion={reducedMotion}
        />
      ) : data.crashEvidence.status !== 'ok' ? (
        <UnavailableNote label="Crash evidence could not be read." />
      ) : null}
    </div>
  );
}
