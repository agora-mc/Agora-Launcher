/**
 * One counting rule for health findings, shared by the instance card, the
 * instance page, the readiness line and the Health Check dialog.
 *
 * - "Problems" are blockers: things that stop the game launching. Each
 *   finding counts once.
 * - "Warnings" are things to review. Recommendations are advice and never
 *   count here.
 */
export interface HealthCounts {
  problems: number;
  warnings: number;
}

export function healthCounts(
  report: { blockers: readonly unknown[]; warnings: readonly unknown[] } | null | undefined,
): HealthCounts {
  return {
    problems: report?.blockers.length ?? 0,
    warnings: report?.warnings.length ?? 0,
  };
}

export function problemsText(count: number): string {
  return `${count} mod ${count === 1 ? 'problem' : 'problems'} to fix`;
}

export function warningsText(count: number): string {
  return `${count} ${count === 1 ? 'warning' : 'warnings'} to review`;
}

/** "6 mod problems to fix · 1 warning to review" (empty when nothing is wrong). */
export function healthHeadline(counts: HealthCounts): string {
  const parts: string[] = [];
  if (counts.problems > 0) parts.push(problemsText(counts.problems));
  if (counts.warnings > 0) parts.push(warningsText(counts.warnings));
  return parts.join(' · ');
}

/** What the card says about launching, consistent with the Health Check dialog. */
export function healthLaunchNote(counts: HealthCounts): string {
  return counts.problems > 0
    ? 'Launch is blocked until these are fixed. Review & repair shows how, or press Play and choose Launch Anyway at your own risk.'
    : 'Review recommended before launch.';
}

export interface RequirementGroupLike {
  targetId: string;
  versionRanges: string[];
  verdict: unknown;
  modIds: string[];
}

const LOADER_LABELS: Record<string, string> = {
  forge: 'Forge',
  neoforge: 'NeoForge',
  fabric: 'Fabric',
  fabricloader: 'Fabric',
  quilt: 'Quilt',
  quilt_loader: 'Quilt',
};

/**
 * Plain-language reading of one grouped loader requirement. `loader` is the
 * instance's loader (for example "forge"). A requirement on a different
 * loader means the mod was built for that loader, not this one.
 */
export function describeRequirement(group: RequirementGroupLike, loader: string): string {
  const who = group.modIds.length > 0 ? group.modIds.join(', ') : 'A mod';
  const target = LOADER_LABELS[group.targetId.toLowerCase()];
  const current = LOADER_LABELS[loader.toLowerCase()] ?? loader;
  if (target && target !== current) {
    return `${who} ${group.modIds.length > 1 ? 'are' : 'is'} made for ${target}, not ${current}; ${group.modIds.length > 1 ? 'they' : 'it'} may not work in this instance.`;
  }
  const range = group.versionRanges.length > 0 ? ` ${group.versionRanges.join(' or ')}` : '';
  const name = target ?? group.targetId;
  if (group.verdict === 'satisfied') return `${who}: ${name}${range} is met.`;
  if (group.verdict === 'unsatisfied') {
    return `${who} needs ${name}${range}, which this instance does not provide.`;
  }
  return `${who} needs ${name}${range}; Agora can't tell whether this instance provides it.`;
}

