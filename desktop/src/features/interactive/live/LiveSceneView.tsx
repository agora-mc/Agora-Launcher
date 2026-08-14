/**
 * Live Scene View — the High Interaction instance view, now rendered by the
 * WorldEditor (the v4-world foreground port). Kept as a thin shell so the
 * host's data/refresh/intent plumbing is unchanged.
 *
 * The safety spine is unchanged: gestures emit `VisualIntent`; the host routes
 * them to the reviewed Standard surface. Nothing here executes a mutation.
 */

import type { CapabilityFlags, VisualId } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import type { Fragment } from './liveScene';
import type { VisualCrashEvidence, VisualRuntimeState, VisualScene, VisualSnapshot } from '../domain/models';
import { WorldEditor } from './WorldEditor';

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
  /** Catalogue detail for the selected item (fetched by the host). */
  selectedDetail?: import('./readAdapters').ContentDetail;
  capabilities: CapabilityFlags;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  onUseStandardView: () => void;
  onLaunch?: () => Promise<void> | void;
  reducedMotion?: boolean;
  /** Presentation: `simple` hides the decorative flourish. */
  presentation?: 'standard' | 'simple' | 'high-interaction';
}

export function LiveSceneView(props: LiveSceneViewProps) {
  const { data, presentation = 'high-interaction' } = props;
  return (
    <div data-testid="live-scene-view" data-source={data.scene.source.kind} data-presentation={presentation}>
      <WorldEditor {...props} />
    </div>
  );
}
