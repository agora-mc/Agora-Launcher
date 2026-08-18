/**
 * WorldEditor data mapping — turns the live `VisualScene` into the v4-world
 * foreground's item model (name/kind/needs/neededBy/missing/rarity). Pure.
 *
 * The prototype's shelf derived rarity from `neededBy` count; here that is
 * `relationshipSummary.requiredBy`. Needs/neededBy resolve relationship
 * endpoints to node names so the detail drawer and links render real names.
 */

import type { ContentKind, VisualScene } from '../domain/models';

export type EditorItemKind = 'mod' | 'look' | 'world';

export interface EditorItem {
  id: string;
  name: string;
  kind: EditorItemKind;
  /** Names of the nodes this item requires. */
  needs: string[];
  /** Names of the nodes that require this item. */
  neededBy: string[];
  /** Names of optional dependencies this item recommends (kind: recommends). */
  optional: string[];
  missing: boolean;
  rarity: 'common' | 'rare' | 'epic' | 'legendary';
  iconUrl?: string;
  catalogIds?: { registryId: string | null; modrinthId: string | null; modJarId?: string | null };
  /** Whether the content is installed in the instance. */
  presence: 'installed' | 'not-installed';
}

export const KIND_LABEL: Record<EditorItemKind, string> = {
  mod: 'A mod',
  look: 'Looks and textures',
  world: 'World content',
};

export function toEditorKind(kind: ContentKind): EditorItemKind {
  if (kind === 'mod' || kind === 'modpack') return 'mod';
  if (kind === 'resource-pack' || kind === 'shader') return 'look';
  return 'world';
}

export interface EditorData {
  items: EditorItem[];
  byId: Map<string, EditorItem>;
  counts: Record<string | 'all', number>;
  total: number;
  /** The item that is missing its file (first health-blocked node), if any. */
  missingItem: EditorItem | null;
  /** Whether a real crash investigation exists (status pill → crash doctor). */
  hasCrash: boolean;
}

/** Map a VisualScene (with an optional crash flag) to editor items. */
export function buildEditorData(scene: VisualScene, hasCrash: boolean): EditorData {
  const nameById = new Map(scene.content.map((node) => [node.id, node.name]));

  const items: EditorItem[] = scene.content.map((node) => {
    const needs = scene.relationships
      .filter((r) => r.fromId === node.id && r.kind === 'requires' && r.toId && nameById.has(r.toId))
      .map((r) => nameById.get(r.toId!)!)
      .filter((name, i, arr) => arr.indexOf(name) === i);
    const neededBy = scene.relationships
      .filter((r) => r.toId === node.id && r.kind === 'requires' && r.toId && nameById.has(r.fromId))
      .map((r) => nameById.get(r.fromId)!)
      .filter((name, i, arr) => arr.indexOf(name) === i);
    // Optional dependencies = `recommends` relationships (the prototype's
    // "optional extras these mods can use").
    const optional = scene.relationships
      .filter((r) => r.fromId === node.id && r.kind === 'recommends' && r.toId && nameById.has(r.toId))
      .map((r) => nameById.get(r.toId!)!)
      .filter((name, i, arr) => arr.indexOf(name) === i);
    const requiredByCount = node.relationshipSummary.requiredBy;
    return {
      id: node.id,
      name: node.name,
      kind: toEditorKind(node.kind),
      needs,
      neededBy,
      optional,
      missing: node.health === 'needs-attention' || node.health === 'blocked',
      ...(node.iconUrl ? { iconUrl: node.iconUrl } : {}),
      ...(node.catalogIds ? { catalogIds: node.catalogIds } : {}),
      rarity: requiredByCount >= 10 ? 'legendary' : requiredByCount >= 4 ? 'epic' : requiredByCount >= 1 ? 'rare' : 'common',
      presence: node.presence.current === 'installed' ? 'installed' : 'not-installed',
    };
  });

  const counts: Record<string | 'all', number> = { all: items.length, mod: 0, look: 0, world: 0 };
  items.forEach((it) => { counts[it.kind] = (counts[it.kind] ?? 0) + 1; });

  const missingItem = items.find((it) => it.missing) ?? null;

  return {
    items,
    // Keyed by BOTH id and display name: the shelf selects by `data-name` and
    // the host routes intents by id, so both lookups must resolve (F-name-id).
    byId: new Map<string, EditorItem>(
      items.flatMap((it) => [[it.id, it], [it.name, it]] as Array<[string, EditorItem]>),
    ),
    counts,
    total: items.length,
    missingItem,
    hasCrash,
  };
}

/** Deterministic monogram + hue (the prototype's `hue`/`mono`/`paint`). */
export function hueOf(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) % 360;
  return h;
}

export function monoOf(name: string): string {
  const t = name.replace(/[^A-Za-z0-9 ]/g, ' ').trim().split(/\s+/);
  return ((t[0] || '?')[0] ?? '?') + (t[1] ? t[1][0] ?? '' : '').toUpperCase();
}

export function tileBackground(name: string): string {
  const h = hueOf(name);
  return `linear-gradient(160deg, hsl(${h} 66% 60%), hsl(${(h + 38) % 360} 62% 38%))`;
}
