/**
 * One-shot request for the instance editor to open on a specific tab, e.g. the
 * Console after a failed launch. The next editor mount consumes it.
 */
let requestedTab: string | null = null;

export function requestEditorTab(tab: string): void {
  requestedTab = tab;
}

export function takeRequestedEditorTab(): string | null {
  const tab = requestedTab;
  requestedTab = null;
  return tab;
}
