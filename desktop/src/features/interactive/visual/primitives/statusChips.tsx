import type { Compatibility, Knowledge, Severity } from '../../domain/models';
import type { RelationshipKind } from '../../domain/models';

/**
 * Status chips for the interactive status vocabulary.
 *
 * Every status is expressed as persistent text, not color alone.
 */

export function StatusChip({ label }: { label: string }) {
  return (
    <span className="inline-flex items-center rounded-full border border-border bg-muted/60 px-2 py-0.5 text-[0.7rem] font-semibold text-foreground">
      {label}
    </span>
  );
}

const SEVERITY_LABEL: Record<Severity, string> = {
  blocker: 'Blocker',
  warning: 'Warning',
  recommendation: 'Recommendation',
};

export function SeverityBadge({ severity }: { severity: Severity }) {
  const base =
    severity === 'blocker'
      ? 'border-destructive/70 bg-destructive/10 text-destructive'
      : severity === 'warning'
        ? 'border-amber-500/70 bg-amber-500/10 text-amber-700 dark:text-amber-300'
        : 'border-border bg-muted/60 text-muted-foreground';
  return (
    <span className={`inline-flex items-center rounded-md border px-2 py-0.5 text-[0.7rem] font-semibold ${base}`}>
      {SEVERITY_LABEL[severity]}
    </span>
  );
}

const RELATIONSHIP_LABEL: Record<RelationshipKind, string> = {
  requires: 'Requires',
  recommends: 'Recommends',
  'conflicts-with': 'Conflicts with',
};

export function RelationshipBadge({ kind }: { kind: RelationshipKind }) {
  return (
    <span className="inline-flex items-center rounded-md border border-border bg-muted/60 px-2 py-0.5 text-[0.7rem] font-semibold text-foreground">
      {RELATIONSHIP_LABEL[kind]}
    </span>
  );
}

const COMPATIBILITY_LABEL: Record<Compatibility, string> = {
  compatible: 'Compatible',
  indeterminate: 'Needs review',
  incompatible: 'Incompatible',
  unknown: 'Unknown',
};

export function CompatibilityChip({ compatibility }: { compatibility: Compatibility }) {
  const base =
    compatibility === 'compatible'
      ? 'border-emerald-600/60 bg-emerald-600/10 text-emerald-700 dark:text-emerald-300'
      : compatibility === 'incompatible'
        ? 'border-destructive/70 bg-destructive/10 text-destructive'
        : compatibility === 'indeterminate'
          ? 'border-amber-500/70 bg-amber-500/10 text-amber-700 dark:text-amber-300'
          : 'border-border bg-muted/60 text-muted-foreground';
  return (
    <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[0.7rem] font-semibold ${base}`}>
      {COMPATIBILITY_LABEL[compatibility]}
    </span>
  );
}

const KNOWLEDGE_LABEL: Record<Knowledge, string> = {
  known: 'Known',
  unknown: 'Unknown',
  unavailable: 'Unavailable',
};

export function KnowledgeChip({ knowledge }: { knowledge: Knowledge }) {
  return <StatusChip label={KNOWLEDGE_LABEL[knowledge]} />;
}
