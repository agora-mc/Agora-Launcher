import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';

/** An instance another Agora program changed, as reported by core's change feed. */
export interface ExternalInstanceChange {
  instance_id: string;
  /** e.g. "Agora CLI"; empty when the other program did not say. */
  program: string;
  /** Core operation name, e.g. "disable_mod". */
  operation: string;
}

export const EXTERNAL_CHANGE_EVENT = 'instances-changed-externally';

/**
 * Run `handler` whenever another Agora program — usually the CLI, driven by an
 * AI agent — changes instances this window may be showing. The backend only
 * reports other processes, so a page's own edits never arrive here.
 */
export function useExternalInstanceChanges(handler: (changes: ExternalInstanceChange[]) => void) {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen<ExternalInstanceChange[]>(EXTERNAL_CHANGE_EVENT, (event) => {
      if (Array.isArray(event.payload) && event.payload.length > 0) handlerRef.current(event.payload);
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
}

/** One line for a toast: who changed the instance, and how. */
export function describeExternalChange(change: ExternalInstanceChange): string {
  const who = change.program ? `The ${change.program}` : 'Another Agora program';
  const what = change.operation ? ` (${change.operation.replace(/[_-]+/g, ' ')})` : '';
  return `${who} changed this instance${what}. Showing the latest.`;
}
