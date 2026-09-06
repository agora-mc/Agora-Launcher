import { useCallback, useEffect, useRef, useState } from 'react';

export type Tab = 'home' | 'browse' | 'instances' | 'governance' | 'ai' | 'guide' | 'field-guide' | 'living-background' | 'lab' | 'about' | 'settings';

/**
 * A single typed application destination. Replaces the previous pattern of
 * three independent state variables (activeTab, selectedModId, editingInstanceId).
 *
 * Destinations:
 * - `tab` — one of the sidebar tabs (home, browse, instances, governance, ai, guide, settings).
 * - `mod-detail` — browsing a specific curated item.
 * - `instance-detail` — editing a specific instance.
 */
export type Destination =
  | { type: 'tab'; tab: Tab; browseInstanceId?: string; browseContentType?: string }
  | { type: 'mod-detail'; itemId: string; browseInstanceId?: string }
  | { type: 'instance-detail'; instanceId: string };

export interface UseDestinationReturn {
  destination: Destination;
  canGoBack: boolean;
  navigate: (dest: Destination) => void;
  goBack: () => void;
  navigateToTab: (tab: Tab) => void;
  navigateToBrowse: (instanceId?: string, contentType?: string) => void;
  navigateToModDetail: (itemId: string, browseInstanceId?: string) => void;
  navigateToInstanceDetail: (instanceId: string) => void;
}

function isValidDestination(d: unknown): d is Destination {
  if (!d || typeof d !== 'object') return false;
  const dest = d as Record<string, unknown>;
  if (dest.type === 'tab') {
    return typeof dest.tab === 'string'
      && (dest.browseInstanceId === undefined || typeof dest.browseInstanceId === 'string')
      && (dest.browseContentType === undefined || typeof dest.browseContentType === 'string');
  }
  if (dest.type === 'mod-detail') {
    return typeof dest.itemId === 'string'
      && (dest.browseInstanceId === undefined || typeof dest.browseInstanceId === 'string');
  }
  if (dest.type === 'instance-detail') return typeof dest.instanceId === 'string';
  return false;
}

const HOME: Destination = { type: 'tab', tab: 'home' };

export function useDestination(): UseDestinationReturn {
  // Depth into the browser's history stack, not a mirror of it.
  //
  // This used to be an array that `push` appended to. Because `goBack()`
  // delegates to `window.history.back()`, the resulting `popstate` came back
  // through the same handler and appended *again* — so navigating backwards
  // grew the stack, and `canGoBack` could never return to false. Browser
  // history is already the source of truth; carrying a position in its state
  // is enough, and cannot drift from it.
  const depthRef = useRef(0);
  const [destination, setDestination] = useState<Destination>(HOME);
  const [canGoBack, setCanGoBack] = useState(false);

  const push = useCallback((dest: Destination) => {
    const depth = depthRef.current + 1;
    depthRef.current = depth;
    setCanGoBack(depth > 0);
    setDestination(dest);
    window.history.pushState({ __agora: dest, __agoraDepth: depth }, '');
  }, []);

  // Handle browser back/forward via popstate.
  useEffect(() => {
    // Initialize from existing history state on page reload to preserve
    // the destination across refreshes. Only write Home when no valid
    // Agora destination exists in the history.
    const existingState = window.history.state as Record<string, unknown> | null;
    const restoredDest = existingState?.__agora as Destination | undefined;
    if (restoredDest && isValidDestination(restoredDest)) {
      const restoredDepth = typeof existingState?.__agoraDepth === 'number'
        ? existingState.__agoraDepth
        : 0;
      depthRef.current = restoredDepth;
      setDestination(restoredDest);
      // Entries behind us survive a reload, so back really does still work.
      setCanGoBack(restoredDepth > 0);
    } else {
      depthRef.current = 0;
      window.history.replaceState({ __agora: HOME, __agoraDepth: 0 }, '');
    }

    const handlePopState = (e: PopStateEvent) => {
      const state = e.state as Record<string, unknown> | null;
      const restored = state?.__agora as Destination | undefined;
      if (restored && isValidDestination(restored)) {
        const depth = typeof state?.__agoraDepth === 'number' ? state.__agoraDepth : 0;
        depthRef.current = depth;
        setDestination(restored);
        setCanGoBack(depth > 0);
      } else {
        // Stepped off the entries this app owns, so there is nothing of ours
        // left behind us.
        depthRef.current = 0;
        setDestination(HOME);
        setCanGoBack(false);
      }
    };
    window.addEventListener('popstate', handlePopState);
    return () => window.removeEventListener('popstate', handlePopState);
  }, []);

  const navigate = useCallback((dest: Destination) => push(dest), [push]);
  // Refuses to step off the entries this app owns. Cancel on a controller is
  // bound to this, so an unguarded `history.back()` at the root would walk the
  // webview out of the app entirely.
  const goBack = useCallback(() => {
    if (depthRef.current <= 0) return;
    window.history.back();
  }, []);
  const navigateToTab = useCallback((tab: Tab) => push({ type: 'tab', tab }), [push]);
  const navigateToBrowse = useCallback(
    (instanceId?: string, contentType?: string) => push({
      type: 'tab',
      tab: 'browse',
      ...(instanceId ? { browseInstanceId: instanceId } : {}),
      ...(contentType ? { browseContentType: contentType } : {}),
    }),
    [push],
  );
  const navigateToModDetail = useCallback(
    (itemId: string, browseInstanceId?: string) => push({
      type: 'mod-detail',
      itemId,
      ...(browseInstanceId ? { browseInstanceId } : {}),
    }),
    [push],
  );
  const navigateToInstanceDetail = useCallback(
    (instanceId: string) => push({ type: 'instance-detail', instanceId }),
    [push],
  );

  return {
    destination,
    canGoBack,
    navigate,
    goBack,
    navigateToTab,
    navigateToBrowse,
    navigateToModDetail,
    navigateToInstanceDetail,
  };
}
