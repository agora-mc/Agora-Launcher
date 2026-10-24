/**
 * A one-shot "open Browse with this search" request, used by "Find <mod>" in
 * the Health Check. Browse may not be mounted yet when the request is made,
 * so the request is parked here until Browse takes it.
 */
const EVENT = 'agora-browse-search';
let pending: string | null = null;

export function requestBrowseSearch(query: string): void {
  pending = query;
  window.dispatchEvent(new CustomEvent(EVENT, { detail: query }));
}

/** Returns the parked request (once) and clears it. */
export function takeBrowseSearchRequest(): string | null {
  const query = pending;
  pending = null;
  return query;
}

export function onBrowseSearchRequest(listener: (query: string) => void): () => void {
  const handler = (event: Event) => {
    const query = takeBrowseSearchRequest();
    listener(query ?? String((event as CustomEvent).detail ?? ''));
  };
  window.addEventListener(EVENT, handler);
  return () => window.removeEventListener(EVENT, handler);
}
