import type { PackEntryOutcome, PackImportReport } from './tauri';

/** The core's note on an entry that was already in the instance (a retry). */
const ALREADY_INSTALLED = 'Already installed.';

/** The entries a user action could still get installed. */
export function needsActionOutcomes(report: PackImportReport): PackEntryOutcome[] {
  return report.outcomes.filter((outcome) => outcome.status === 'needs_action');
}

/**
 * Entries that could not be downloaded for lack of a connection. They carry no
 * Settings action: they wait for the user to be online and try again.
 */
export function offlineOutcomes(report: PackImportReport): PackEntryOutcome[] {
  return needsActionOutcomes(report).filter((outcome) => outcome.action === null);
}

export function failedOutcomes(report: PackImportReport): PackEntryOutcome[] {
  return report.outcomes.filter((outcome) => outcome.status === 'failed');
}

/** Installed, but not checked against a hash the pack pinned. */
export function uncheckedOutcomes(report: PackImportReport): PackEntryOutcome[] {
  return report.outcomes.filter(
    (outcome) => outcome.status === 'installed'
      && !outcome.verified_against_pack
      && outcome.reason !== ALREADY_INSTALLED,
  );
}

/** A report needs the user's eyes unless every mod installed and was checked. */
export function importHasIssues(report: PackImportReport): boolean {
  return (
    report.installed !== report.total
    || report.warnings.length > 0
    || uncheckedOutcomes(report).length > 0
  );
}

/** "4 of 5 mods installed." Counts come from the outcomes, never from the pack's own claim. */
export function importHeadline(report: PackImportReport): string {
  const noun = report.total === 1 ? 'mod' : 'mods';
  return `${report.installed} of ${report.total} ${noun} installed.`;
}

/**
 * Fold the result of retrying the needs-action entries into the earlier
 * report: each retried entry replaces its earlier outcome, everything else
 * stays as it was.
 */
export function mergeRetry(previous: PackImportReport, retried: PackImportReport): PackImportReport {
  const queue = [...retried.outcomes];
  const outcomes = previous.outcomes.map((outcome) => {
    if (outcome.status !== 'needs_action') return outcome;
    return queue.shift() ?? outcome;
  });
  return {
    ...previous,
    outcomes,
    installed: outcomes.filter((outcome) => outcome.status === 'installed').length,
    total: outcomes.length,
    warnings: [...previous.warnings, ...retried.warnings],
  };
}
