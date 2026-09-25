/**
 * One-shot hand-off of a freshly created instance to Browse.
 *
 * Right after creating an instance the next thing a user almost always does is
 * look for mods for it, so Browse opens with that instance already chosen as
 * its context — the first search then shows what fits it. Consumed on read, so
 * it applies to the next visit to Browse only and never overrides a context the
 * user picks afterwards.
 *
 * Module state rather than a stored setting on purpose: it describes "what you
 * just did in this session", which should not survive a restart.
 */
let offered: string | null = null;

export function offerBrowseContext(instanceId: string): void {
  offered = instanceId;
}

export function takeOfferedBrowseContext(): string | null {
  const instanceId = offered;
  offered = null;
  return instanceId;
}
