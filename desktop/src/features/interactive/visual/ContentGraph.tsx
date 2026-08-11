import { useMemo, useState } from 'react';
import type { CapabilityFlags, VisualId, VisualRelationship, VisualScene } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { LinearViewToggle, useLinearView } from './primitives/LinearView';
import { RelationshipBadge, SeverityBadge } from './primitives/statusChips';
import { ProposedMark } from './primitives/stateMarks';

/**
 * Content Graph — "What relies on what, and what would this change affect?"
 *
 * Renders `VisualContentNode`s and `VisualRelationship`s with EXPLICIT
 * four-state semantics (satisfied, missing/conflicting, indeterminate, unknown)
 * in both diagram (socket chips) and linear views — the state is never inferred
 * from prose or from whether a target node exists (FIX BEFORE LIVE MODE 2).
 *
 * Single source authority: the scene carries its own `source`; there is no
 * separate source prop (FIX BEFORE LIVE MODE 1).
 *
 * Large-instance readiness (FIX BEFORE LIVE MODE 3): node lookup and
 * relationship grouping are memoized per scene revision, the spatial viewport
 * is capped with deliberate disclosure, the linear view is complete and
 * searchable, and the node list uses roving focus (arrow keys).
 *
 * Duplicate/unavailable actions (FIX BEFORE LIVE MODE 1): a stage action is not
 * offered when an identical proposal already exists or the node is not
 * available; unavailable nodes show a persistent reason instead.
 *
 * A controlled component: emits `VisualIntent`s only, never calls Tauri.
 */

export interface ContentGraphProps {
  scene: VisualScene;
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

const SPATIAL_CAP = 12;

type StateTone = 'good' | 'bad' | 'warn' | 'neutral';

function relationshipState(
  kind: VisualRelationship['kind'],
  state: VisualRelationship['state'],
): { label: string; tone: StateTone } {
  switch (state) {
    case 'satisfied':
      return { label: kind === 'conflicts-with' ? 'Resolved' : 'Satisfied', tone: 'good' };
    case 'missing':
      return { label: kind === 'conflicts-with' ? 'Unknown' : 'Missing', tone: 'bad' };
    case 'conflicting':
      return { label: 'Blocking', tone: 'bad' };
    case 'indeterminate':
      return { label: 'Needs review', tone: 'warn' };
    default:
      return { label: 'Unknown', tone: 'neutral' };
  }
}

const TONE_CLASS: Record<StateTone, string> = {
  good: 'border-emerald-600/50 bg-emerald-600/5 text-emerald-700 dark:text-emerald-300',
  bad: 'border-destructive/70 bg-destructive/10 text-destructive',
  warn: 'border-amber-500/70 bg-amber-500/10 text-amber-700 dark:text-amber-300',
  neutral: 'border-border bg-muted/40 text-muted-foreground',
};

function StateBadge({ kind, state }: { kind: VisualRelationship['kind']; state: VisualRelationship['state'] }) {
  const { label, tone } = relationshipState(kind, state);
  return (
    <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[0.7rem] font-semibold ${TONE_CLASS[tone]}`}>
      {label}
    </span>
  );
}

/** Memoized per-revision lookup: node id -> node, relationships grouped by fromId. */
function useGraphIndex(scene: VisualScene) {
  return useMemo(() => {
    const nodeById = new Map<VisualId, (typeof scene.content)[number]>();
    for (const node of scene.content) nodeById.set(node.id, node);
    const byFrom = new Map<VisualId, VisualRelationship[]>();
    for (const relationship of scene.relationships) {
      const list = byFrom.get(relationship.fromId) ?? [];
      list.push(relationship);
      byFrom.set(relationship.fromId, list);
    }
    const byTo = new Map<VisualId, VisualRelationship[]>();
    for (const relationship of scene.relationships) {
      if (relationship.toId) {
        const list = byTo.get(relationship.toId) ?? [];
        list.push(relationship);
        byTo.set(relationship.toId, list);
      }
    }
    return { nodeById, byFrom, byTo };
  }, [scene]);
}

/**
 * A socket chip renders one relationship at a node with an explicit
 * filled/empty socket and text label. All four states are explicit.
 */
function SocketChip({
  relationship,
  nodeById,
  onSelect,
}: {
  relationship: VisualRelationship;
  nodeById: Map<VisualId, { id: VisualId; name: string }>;
  onSelect: (id: VisualId | null) => void;
}) {
  const to = relationship.toId ? nodeById.get(relationship.toId) : undefined;
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
  const { tone } = relationshipState(relationship.kind, relationship.state);
  const socket =
    relationship.state === 'satisfied'
      ? { mark: '●', cls: TONE_CLASS.good }
      : relationship.state === 'indeterminate'
        ? { mark: '◑', cls: TONE_CLASS.warn }
        : { mark: '◌', cls: TONE_CLASS[tone] };

  return (
    <li
      className={`inline-flex max-w-full flex-wrap items-center gap-1.5 rounded-md border px-2 py-1 text-xs font-medium text-foreground ${socket.cls}`}
    >
      <span aria-hidden="true">{socket.mark}</span>
      <span>
        {relationship.kind === 'requires'
          ? 'Requires'
          : relationship.kind === 'recommends'
            ? 'Recommends'
            : 'Conflicts with'}{' '}
        {target}
      </span>
      <StateBadge kind={relationship.kind} state={relationship.state} />
    </li>
  );
}

/** Socket strip on a node card: outgoing relationships + incoming requires. */
function SocketStrip({
  nodeId,
  byFrom,
  byTo,
  nodeById,
  onSelect,
}: {
  nodeId: VisualId;
  byFrom: Map<VisualId, VisualRelationship[]>;
  byTo: Map<VisualId, VisualRelationship[]>;
  nodeById: Map<VisualId, { id: VisualId; name: string }>;
  onSelect: (id: VisualId | null) => void;
}) {
  const outgoing = byFrom.get(nodeId) ?? [];
  const requiredBy = (byTo.get(nodeId) ?? []).filter((relationship) => relationship.kind === 'requires');
  if (outgoing.length === 0 && requiredBy.length === 0) return null;
  const nodeName = nodeById.get(nodeId)?.name ?? nodeId;
  return (
    <ul className="mt-2 space-y-1" aria-label={`Relationships for ${nodeName}`}>
      {outgoing.map((relationship) => (
        <SocketChip key={relationship.id} relationship={relationship} nodeById={nodeById} onSelect={onSelect} />
      ))}
      {requiredBy.length > 0 ? (
        <li className="text-xs text-muted-foreground">
          Required by{' '}
          {requiredBy.map((relationship, index) => {
            const from = nodeById.get(relationship.fromId);
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
  nodeById,
  onSelect,
}: {
  relationship: VisualRelationship;
  nodeById: Map<VisualId, { id: VisualId; name: string }>;
  onSelect: (id: VisualId | null) => void;
}) {
  const from = nodeById.get(relationship.fromId);
  const to = relationship.toId ? nodeById.get(relationship.toId) : undefined;
  return (
    <li className="flex flex-wrap items-center gap-2 rounded-lg border border-border bg-card px-3 py-2">
      <span className="text-sm font-medium text-foreground">{from?.name ?? relationship.fromId}</span>
      <RelationshipBadge kind={relationship.kind} />
      {relationship.toId ? (
        <button
          type="button"
          onClick={() => onSelect(relationship.toId ?? null)}
          className="text-sm font-medium text-foreground underline decoration-dotted underline-offset-2 hover:text-primary"
        >
          {to?.name ?? 'unknown item'}
        </button>
      ) : (
        <span className="text-sm font-medium text-muted-foreground">missing item</span>
      )}
      <StateBadge kind={relationship.kind} state={relationship.state} />
      <span className="text-xs text-muted-foreground">{relationship.explanation}</span>
    </li>
  );
}

export function ContentGraph({ scene, selection, onSelect, onIntent, capabilities }: ContentGraphProps) {
  const { linear, setLinear } = useLinearView(false);
  const { nodeById, byFrom, byTo } = useGraphIndex(scene);
  const [showAll, setShowAll] = useState(false);
  const [query, setQuery] = useState('');
  const [focusIndex, setFocusIndex] = useState<number>(0);
  const source = scene.source;

  const missingRequirements = scene.relationships.filter(
    (relationship) => relationship.kind === 'requires' && relationship.state === 'missing',
  );
  const activeConflicts = scene.relationships.filter(
    (relationship) => relationship.kind === 'conflicts-with' && relationship.state === 'conflicting',
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return scene.content;
    return scene.content.filter((node) => node.name.toLowerCase().includes(q));
  }, [scene.content, query]);

  const visibleNodes = linear ? filtered : filtered.slice(0, showAll ? undefined : SPATIAL_CAP);
  const hiddenCount = filtered.length - visibleNodes.length;

  const proposalFor = (nodeId: VisualId) =>
    scene.proposals.find((proposal) => {
      const intent = proposal.intent;
      return (
        (intent.kind === 'propose-install' || intent.kind === 'propose-remove' || intent.kind === 'propose-update')
        && 'contentId' in intent
        && intent.contentId === nodeId
      );
    });

  return (
    <section aria-label="Content graph" className="space-y-3" data-source={source.kind}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-base font-bold text-foreground">What relies on what</h3>
        <LinearViewToggle linear={linear} onChange={setLinear} />
      </div>

      {linear ? (
        <div className="flex flex-wrap items-center gap-2">
          <label className="text-xs font-semibold text-muted-foreground" htmlFor="content-graph-search">
            Search content
          </label>
          <input
            id="content-graph-search"
            type="search"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setShowAll(false);
            }}
            placeholder="Filter by name…"
            className="w-56 max-w-full rounded-md border border-border bg-card px-2.5 py-1 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
          />
        </div>
      ) : null}

      {missingRequirements.length > 0 ? (
        <div className="rounded-lg border border-destructive/50 bg-destructive/5 p-3" role="region" aria-label="Missing requirements">
          <h4 className="flex items-center gap-2 text-sm font-bold text-destructive">
            <span aria-hidden="true">⌁</span> Missing requirements
          </h4>
          <ul className="mt-2 space-y-2">
            {missingRequirements.map((relationship) => (
              <RelationshipRow key={relationship.id} relationship={relationship} nodeById={nodeById} onSelect={onSelect} />
            ))}
          </ul>
        </div>
      ) : null}

      {activeConflicts.length > 0 ? (
        <div className="rounded-lg border border-amber-500/50 bg-amber-500/5 p-3" role="region" aria-label="Conflicts">
          <h4 className="text-sm font-bold text-amber-700 dark:text-amber-300">Conflicts</h4>
          <ul className="mt-2 space-y-2">
            {activeConflicts.map((relationship) => (
              <RelationshipRow key={relationship.id} relationship={relationship} nodeById={nodeById} onSelect={onSelect} />
            ))}
          </ul>
        </div>
      ) : null}

      <ul
        className={`${linear ? 'space-y-2' : 'grid grid-cols-1 gap-3 sm:grid-cols-2'}`}
        aria-label="Content"
        onKeyDown={(event) => {
          if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
          event.preventDefault();
          const delta = event.key === 'ArrowDown' ? 1 : -1;
          const next = Math.min(visibleNodes.length - 1, Math.max(0, focusIndex + delta));
          setFocusIndex(next);
          const node = visibleNodes[next];
          if (node) onSelect(node.id);
        }}
      >
        {visibleNodes.map((node, index) => {
          const selected = selection === node.id;
          const proposedPresence = node.presence.proposed;
          const proposedEnabled = node.enabled.proposed;
          const requires = node.relationshipSummary.requires;
          const conflicts = node.relationshipSummary.conflicts;
          const existingProposal = proposalFor(node.id);
          // A node can carry a proposed change without a matching entry in the
          // staging dock (the scene may project it directly). Both count as
          // "already proposed" — otherwise the stage button stays enabled and
          // does nothing at all when clicked (T6-7, a silent no-op).
          const nodeShowsProposal =
            (proposedPresence !== undefined && proposedPresence !== node.presence.current) ||
            (proposedEnabled !== undefined && proposedEnabled !== node.enabled.current);
          const alreadyProposed = Boolean(existingProposal) || nodeShowsProposal;
          const availabilityReason =
            node.availability === 'locked'
              ? 'Locked — this item cannot be changed right now.'
              : node.availability === 'busy'
                ? 'Busy — an operation is using this item.'
                : node.availability === 'unavailable'
                  ? 'Unavailable — this item cannot be changed right now.'
                  : null;
          const canStageInstall =
            capabilities.canProposeInstall && node.presence.current === 'not-installed' && !alreadyProposed && !availabilityReason;
          const canStageRemove =
            capabilities.canProposeRemove && node.presence.current === 'installed' && !alreadyProposed && !availabilityReason;
          return (
            <li
              key={node.id}
              className={`rounded-xl border bg-card p-3 ${selected ? 'border-primary ring-1 ring-primary/40' : 'border-border'}`}
            >
              <div className="flex items-start justify-between gap-2">
                <button
                  type="button"
                  onClick={() => onSelect(selected ? null : node.id)}
                  onFocus={() => setFocusIndex(index)}
                  aria-pressed={selected}
                  tabIndex={focusIndex === index ? 0 : -1}
                  className="text-left"
                >
                  <span className="block text-sm font-bold text-foreground">{node.name}</span>
                  <span className="text-xs text-muted-foreground">{KIND_LABEL[node.kind] ?? node.kind}</span>
                  {/* The exact filename stays visible whenever `name` is derived,
                      so the friendly label is never mistaken for authority. */}
                  {node.fileLabel ? (
                    <span className="block break-all text-[0.7rem] text-muted-foreground">{node.fileLabel}</span>
                  ) : null}
                </button>
                {proposedPresence !== undefined || proposedEnabled !== undefined ? <ProposedMark /> : null}
              </div>

              {proposedPresence !== undefined && proposedPresence !== node.presence.current ? (
                <p className="mt-1 text-xs font-medium text-foreground">
                  {proposedPresence === 'installed' ? 'Proposed: install' : 'Proposed: remove'}
                </p>
              ) : null}

              {!linear ? <SocketStrip nodeId={node.id} byFrom={byFrom} byTo={byTo} nodeById={nodeById} onSelect={onSelect} /> : null}

              <div className="mt-2 flex flex-wrap items-center gap-2">
                {node.health === 'blocked' ? <SeverityBadge severity="blocker" /> : null}
                {node.health === 'needs-attention' ? <SeverityBadge severity="warning" /> : null}
                {requires > 0 ? <span className="text-xs text-muted-foreground">requires {requires}</span> : null}
                {conflicts > 0 ? <span className="text-xs text-destructive">conflicts {conflicts}</span> : null}
              </div>

              {availabilityReason ? (
                <p className="mt-1 text-xs font-medium text-muted-foreground" role="status">
                  {availabilityReason}
                </p>
              ) : null}

              {/* Accessible names carry the TARGET, not just the verb: a
                  screen-reader button list of 136 identical "Stage install"
                  entries is unusable (T6-9). */}
              {canStageInstall ? (
                <button
                  type="button"
                  onClick={() => onIntent({ kind: 'propose-install', contentId: node.id })}
                  aria-label={`Stage install: ${node.name}`}
                  className="mt-2 rounded-md bg-primary px-2.5 py-1 text-xs font-semibold text-primary-foreground hover:opacity-90"
                >
                  Stage install
                </button>
              ) : null}
              {canStageRemove ? (
                <button
                  type="button"
                  onClick={() => onIntent({ kind: 'propose-remove', contentId: node.id })}
                  aria-label={`Stage removal: ${node.name}`}
                  className="mt-2 rounded-md border border-destructive/60 px-2.5 py-1 text-xs font-semibold text-destructive hover:bg-destructive/10"
                >
                  Stage removal
                </button>
              ) : null}
              {!canStageInstall && !canStageRemove && availabilityReason === null && alreadyProposed ? (
                <p className="mt-1 text-xs font-medium text-muted-foreground">Already proposed — review it in the staging dock.</p>
              ) : null}
            </li>
          );
        })}
      </ul>

      {!linear && hiddenCount > 0 ? (
        <button
          type="button"
          onClick={() => setShowAll(true)}
          className="rounded-md border border-border bg-card px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
        >
          Show all {filtered.length} items ({hiddenCount} more)
        </button>
      ) : null}

      {filtered.length === 0 ? <p className="text-sm text-muted-foreground">No content matches your search.</p> : null}

      {linear && scene.relationships.length > 0 ? (
        <div>
          <h4 className="mb-2 text-xs font-bold uppercase tracking-wide text-muted-foreground">Relationships</h4>
          <ul className="space-y-2">
            {scene.relationships.map((relationship) => (
              <RelationshipRow key={relationship.id} relationship={relationship} nodeById={nodeById} onSelect={onSelect} />
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  );
}
