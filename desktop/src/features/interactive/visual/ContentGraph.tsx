import type { CapabilityFlags, ExperienceSource, VisualId, VisualRelationship, VisualScene } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { LinearViewToggle, useLinearView } from './primitives/LinearView';
import { RelationshipBadge, SeverityBadge } from './primitives/statusChips';
import { ProposedMark } from './primitives/stateMarks';

/**
 * Content Graph — "What relies on what, and what would this change affect?"
 *
 * Renders `VisualContentNode`s and `VisualRelationship`s. A node's direct
 * relationships are the semantic model; the diagram arrangement is
 * presentation. Missing requirements render as an open/broken socket with a
 * named target — the visual never invents a substitute node.
 *
 * A controlled component: emits `VisualIntent`s only, never calls Tauri.
 */

export interface ContentGraphProps {
  scene: VisualScene;
  source: ExperienceSource;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  capabilities: CapabilityFlags;
  reducedMotion?: boolean;
}

const KIND_LABEL: Record<string, string> = {
  mod: 'Mod',
  modpack: 'Pack',
  'resource-pack': 'Resource pack',
  shader: 'Shader',
  datapack: 'Datapack',
  world: 'World',
};

function nodeById(scene: VisualScene, id: VisualId) {
  return scene.content.find((node) => node.id === id);
}

/**
 * A socket chip renders one relationship at a node with an explicit
 * filled/empty socket and text label, so requires/recommends/conflicts and
 * satisfied/missing/resolved are readable without the explanatory sentence.
 */
function SocketChip({
  relationship,
  scene,
  onSelect,
}: {
  relationship: VisualRelationship;
  scene: VisualScene;
  onSelect: (id: VisualId | null) => void;
}) {
  const to = relationship.toId ? nodeById(scene, relationship.toId) : undefined;
  const targetName = to?.name ?? 'missing item';
  const target = relationship.toId ? (
    <button
      type="button"
      onClick={() => onSelect(relationship.toId ?? null)}
      className="font-semibold underline decoration-dotted underline-offset-2 hover:text-primary"
    >
      {targetName}
    </button>
  ) : (
    <span className="font-semibold">{targetName}</span>
  );

  if (relationship.kind === 'requires') {
    if (relationship.state === 'satisfied') {
      return (
        <li className="inline-flex items-center gap-1.5 rounded-md border border-emerald-600/50 bg-emerald-600/5 px-2 py-1 text-xs font-medium text-foreground">
          <span aria-hidden="true" className="text-emerald-600 dark:text-emerald-400">●</span>
          <span>Requires {target}</span>
          <span className="text-emerald-700 dark:text-emerald-300">Satisfied</span>
        </li>
      );
    }
    return (
      <li className="inline-flex items-center gap-1.5 rounded-md border border-dashed border-destructive/70 bg-destructive/5 px-2 py-1 text-xs font-medium text-foreground">
        <span aria-hidden="true" className="text-destructive">◌</span>
        <span>Requires {target}</span>
        <span className="font-semibold text-destructive">Missing</span>
      </li>
    );
  }

  if (relationship.kind === 'recommends') {
    if (relationship.state === 'satisfied') {
      return (
        <li className="inline-flex items-center gap-1.5 rounded-md border border-emerald-600/50 bg-emerald-600/5 px-2 py-1 text-xs font-medium text-foreground">
          <span aria-hidden="true" className="text-emerald-600 dark:text-emerald-400">●</span>
          <span>Recommends {target}</span>
        </li>
      );
    }
    return (
      <li className="inline-flex items-center gap-1.5 rounded-md border border-dashed border-border bg-muted/40 px-2 py-1 text-xs font-medium text-foreground">
        <span aria-hidden="true" className="text-muted-foreground">◌</span>
        <span>Recommends {target}</span>
        <span className="text-muted-foreground">(optional)</span>
      </li>
    );
  }

  // conflicts-with
  if (relationship.state === 'satisfied') {
    return (
      <li className="inline-flex items-center gap-1.5 rounded-md border border-border bg-muted/40 px-2 py-1 text-xs font-medium text-muted-foreground">
        <span aria-hidden="true">✓</span>
        <span className="line-through">Conflicts with {target}</span>
        <span>Resolved</span>
      </li>
    );
  }
  return (
    <li className="inline-flex items-center gap-1.5 rounded-md border border-destructive/60 bg-destructive/5 px-2 py-1 text-xs font-medium text-foreground">
      <span aria-hidden="true" className="text-destructive">✕</span>
      <span>Conflicts with {target}</span>
      <span className="font-semibold text-destructive">Blocking</span>
    </li>
  );
}

/** Socket strip on a node card: outgoing relationships + incoming requires. */
function SocketStrip({
  nodeId,
  scene,
  onSelect,
}: {
  nodeId: VisualId;
  scene: VisualScene;
  onSelect: (id: VisualId | null) => void;
}) {
  const outgoing = scene.relationships.filter((relationship) => relationship.fromId === nodeId);
  const requiredBy = scene.relationships.filter(
    (relationship) => relationship.toId === nodeId && relationship.kind === 'requires',
  );
  if (outgoing.length === 0 && requiredBy.length === 0) return null;
  const nodeName = nodeById(scene, nodeId)?.name ?? nodeId;
  return (
    <ul className="mt-2 space-y-1" aria-label={`Relationships for ${nodeName}`}>
      {outgoing.map((relationship) => (
        <SocketChip key={relationship.id} relationship={relationship} scene={scene} onSelect={onSelect} />
      ))}
      {requiredBy.length > 0 ? (
        <li className="text-xs text-muted-foreground">
          Required by{' '}
          {requiredBy.map((relationship, index) => {
            const from = nodeById(scene, relationship.fromId);
            return (
              <span key={relationship.id}>
                {index > 0 ? ', ' : ''}
                <button
                  type="button"
                  onClick={() => onSelect(relationship.fromId)}
                  className="font-semibold text-foreground underline decoration-dotted underline-offset-2 hover:text-primary"
                >
                  {from?.name ?? relationship.fromId}
                </button>
              </span>
            );
          })}
        </li>
      ) : null}
    </ul>
  );
}

function RelationshipRow({
  relationship,
  scene,
  onSelect,
}: {
  relationship: VisualRelationship;
  scene: VisualScene;
  onSelect: (id: VisualId | null) => void;
}) {
  const from = nodeById(scene, relationship.fromId);
  const to = relationship.toId ? nodeById(scene, relationship.toId) : undefined;
  const missing = !to && relationship.kind === 'requires';
  return (
    <li className="flex flex-wrap items-center gap-2 rounded-lg border border-border bg-card px-3 py-2">
      <span className="text-sm font-medium text-foreground">{from?.name ?? relationship.fromId}</span>
      <RelationshipBadge kind={relationship.kind} />
      {missing ? (
        <>
          <span className="inline-flex items-center gap-1 rounded-md border border-dashed border-destructive/70 bg-destructive/10 px-2 py-0.5 text-xs font-semibold text-destructive">
            <span aria-hidden="true">⌁</span> Missing requirement
          </span>
          <span className="text-xs text-muted-foreground">{relationship.explanation}</span>
          {relationship.affectedCount !== undefined ? (
            <span className="text-xs text-muted-foreground">affects {relationship.affectedCount}</span>
          ) : null}
        </>
      ) : (
        <>
          <button
            type="button"
            onClick={() => onSelect(to ? to.id : null)}
            className="text-sm font-medium text-foreground underline decoration-dotted underline-offset-2 hover:text-primary"
          >
            {to?.name ?? 'unknown item'}
          </button>
          <span className="text-xs text-muted-foreground">{relationship.explanation}</span>
        </>
      )}
    </li>
  );
}

export function ContentGraph({
  scene,
  source,
  selection,
  onSelect,
  onIntent,
  capabilities,
}: ContentGraphProps) {
  const { linear, setLinear } = useLinearView(false);
  const missingRequirements = scene.relationships.filter(
    (relationship) => relationship.kind === 'requires' && !relationship.toId,
  );
  const activeConflicts = scene.relationships.filter(
    (relationship) => relationship.kind === 'conflicts-with' && relationship.state === 'conflicting',
  );

  return (
    <section aria-label="Content graph" className="space-y-3" data-source={source.kind}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-base font-bold text-foreground">What relies on what</h3>
        <LinearViewToggle linear={linear} onChange={setLinear} />
      </div>

      {missingRequirements.length > 0 ? (
        <div className="rounded-lg border border-destructive/50 bg-destructive/5 p-3" role="region" aria-label="Missing requirements">
          <h4 className="flex items-center gap-2 text-sm font-bold text-destructive">
            <span aria-hidden="true">⌁</span> Missing requirements
          </h4>
          <ul className="mt-2 space-y-2">
            {missingRequirements.map((relationship) => (
              <RelationshipRow key={relationship.id} relationship={relationship} scene={scene} onSelect={onSelect} />
            ))}
          </ul>
        </div>
      ) : null}

      {activeConflicts.length > 0 ? (
        <div className="rounded-lg border border-amber-500/50 bg-amber-500/5 p-3" role="region" aria-label="Conflicts">
          <h4 className="text-sm font-bold text-amber-700 dark:text-amber-300">Conflicts</h4>
          <ul className="mt-2 space-y-2">
            {activeConflicts.map((relationship) => (
              <RelationshipRow key={relationship.id} relationship={relationship} scene={scene} onSelect={onSelect} />
            ))}
          </ul>
        </div>
      ) : null}

      <ul className={`${linear ? 'space-y-2' : 'grid grid-cols-1 gap-3 sm:grid-cols-2'}`} aria-label="Content">
        {scene.content.map((node) => {
          const selected = selection === node.id;
          const proposedPresence = node.presence.proposed;
          const proposedEnabled = node.enabled.proposed;
          const requires = node.relationshipSummary.requires;
          const conflicts = node.relationshipSummary.conflicts;
          return (
            <li
              key={node.id}
              className={`rounded-xl border bg-card p-3 ${selected ? 'border-primary ring-1 ring-primary/40' : 'border-border'}`}
            >
              <div className="flex items-start justify-between gap-2">
                <button
                  type="button"
                  onClick={() => onSelect(selected ? null : node.id)}
                  aria-pressed={selected}
                  className="text-left"
                >
                  <span className="block text-sm font-bold text-foreground">{node.name}</span>
                  <span className="text-xs text-muted-foreground">{KIND_LABEL[node.kind] ?? node.kind}</span>
                </button>
                {proposedPresence !== undefined || proposedEnabled !== undefined ? <ProposedMark /> : null}
              </div>

              {proposedPresence !== undefined && proposedPresence !== node.presence.current ? (
                <p className="mt-1 text-xs font-medium text-foreground">
                  {proposedPresence === 'installed' ? 'Proposed: install' : 'Proposed: remove'}
                </p>
              ) : null}

              {!linear ? <SocketStrip nodeId={node.id} scene={scene} onSelect={onSelect} /> : null}

              <div className="mt-2 flex flex-wrap items-center gap-2">
                {node.health === 'blocked' ? <SeverityBadge severity="blocker" /> : null}
                {node.health === 'needs-attention' ? <SeverityBadge severity="warning" /> : null}
                {requires > 0 ? (
                  <span className="text-xs text-muted-foreground">requires {requires}</span>
                ) : null}
                {conflicts > 0 ? (
                  <span className="text-xs text-destructive">conflicts {conflicts}</span>
                ) : null}
              </div>

              {node.availability === 'locked' || node.availability === 'busy' ? (
                <p className="mt-1 text-xs font-medium text-muted-foreground">
                  {node.availability === 'locked' ? 'Locked' : 'Busy'} — reason below
                </p>
              ) : null}

              {capabilities.canProposeInstall && node.presence.current === 'not-installed' ? (
                <button
                  type="button"
                  onClick={() => onIntent({ kind: 'propose-install', contentId: node.id })}
                  className="mt-2 rounded-md bg-primary px-2.5 py-1 text-xs font-semibold text-primary-foreground hover:opacity-90"
                >
                  Stage install
                </button>
              ) : null}
              {capabilities.canProposeRemove && node.presence.current === 'installed' ? (
                <button
                  type="button"
                  onClick={() => onIntent({ kind: 'propose-remove', contentId: node.id })}
                  className="mt-2 rounded-md border border-destructive/60 px-2.5 py-1 text-xs font-semibold text-destructive hover:bg-destructive/10"
                >
                  Stage removal
                </button>
              ) : null}
            </li>
          );
        })}
      </ul>

      {linear && scene.relationships.length > 0 ? (
        <div>
          <h4 className="mb-2 text-xs font-bold uppercase tracking-wide text-muted-foreground">Relationships</h4>
          <ul className="space-y-2">
            {scene.relationships.map((relationship) => (
              <RelationshipRow key={relationship.id} relationship={relationship} scene={scene} onSelect={onSelect} />
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  );
}
