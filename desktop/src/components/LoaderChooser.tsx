// ---------------------------------------------------------------------------
// Reusable loader version chooser.
//
// Renders a "launcher choosing" panel used wherever a user must decide between
// keeping the current loader version or switching to a signed-catalog version
// that satisfies the enabled mods' hard requirements:
//   - the install review flow, when a `loader-change` pending choice exists
//     (with an optional "skip these mods instead" escape hatch), and
//   - the instance editor, to change the loader version on demand.
//
// All data comes from structured loader-compatibility payloads (the install
// `PendingChoice::LoaderChange` or the loader-service `LoaderChangePlan`);
// human-readable messages are never parsed here.
// ---------------------------------------------------------------------------

import { useState } from 'react';
import type { LoaderConflict, RequirementVerdict } from '@/lib/tauri';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
} from '@/components/ui/select';
import { Button } from '@/components/ui/button';

/** Normalized requirement evidence for one loader requirement. */
export interface LoaderChooserRequirement {
  targetId: string;
  versionRanges: string[];
  candidateVersion: string | null;
  verdict: RequirementVerdict;
  modIds: string[];
}

export interface LoaderChooserProps {
  loader: string;
  currentVersion: string | null;
  recommendedVersion: string | null;
  compatibleVersions: string[];
  indeterminateVersions?: string[];
  requirements: LoaderChooserRequirement[];
  conflicts: LoaderConflict[];
  /** Disable the switch/skip actions while a switch is in flight. */
  busy?: boolean;
  error?: string | null;
  /** Label for the recommended-version switch button. */
  switchLabel?: string;
  /** Commit a switch to the given signed-catalog version. */
  onChoose: (version: string) => void | Promise<void>;
  /** Optional "skip installing the problematic mods" escape hatch. */
  onSkip?: () => void;
  skipLabel?: string;
}

function verdictLabel(verdict: RequirementVerdict): string {
  if (verdict === 'satisfied') return 'Satisfied';
  if (verdict === 'unsatisfied') return 'Unsatisfied';
  return 'Unsupported';
}

function RequirementRow({ requirement }: { requirement: LoaderChooserRequirement }) {
  const predicate = requirement.versionRanges.length > 0
    ? requirement.versionRanges.join(' or ')
    : 'any version';
  const affected = requirement.modIds.length === 1
    ? requirement.modIds[0]
    : requirement.modIds.length > 0
      ? `${requirement.modIds.length} mods`
      : null;
  return (
    <div className="rounded border border-border bg-background/60 p-2 text-xs">
      <p className="font-medium">
        {requirement.targetId} {predicate}
        <span className="ml-1 text-muted-foreground">
          ({verdictLabel(requirement.verdict)})
        </span>
      </p>
      {requirement.candidateVersion && (
        <p className="text-muted-foreground">
          Current loader provides {requirement.candidateVersion}
        </p>
      )}
      {affected && (
        <p className="mt-0.5 break-words text-muted-foreground">
          Required by {affected}
        </p>
      )}
    </div>
  );
}

/**
 * Loader version chooser panel. Renders the current loader, the recommended
 * version (when one exists), a select of every signed-catalog compatible
 * version, an optional skip action, and collapsible compatibility evidence.
 */
export function LoaderChooser({
  loader,
  currentVersion,
  recommendedVersion,
  compatibleVersions,
  indeterminateVersions = [],
  requirements,
  conflicts,
  busy = false,
  error,
  switchLabel,
  onChoose,
  onSkip,
  skipLabel = 'Skip this mod instead',
}: LoaderChooserProps) {
  const [localError, setLocalError] = useState<string | null>(null);

  const switchable = compatibleVersions.filter((version) => version !== currentVersion);
  const hasCandidates = switchable.length > 0;
  const effectiveError = error ?? localError;
  const disabled = busy;

  const runChoose = async (version: string) => {
    if (disabled) return;
    setLocalError(null);
    try {
      await onChoose(version);
    } catch (cause) {
      setLocalError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const currentLabel = currentVersion ? `${loader} ${currentVersion}` : loader;
  const hasUnresolvedRequirements = requirements.some((r) => r.verdict !== 'satisfied');
  const switchButtonLabel = switchLabel ?? (recommendedVersion
    ? `Switch to ${recommendedVersion} & Continue`
    : 'Switch & Continue');

  return (
    <div className="rounded-lg border border-amber-500 bg-amber-500/10 p-3 text-sm">
      <p className="font-semibold">
        Loader version change required
      </p>
      <p className="mt-1 text-xs text-muted-foreground">
        The mods you are installing require a loader version the instance does not
        currently provide. Switch to a compatible version, or skip installing the
        incompatible mods.
      </p>
      <div className="mt-2 rounded border border-border bg-background/60 p-2 text-xs">
        <p className="font-medium">Loader: {currentLabel}</p>
        {recommendedVersion && recommendedVersion !== currentVersion ? (
          <p className="mt-1 text-muted-foreground">
            Recommended version: {recommendedVersion}
          </p>
        ) : !hasCandidates ? (
          <p className="mt-1 text-destructive">
            No signed loader version satisfies every enabled mod requirement.
            {indeterminateVersions.length > 0
              ? ` ${indeterminateVersions.length} version${indeterminateVersions.length > 1 ? 's are' : ' is'} available for manual confirmation.`
              : ''}
          </p>
        ) : null}
      </div>

      {effectiveError && (
        <p className="mt-2 text-xs text-destructive">{effectiveError}</p>
      )}

      {hasCandidates && (
        <div className="mt-2 flex flex-wrap items-center gap-2">
          {recommendedVersion && recommendedVersion !== currentVersion && (
            <Button
              size="sm"
              disabled={disabled}
              onClick={() => void runChoose(recommendedVersion)}
            >
              {busy ? 'Switching…' : switchButtonLabel}
            </Button>
          )}
          <Select
            onValueChange={(version) => void runChoose(version)}
            disabled={disabled}
          >
            <SelectTrigger
              className="h-8 w-auto min-w-[12rem] text-xs"
              aria-label="Choose compatible version"
            >
              <span className="text-muted-foreground">Choose compatible version</span>
            </SelectTrigger>
            <SelectContent>
              {switchable.map((version) => (
                <SelectItem key={version} value={version}>
                  {version}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}

      {onSkip && (
        <div className="mt-3 border-t border-amber-500/30 pt-2">
          <Button
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => {
              setLocalError(null);
              onSkip();
            }}
          >
            {skipLabel}
          </Button>
          <p className="mt-1 text-xs text-muted-foreground">
            Skip installing the mods whose loader requirements cannot be met with
            the current loader.
          </p>
        </div>
      )}

      {(requirements.length > 0 || conflicts.length > 0 || indeterminateVersions.length > 0) && (
        <details className="mt-2">
          <summary className="cursor-pointer select-none text-xs font-medium">
            View compatibility evidence
            {requirements.length > 0 ? ` (${requirements.length} requirement${requirements.length > 1 ? 's' : ''})` : ''}
            {conflicts.length > 0 ? ` · ${conflicts.length} conflict${conflicts.length > 1 ? 's' : ''}` : ''}
          </summary>
          <div className="mt-2 space-y-2">
            {hasUnresolvedRequirements && (
              <div className="space-y-1">
                <p className="text-xs font-medium text-destructive">
                  Unresolved requirements
                </p>
                {requirements
                  .filter((r) => r.verdict !== 'satisfied')
                  .map((requirement, index) => (
                    <RequirementRow key={`${requirement.targetId}-${index}`} requirement={requirement} />
                  ))}
              </div>
            )}
            {requirements.some((r) => r.verdict === 'satisfied') && (
              <div className="space-y-1">
                <p className="text-xs font-medium text-green-700 dark:text-green-400">
                  Satisfied requirements
                </p>
                {requirements
                  .filter((r) => r.verdict === 'satisfied')
                  .map((requirement, index) => (
                    <RequirementRow key={`${requirement.targetId}-${index}`} requirement={requirement} />
                  ))}
              </div>
            )}
            {indeterminateVersions.length > 0 && (
              <p className="text-xs text-amber-600 dark:text-amber-400">
                Manual candidates: {indeterminateVersions.join(', ')}. These versions
                require explicit confirmation because at least one capability is unverified.
              </p>
            )}
            {conflicts.map((conflict, index) => (
              <div key={`conflict-${index}`} className="rounded border border-destructive bg-background/60 p-2">
                <p className="text-xs font-medium">Conflict</p>
                <p className="text-xs text-muted-foreground">{conflict.message}</p>
                <p className="text-xs text-muted-foreground">
                  {conflict.declaring_mod_id ?? 'unowned'} requires {conflict.target_id}{' '}
                  {conflict.version_ranges.join(' or ')} · {conflict.with_declaring_mod_id ?? 'unowned'}{' '}
                  requires {conflict.with_target_id} {conflict.with_version_ranges.join(' or ')}
                </p>
              </div>
            ))}
          </div>
        </details>
      )}
    </div>
  );
}
