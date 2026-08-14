/**
 * Live read adapters: map backend DTOs into the minimal presentation models.
 *
 * Only `live/` may map backend DTOs (MASTER_ARCHITECTURE §8). These adapters
 * are pure and REDACT private authority data — hashes, paths, fingerprints,
 * scan tokens, receipts, and raw manifests never enter a presentation model.
 * Uncertainty is preserved (`unknown` / `indeterminate`), never invented.
 *
 * Read-only by construction: no mutation command is called here.
 */

import { fetchModrinthProject, getRegistryItem, isModrinthEnabled } from '@/lib/tauri';
import type {
  CrashInvestigation,
  DependencyEdge,
  HealthReport,
  InstanceDetail,
  InstanceRow,
  JavaRuntimeSummary,
  MemoryRecommendation,
  RunningProcess,
  Snapshot,
} from '@/lib/tauri';
import type {
  VisualCrashEvidence,
  VisualHealthFinding,
  VisualInstance,
  VisualRelationship,
  VisualRuntimeState,
  VisualSnapshot,
} from '../../domain/models';

/** Stable opaque identity for an installed-content row (never an absolute path). */
function nodeIdFor(contentType: string, filename: string): string {
  return `live:content:${contentType}:${filename}`;
}

/**
 * Null-safe identity comparison (SOL §22.2).
 *
 * `HealthBlocker.mod_id` / `HealthWarning.mod_id` and `InstalledMod.registry_id`
 * are all `string | null`. A plain `===` makes `null === null` true, so a
 * finding Agora could NOT attribute to a specific mod matched every mod Agora
 * could NOT attribute to the registry — one unattributed warning marked an
 * entire instance. Identity only counts when BOTH sides are known.
 */
function sameIdentity(a: string | null | undefined, b: string | null | undefined): boolean {
  return a != null && b != null && a === b;
}

/**
 * True when a finding names no specific content. Such findings describe the
 * INSTANCE (e.g. "the manifest tracks 1 enabled mod file absent from mods/")
 * and must never be projected onto content nodes.
 */
function isInstanceLevel(finding: { filename: string | null; mod_id: string | null }): boolean {
  return finding.filename == null && finding.mod_id == null;
}

const CONTENT_EXTENSIONS = /\.(jar|zip)(\.disabled)?$/i;
const LOADER_TOKENS = /^(fabric|forge|quilt|neoforge|neo|mc|minecraft|all)$/i;

/**
 * Human-readable label derived from a content filename.
 *
 * `InstalledMod` carries no display name, and SOL §22.5 forbids adding a new
 * live read command for one. The derived label is therefore a CONVENIENCE: the
 * exact filename is always retained alongside it (`fileLabel`), so the
 * derivation is never presented as an authoritative name. Returns null when
 * nothing meaningful can be derived, in which case callers show the filename.
 */
export function displayNameFromFilename(filename: string): string | null {
  const base = filename.replace(CONTENT_EXTENSIONS, '');
  const tokens: string[] = [];
  for (const token of base.split(/[-_]+/)) {
    if (!token) continue;
    // Stop at the first version-ish or loader token: everything after it is
    // packaging metadata, not the name.
    if (/^v?\d/.test(token) || token.includes('+') || LOADER_TOKENS.test(token)) break;
    tokens.push(token);
  }
  if (tokens.length === 0) return null;
  const words = tokens
    .flatMap((token) => token.replace(/([a-z0-9])([A-Z])/g, '$1 $2').split(/\s+/))
    .filter(Boolean)
    .map((word) => (/^[a-z]/.test(word) ? word.charAt(0).toUpperCase() + word.slice(1) : word));
  const label = words.join(' ').trim();
  return label.length > 0 && label.toLowerCase() !== base.toLowerCase() ? label : null;
}

// --- Instance view ---
export function instanceToVisual(
  detail: InstanceDetail,
  running: RunningProcess | null,
  processUnknown = false,
): VisualInstance {
  const mods = detail.manifest?.mods ?? [];
  const enabled = mods.filter((mod) => mod.enabled).length;
  // Process-state uncertainty is treated conservatively as non-editable/busy
  // (SOL-2 BLOCKER 1): we never claim editable when we could not read the
  // process state.
  const lockState: VisualInstance['lockState'] = processUnknown
    ? 'busy'
    : running?.instance_id === detail.row.instance_id
      ? 'busy'
      : detail.row.is_locked
        ? 'locked-by-player'
        : 'editable';
  const recoveryReadiness: VisualInstance['recoveryReadiness'] =
    detail.snapshot_readiness === 'ready'
      ? 'ready'
      : detail.snapshot_readiness === 'pending'
        ? 'preparing'
        : 'failed';
  return {
    id: detail.row.instance_id,
    name: detail.row.name,
    gameVersion: detail.row.minecraft_version,
    loader: {
      current: {
        family: detail.row.loader || 'Vanilla',
        ...(detail.row.loader_version ? { version: detail.row.loader_version } : {}),
        compatibility: 'unknown', // loader compatibility requires a health/compat read
      },
    },
    lockState,
    recoveryReadiness,
    launchState: running?.instance_id === detail.row.instance_id ? 'running' : 'idle',
    contentSummary: {
      enabled,
      disabled: mods.length - enabled,
      needsAttention: 0,
    },
  };
}

// --- Content nodes + relationships ---
export function contentToVisual(
  detail: InstanceDetail,
  health: HealthReport | null,
  healthKnown = true,
  /**
   * Mod-to-mod dependency edges from `getDependencyGraph`.
   *
   * Without these the ONLY relationships available are loader-version
   * requirements lifted out of health blockers — which exist solely when the
   * instance is unhealthy, and whose targets are loader ids that rarely resolve
   * to another installed node. That left a healthy instance with an empty graph:
   * no dependency curves to draw, and `requiredBy` stuck at 0 for everything,
   * so every item came out `common`.
   */
  dependencyEdges: DependencyEdge[] = [],
): { content: import('../../domain/models').VisualContentNode[]; relationships: VisualRelationship[] } {
  const manifest = detail.manifest;
  const mods = manifest?.mods ?? [];
  const content = mods.map((mod) => {
    const id = nodeIdFor(mod.content_type, mod.filename);
    // When the health read failed we do NOT mark nodes healthy from absence
    // (SOL-2 BLOCKER 1): health becomes `unknown`.
    // Identity matching is null-safe and instance-level findings are excluded
    // (SOL §22.2): an unattributed finding describes the instance, not content.
    const finding = !healthKnown
      ? ('unknown' as const)
      : (health?.blockers ?? []).some(
          (blocker) =>
            !isInstanceLevel(blocker) &&
            (sameIdentity(blocker.filename, mod.filename) || sameIdentity(blocker.mod_id, mod.registry_id)),
        )
        ? 'blocked'
        : (health?.warnings ?? []).some(
            (warning) =>
              !isInstanceLevel(warning) &&
              (sameIdentity(warning.filename, mod.filename) || sameIdentity(warning.mod_id, mod.registry_id)),
          )
          ? 'needs-attention'
          : 'healthy';
    const derivedName = displayNameFromFilename(mod.filename);
    return {
      id,
      name: derivedName ?? mod.filename,
      // The authoritative on-disk filename is always retained beside the
      // derived label so the derivation is never mistaken for a real name.
      ...(derivedName ? { fileLabel: mod.filename } : {}),
      kind: (mod.content_type as import('../../domain/models').ContentKind) ?? 'mod',
      ...(mod.version ? { version: { current: mod.version } } : {}),
      presence: { current: 'installed' as const },
      enabled: { current: mod.enabled },
      catalogIds: { registryId: mod.registry_id, modrinthId: mod.modrinth_id },
      health: finding as 'healthy' | 'needs-attention' | 'blocked' | 'unknown',
      relationshipSummary: { requiredBy: 0, requires: 0, conflicts: 0 },
      availability: 'available' as const,
    };
  });

  const relationships: VisualRelationship[] = [];
  const byFilename = new Map(mods.map((mod) => [mod.filename, nodeIdFor(mod.content_type, mod.filename)]));
  // O(nodes + relationships): id lookups and requirement-target resolution are
  // map-based; summaries are accumulated in one pass over the relationships.
  const idToNodeId = new Map<string, VisualRelationship['fromId']>();
  for (const node of content) idToNodeId.set(node.id.split(':').pop() ?? '', node.id);
  for (const blocker of health?.blockers ?? []) {
    const issue = blocker.loader_compatibility;
    if (!issue) continue;
    for (const requirement of issue.requirements) {
      const declaringNames = requirement.declaring_mod_ids ?? (requirement.declaring_mod_id ? [requirement.declaring_mod_id] : []);
      const declaringId = declaringNames.map((name) => byFilename.get(name)).find((id) => id !== undefined);
      const fromId = declaringId ?? (blocker.filename ? byFilename.get(blocker.filename) : undefined);
      if (!fromId) continue;
      const targetVisible = idToNodeId.get(requirement.target_id);
      relationships.push({
        id: `live:rel:${fromId}:${requirement.target_id}`,
        fromId,
        ...(targetVisible ? { toId: targetVisible } : {}),
        kind: requirement.importance === 'required' ? 'requires' : 'recommends',
        state: requirement.verdict === 'satisfied'
          ? 'satisfied'
          : requirement.verdict === 'unsatisfied'
            ? 'missing'
            : 'indeterminate',
        importance: requirement.importance === 'required' ? 'required' : 'recommended',
        explanation: `Loader requirement: ${requirement.target_id}`,
      });
    }
  }
  // Real mod-to-mod edges. Both endpoints are installed by construction (core
  // drops unresolved declarations), so every one of these is drawable.
  for (const edge of dependencyEdges) {
    const fromId = byFilename.get(edge.from_filename);
    const toId = byFilename.get(edge.to_filename);
    if (!fromId || !toId || fromId === toId) continue;
    const required = edge.requirement === 'required';
    relationships.push({
      id: `live:dep:${fromId}:${toId}`,
      fromId,
      toId,
      kind: required ? 'requires' : 'recommends',
      state: 'satisfied',
      importance: required ? 'required' : 'recommended',
      explanation: required
        ? `${edge.from_filename} requires ${edge.to_filename}`
        : `${edge.from_filename} can use ${edge.to_filename}`,
    });
  }

  // Single pass to compute per-node summaries.
  const requiresByNode = new Map<string, number>();
  const requiredByByNode = new Map<string, number>();
  for (const rel of relationships) {
    requiresByNode.set(rel.fromId, (requiresByNode.get(rel.fromId) ?? 0) + (rel.kind === 'requires' ? 1 : 0));
    if (rel.toId && rel.kind === 'requires') {
      requiredByByNode.set(rel.toId, (requiredByByNode.get(rel.toId) ?? 0) + 1);
    }
  }
  for (const node of content) {
    node.relationshipSummary = {
      requires: requiresByNode.get(node.id) ?? 0,
      requiredBy: requiredByByNode.get(node.id) ?? 0,
      conflicts: 0,
    };
  }
  return { content, relationships };
}

/**
 * Short headline for a finding (SOL §22 / T6-12).
 *
 * `title` and `summary` were both set to the raw backend message, so every
 * finding printed the same sentence twice. The headline names WHAT and WHERE;
 * the full message stays in `summary`.
 */
function findingTitle(
  finding: { kind: string; filename: string | null; mod_id: string | null; message: string },
  fallback: string,
): string {
  const subject = finding.filename
    ? (displayNameFromFilename(finding.filename) ?? finding.filename)
    : finding.mod_id;
  const kindLabel = finding.kind
    ? finding.kind.replace(/[_-]+/g, ' ').replace(/^./, (c) => c.toUpperCase())
    : fallback;
  return subject ? `${kindLabel} — ${subject}` : kindLabel;
}

/**
 * Category for a recommendation. Previously hardcoded to `runtime`, which
 * mislabelled every content/dependency recommendation (T6-12). Anything that
 * names a source content file is content; runtime/memory/java kinds stay
 * runtime; everything else is `other` rather than a confident wrong guess.
 */
function recommendationKind(
  kind: string,
  sourceFilename: string | null,
): NonNullable<VisualHealthFinding['structuredKind']> {
  if (/java|memory|runtime|jvm|heap/i.test(kind)) return 'runtime';
  if (/snapshot|recover|backup/i.test(kind)) return 'recovery';
  if (sourceFilename || /mod|content|depend|recommend|pack/i.test(kind)) return 'content';
  return 'other';
}

// --- Health map ---
export function healthToVisual(report: HealthReport): VisualHealthFinding[] {
  const findings: VisualHealthFinding[] = [];
  for (const blocker of report.blockers) {
    findings.push({
      id: `live:health:blocker:${blocker.kind}:${blocker.filename ?? blocker.mod_id ?? findings.length}`,
      severity: 'blocker',
      title: findingTitle(blocker, 'Blocker'),
      summary: blocker.message,
      affectedIds: blocker.filename ? [nodeIdFor('mod', blocker.filename)] : [],
      ...(blocker.suggested_action ? { suggestedAction: blocker.suggested_action } : {}),
      structuredKind: blocker.loader_compatibility ? 'loader-compatibility' : 'content',
      ...(blocker.loader_compatibility ? { reviewIntent: { kind: 'review-loader' as const } } : {}),
    });
  }
  for (const warning of report.warnings) {
    findings.push({
      id: `live:health:warning:${warning.kind}:${warning.filename ?? warning.mod_id ?? findings.length}`,
      severity: 'warning',
      title: findingTitle(warning, 'Warning'),
      summary: warning.message,
      affectedIds: warning.filename ? [nodeIdFor('mod', warning.filename)] : [],
      ...(warning.suggested_action ? { suggestedAction: warning.suggested_action } : {}),
      structuredKind: warning.loader_compatibility ? 'loader-compatibility' : 'content',
    });
  }
  for (const recommendation of report.recommendations) {
    findings.push({
      id: `live:health:recommendation:${recommendation.kind}:${findings.length}`,
      severity: 'recommendation',
      title: findingTitle(
        { ...recommendation, filename: recommendation.source_filename },
        'Recommendation',
      ),
      summary: recommendation.message,
      // Recommendations DO name their source content (`source_filename`); the
      // adapter previously discarded it and hardcoded `runtime`, so content
      // recommendations rendered as "Runtime · affects 0" (T6-12).
      affectedIds: recommendation.source_filename
        ? [nodeIdFor('mod', recommendation.source_filename)]
        : [],
      ...(recommendation.suggested_action ? { suggestedAction: recommendation.suggested_action } : {}),
      structuredKind: recommendationKind(recommendation.kind, recommendation.source_filename),
    });
  }
  return findings;
}

// --- Snapshot timeline ---
export function snapshotsToVisual(rows: Snapshot[], available = true): VisualSnapshot[] {
  return rows.map((row) => {
    const role: VisualSnapshot['role'] = row.is_pre_restore
      ? 'undo-restore'
      : row.is_current_lkg
        ? 'current-known-good'
        : row.is_lkg
          ? 'known-good'
          : 'manual';
    const sizeMb = row.size_estimate / (1024 * 1024);
    return {
      id: row.id,
      label: row.label ?? (role === 'known-good' ? 'Last known good' : role === 'current-known-good' ? 'Current known good' : role === 'undo-restore' ? 'Undo restore point' : 'Manual snapshot'),
      createdAt: row.created_at.slice(0, 10),
      sortKey: row.created_at,
      role,
      sizeLabel: sizeMb >= 1024 ? `${(sizeMb / 1024).toFixed(1)} GB` : `${Math.max(1, Math.round(sizeMb))} MB`,
      protects: ['mods', 'config', 'other-instance-files'],
      worldProtection: 'unknown', // snapshot list does not state world scope
      availability: available ? 'available' : 'unavailable',
    };
  });
}

// --- Crash evidence ---
export function crashToVisual(investigation: CrashInvestigation): VisualCrashEvidence {
  const hypotheses = investigation.suspects.map((suspect, index) => ({
    id: `live:crash:hyp:${suspect.mod_id ?? suspect.filename}:${index}`,
    title: suspect.filename || suspect.mod_id || 'Unknown suspect',
    strength: (suspect.total_score >= 3 ? 'high' : suspect.total_score >= 1 ? 'medium' : 'low') as 'high' | 'medium' | 'low',
    supportingClues: [`Evidence rank #${index + 1}`],
    contradictoryClues: [],
    state: 'candidate' as const,
  }));
  return {
    incidentLabel:
      investigation.failure_category === 'Oom'
        ? 'Crash evidence — out of memory'
        : investigation.failure_category === 'JvmFatal'
          ? 'Crash evidence — JVM fatal error'
          : investigation.failure_category === 'NoEvidence'
            ? 'Crash evidence — no clear report'
            : 'Crash evidence',
    evidenceSources: investigation.evidence.sources.map((source) => ({
      kind: source.meta.kind.startsWith('CrashReport')
        ? ('crash-report' as const)
        : source.meta.kind.startsWith('Jvm') || source.meta.kind.startsWith('Debug')
          ? ('log' as const)
          : source.meta.kind === 'LatestLog'
            ? ('log' as const)
            : ('process-outcome' as const),
      state: (source.meta.stale ? 'unknown' : 'known') as 'known' | 'unknown',
      summary: `${source.meta.basename}${source.meta.truncated ? ' (truncated)' : ''}`,
    })),
    hypotheses,
    experiment: {
      phase: 'read-only',
      // Recovery is NOT ready at read time — Crash Doctor creates it before
      // the first experiment (SOL-2 BLOCKER D). Never claim it exists.
      recoveryReady: false,
      summary: 'Crash Doctor will create a recovery point before any experiment.',
    },
    privacyNote: 'Evidence stays on this device. Full logs and paths are not shown here.',
  };
}

// --- Runtime / memory state ---
export function runtimeToVisual(
  row: InstanceRow,
  memory: MemoryRecommendation | null,
  javas: JavaRuntimeSummary[],
  available = true,
): VisualRuntimeState {
  const managedJava = javas.find((java) => java.path === row.java_path);
  return {
    runtime: {
      currentLabel: managedJava ? `Java ${managedJava.version}` : row.java_path ? 'Custom Java' : 'System Java',
      compatibility: 'unknown',
      managedByAgora: managedJava?.source === 'managed',
    },
    memory: {
      mode: { current: row.jvm_memory_mode === 'manual' ? 'manual' : 'automatic' },
      currentMiB: row.jvm_memory_mb,
      ...(memory ? { recommendedMiB: memory.recommended_mb } : {}),
      ...(memory ? { safeHeadroomLabel: memory.tier_label } : {}),
      explanation: memory?.explanation ?? 'Agora manages memory automatically.',
    },
    garbageCollector: {
      current: row.jvm_gc && row.jvm_gc !== 'auto' ? { mode: 'manual', label: row.jvm_gc } : { mode: 'automatic' },
    },
    availability: available ? 'available' : 'unavailable',
  };
}

// --- Per-item catalogue detail (lazy, on selection) ---

/**
 * The extra information the detail panel shows for one selected item.
 *
 * Deliberately fetched ONE AT A TIME, on selection. The obvious alternative —
 * enriching every node when the scene loads — is a per-mod network/DB round trip
 * for a 130-mod pack to populate a panel showing exactly one of them.
 */
export interface ContentDetail {
  description: string | null;
  categories: string[];
  pageUrl: string | null;
  /** Which catalogue answered: shown so the text is never misattributed. */
  source: 'agora' | 'modrinth' | null;
}

export const EMPTY_CONTENT_DETAIL: ContentDetail = {
  description: null,
  categories: [],
  pageUrl: null,
  source: null,
};

/**
 * Look up catalogue detail for an installed item.
 *
 * Agora's curated entry wins when there is one — it is the reviewed description,
 * and this is Agora's own launcher. Modrinth is the fallback, and the only
 * source of category slugs today.
 *
 * Every failure is swallowed into an empty result: a panel that cannot show a
 * description must still show the name, the dependencies and the actions.
 */
export async function readContentDetail(
  registryId: string | null,
  modrinthId: string | null,
): Promise<ContentDetail> {
  let out: ContentDetail = { ...EMPTY_CONTENT_DETAIL };
  if (registryId) {
    try {
      const item = await getRegistryItem(registryId);
      if (item) {
        out = {
          description: item.description ?? null,
          categories: [],
          pageUrl: item.page_url ?? null,
          source: 'agora',
        };
      }
    } catch {
      // fall through to Modrinth
    }
  }
  // Categories only exist on the Modrinth side, so ask for them even when Agora
  // already supplied the prose.
  if (modrinthId && (out.categories.length === 0 || !out.description)) {
    try {
      if (await isModrinthEnabled()) {
        const project = await fetchModrinthProject(modrinthId);
        out = {
          description: out.description ?? project.description ?? null,
          categories: project.categories ?? [],
          pageUrl: out.pageUrl ?? project.page_url ?? null,
          source: out.source ?? 'modrinth',
        };
      }
    } catch {
      // keep whatever Agora gave us
    }
  }
  return out;
}
