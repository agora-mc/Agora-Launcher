import { useCallback, useEffect, useRef, useState, type ComponentProps } from 'react';
import { Sidebar } from './components/Sidebar';
import { CommandPalette } from './components/command-palette';
import { Home } from './pages/Home';
import { Browse } from './pages/Browse';
import { Instances } from './pages/Instances';
import { Governance } from './pages/Governance';
import { Guide } from './pages/Guide';
import { FieldGuide } from './pages/FieldGuide';
import { LivingBackground } from './pages/LivingBackground';
import { About } from './pages/About';
import { Settings } from './pages/Settings';
import AiChatPage from './pages/AiChatPage';
import { Onboarding } from './pages/Onboarding';
import { ModDetail } from './pages/ModDetail';
import { InstanceEditor } from './pages/InstanceEditor';
import { changeLoaderVersion, getInstanceDetail, getSetting, type HealthReport } from './lib/tauri';
import { OfflineBanner } from './components/offline-banner';
import { SandboxBanner } from './components/sandbox-banner';
import { HealthDialog } from './components/HealthDialog';
import { CrashInvestigator } from './components/CrashInvestigator';
import { ToastContainer } from './components/Toast';
import { useDestination, type Destination, type Tab } from './lib/useDestination';
import { useProcessController } from './lib/useProcessController';
import { useInstanceHealthMonitor } from './lib/useInstanceHealthMonitor';
import { useRegistryState } from './lib/useRegistryState';
import { BrandMark } from './components/BrandMark';
import { PackInstallProvider } from './components/PackInstallProgress';
import { GUIDE_TOPICS } from './data/guideContent';
import { LabShell } from './features/interactive/lab/LabShell';
import type { StandardDestination } from './features/interactive/domain/intents';
import { AmbienceProvider, useAmbience } from './features/ambience/AmbienceProvider';
import { AmbienceToasts } from './features/ambience/AmbienceToasts';
import { AmbienceCoordinator } from './components/ambience-coordinator';
import { TourProvider, TourOverlay, consumeQueuedTourStart, useTour } from './features/tour';
import { BookOpen, Bot, Boxes, Compass, HomeIcon, Info, Landmark, Mountain, NotebookPen, SettingsIcon } from 'lucide-react';

const BASE_TABS = [
  { id: 'home' as Tab, label: 'Home', icon: HomeIcon },
  { id: 'browse' as Tab, label: 'Browse', icon: Compass },
  { id: 'instances' as Tab, label: 'My Instances', icon: Boxes },
  { id: 'governance' as Tab, label: 'Community Governance', icon: Landmark },
  { id: 'guide' as Tab, label: 'Help & Guide', icon: BookOpen },
  { id: 'field-guide' as Tab, label: 'Field Guide', icon: NotebookPen },
  { id: 'about' as Tab, label: 'The Agora Difference', icon: Info },
  { id: 'settings' as Tab, label: 'Settings', icon: SettingsIcon },
];

const GUIDE_TOPIC_LABELS: Record<string, string> = Object.fromEntries(
  GUIDE_TOPICS.map((topic) => [topic.id, topic.title]),
);

/**
 * Lab benches teach cause-and-effect, so they drop ambience to `calm` while
 * open (V5-PORT-PLAN §12.1). The Lab itself never imports ambience — this
 * bridge at the app boundary wires the callback.
 */
function LabShellWithAmbience(props: ComponentProps<typeof LabShell>) {
  const { overrideProfile } = useAmbience();
  return <LabShell {...props} onAmbienceChange={(p) => overrideProfile(p)} />;
}

/** Bridge so App can read whether the living background is enabled (for the
 * conditional sidebar tab) without drilling props. Reports up via onChange. */
function AmbienceEnabledBridge({ onChange }: { onChange: (enabled: boolean) => void }) {
  const { enabled } = useAmbience();
  useEffect(() => { onChange(enabled); }, [enabled, onChange]);
  return null;
}

/**
 * Starts the guided walkthrough the onboarding wizard asked for. Onboarding
 * finishes by unmounting itself, so it leaves a note instead of starting one
 * directly; the note is consumed exactly once per app start.
 */
function QueuedTourStarter() {
  const tour = useTour();
  const start = tour?.start;
  const consumedRef = useRef(false);
  useEffect(() => {
    if (consumedRef.current || !start) return;
    consumedRef.current = true;
    if (consumeQueuedTourStart()) start();
  }, [start]);
  return null;
}

const AI_TAB = {
  id: 'ai' as Tab,
  label: 'AI Assistant',
  icon: Bot,
};

interface ShellLayout {
  version: 1;
  sidebar: {
    collapsed: boolean;
    width: number;
    lastExpandedWidth: number;
  };
}

const SHELL_LAYOUT_KEY = 'agora-shell-layout';
const DEFAULT_SHELL_LAYOUT: ShellLayout = {
  version: 1,
  sidebar: { collapsed: false, width: 256, lastExpandedWidth: 256 },
};

function loadShellLayout(): ShellLayout {
  // On narrow viewports the fixed (expanded) sidebar starves the main content
  // and every page — including Agora Lab — overflows horizontally. Default the
  // sidebar to collapsed below the `sm` breakpoint so content gets usable
  // width; the user can still expand it with the sidebar toggle.
  const narrow = typeof window !== 'undefined' && window.innerWidth < 640;
  const collapseForNarrow = (layout: ShellLayout): ShellLayout =>
    narrow
      ? { ...layout, sidebar: { ...layout.sidebar, collapsed: true, width: 64 } }
      : layout;
  try {
    const parsed = JSON.parse(localStorage.getItem(SHELL_LAYOUT_KEY) ?? 'null') as Partial<ShellLayout> | null;
    const sidebar = parsed?.sidebar;
    if (parsed?.version !== 1 || !sidebar || typeof sidebar.width !== 'number') {
      return collapseForNarrow(DEFAULT_SHELL_LAYOUT);
    }
    const width = Math.min(420, Math.max(180, sidebar.width));
    const lastExpandedWidth = typeof sidebar.lastExpandedWidth === 'number'
      ? Math.min(420, Math.max(180, sidebar.lastExpandedWidth))
      : width;
    return collapseForNarrow({
      version: 1,
      sidebar: { collapsed: sidebar.collapsed === true, width, lastExpandedWidth },
    });
  } catch {
    return collapseForNarrow(DEFAULT_SHELL_LAYOUT);
  }
}

function storeShellLayout(layout: ShellLayout) {
  try {
    localStorage.setItem(SHELL_LAYOUT_KEY, JSON.stringify(layout));
  } catch {
    // Layout remains usable for the current session when storage is unavailable.
  }
}

/**
 * Parse a stored boolean setting strictly.
 * - `true` / `false` → as-is
 * - `"true"` / `"1"` → true
 * - `"false"` / `"0"` → false
 * - Everything else (including `null`, missing, corrupt) → fallback
 */
function parseStoredBoolean(value: unknown, fallback: boolean): boolean {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    if (value === 'true' || value === '1') return true;
    if (value === 'false' || value === '0') return false;
  }
  if (typeof value === 'number') return value === 1;
  return fallback;
}

/** Minimal branded loading shell shown while async initialization runs. */
function BrandedSplash() {
  return (
    <div className="flex h-screen w-screen items-center justify-center bg-background">
      <div className="text-center">
        <BrandMark className="justify-center" />
        <p className="mt-4 text-sm text-muted-foreground">Preparing your library…</p>
        <div className="mt-4 flex justify-center">
          <div className="h-5 w-5 animate-spin rounded-full border-2 border-primary border-t-transparent" />
        </div>
      </div>
    </div>
  );
}

/** Derive the effective tab from a destination. */
function destToTab(dest: Destination): Tab {
  if (dest.type === 'tab') return dest.tab;
  if (dest.type === 'instance-detail') return 'instances';
  return 'home'; // mod-detail doesn't change the tab
}

/** Deliberate recoverable not-found view for unrecognized destinations. */
function NotFoundView({ canGoBack, onGoHome, onGoBack }: { canGoBack: boolean; onGoHome: () => void; onGoBack: () => void }) {
  return (
    <div className="space-y-6">
      {canGoBack ? (
        <button
          onClick={onGoBack}
          className="rounded-lg border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent"
        >
          ← Back
        </button>
      ) : (
        <button
          onClick={onGoHome}
          className="rounded-lg border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent"
        >
          ← Back to Home
        </button>
      )}
      <div className="rounded-xl border border-destructive bg-destructive/10 p-6 text-center" data-testid="not-found-view">
        <h2 className="text-xl font-bold text-foreground">Page Not Found</h2>
        <p className="text-sm text-muted-foreground mt-2">
          The requested page could not be found.
        </p>
      </div>
    </div>
  );
}

/** The three known destination types used for validation. */
const KNOWN_DEST_TYPES = new Set(['tab', 'mod-detail', 'instance-detail']);

export default function App() {
  const {
    destination,
    canGoBack,
    navigateToTab,
    navigateToBrowse,
    navigateToModDetail,
    navigateToInstanceDetail,
    goBack,
  } = useDestination();

  const processController = useProcessController();
  const mainRef = useRef<HTMLElement>(null);
  const previousDestinationRef = useRef<Destination>(destination);
  const browseScrollTopRef = useRef(0);
  const instanceEditorScrollTopRef = useRef(0);
  const modDetailOriginRef = useRef<Destination | null>(null);
  const [browseVisited, setBrowseVisited] = useState(false);
  const [instanceEditorVisited, setInstanceEditorVisited] = useState(false);

  const [onboardingComplete, setOnboardingComplete] = useState<boolean | null>(null);
  const [aiChatEnabled, setAiChatEnabled] = useState<boolean>(false);
  const [ambienceEnabled, setAmbienceEnabled] = useState<boolean>(true);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [guideTopic, setGuideTopic] = useState<string | null>(null);
  const [shellLayout, setShellLayout] = useState<ShellLayout>(loadShellLayout);
  const [crashInvestigation, setCrashInvestigation] = useState<{
    instanceId: string;
    crashFilename: string | null;
    manualLogText: string | null;
    directLaunch: boolean;
  } | null>(null);
  const [healthReview, setHealthReview] = useState<{
    instanceId: string;
    instanceName: string;
    report: HealthReport;
  } | null>(null);
  const healthMonitor = useInstanceHealthMonitor(onboardingComplete === true);
  const registry = useRegistryState();

  // Fetch the latest signed registry at launch so the app always starts on a
  // fresh catalog. Skipped when registry sync is disabled in Privacy settings
  // (the backend errors on that case; a launch-time alert would nag every start).
  useEffect(() => {
    if (onboardingComplete !== true) return;
    let cancelled = false;
    (async () => {
      try {
        const value = await getSetting('network_registry_sync_enabled');
        if (cancelled || value === false || value === 'false') return;
      } catch {
        // Fail-open: attempt the sync when the setting cannot be read.
      }
      await registry.actions.sync();
    })();
    return () => {
      cancelled = true;
    };
  }, [onboardingComplete, registry.actions]);

  useEffect(() => {
    if (destination.type === 'tab' && destination.tab === 'browse') {
      setBrowseVisited(true);
    }
    if (destination.type === 'instance-detail') {
      setInstanceEditorVisited(true);
    }

    const previous = previousDestinationRef.current;
    const cameFromBrowse = previous.type === 'tab' && previous.tab === 'browse';
    const cameFromInstanceEditor = previous.type === 'instance-detail';
    const returnedToBrowse =
      destination.type === 'tab'
      && destination.tab === 'browse'
      && previous.type === 'mod-detail'
      && modDetailOriginRef.current?.type === 'tab'
      && modDetailOriginRef.current.tab === 'browse';
    const returnedToInstanceEditor =
      destination.type === 'instance-detail'
      && previous.type === 'mod-detail'
      && modDetailOriginRef.current?.type === 'instance-detail';

    if (destination.type === 'mod-detail' && (cameFromBrowse || cameFromInstanceEditor)) {
      modDetailOriginRef.current = previous;
      mainRef.current?.scrollTo({ top: 0, behavior: 'auto' });
    } else if (returnedToBrowse) {
      requestAnimationFrame(() => {
        mainRef.current?.scrollTo({ top: browseScrollTopRef.current, behavior: 'auto' });
      });
    } else if (returnedToInstanceEditor) {
      requestAnimationFrame(() => {
        mainRef.current?.scrollTo({ top: instanceEditorScrollTopRef.current, behavior: 'auto' });
      });
    }

    previousDestinationRef.current = destination;
  }, [destination]);

  // Legacy bridge: the CommandPalette still uses (tab, instanceId?) signature.
  const handleNavigate = (tab: Tab, instanceId?: string) => {
    if (instanceId) {
      navigateToInstanceDetail(instanceId);
    } else {
      navigateToTab(tab);
    }
  };

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const value = await getSetting('onboarding_complete');
        if (!cancelled) setOnboardingComplete(parseStoredBoolean(value, false));
      } catch {
        // On transient read failure, assume completed (safe for non-Tauri dev).
        if (!cancelled) setOnboardingComplete(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Re-read the ai_chat_enabled toggle whenever the destination changes
  // so the sidebar reflects the current setting without an app restart.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const ai = await getSetting('ai_chat_enabled');
        if (!cancelled) setAiChatEnabled(ai === true || ai === 'true');
      } catch {
        if (!cancelled) setAiChatEnabled(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [destination]);

  // React to the agora-navigate custom event (used by external code).
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail as string;
      if (detail === 'settings') {
        navigateToTab('settings');
      }
    };
    window.addEventListener('agora-navigate', handler);
    return () => window.removeEventListener('agora-navigate', handler);
  }, [navigateToTab]);

  // The walkthrough opens on Home, whatever page the user started it from.
  // Every step after that is driven by the user, never by the tour. Declared
  // with the other hooks, above the onboarding early-returns.
  const handleTourStart = useCallback(() => navigateToTab('home'), [navigateToTab]);

  // Ctrl+K / Cmd+K opens the command palette.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        const target = e.target instanceof HTMLElement ? e.target : null;
        const tag = target?.tagName;
        if (tag === 'INPUT' || tag === 'TEXTAREA' || target?.isContentEditable || target?.getAttribute('role') === 'textbox') {
          return;
        }
        e.preventDefault();
        setCommandPaletteOpen((prev) => !prev);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  if (onboardingComplete === null) {
    return <BrandedSplash />;
  }

  if (!onboardingComplete) {
    return (
      <div className="h-screen w-screen overflow-hidden bg-card">
        <Onboarding onComplete={() => setOnboardingComplete(true)} />
      </div>
    );
  }

  // The AI Assistant tab appears between Governance and Settings when enabled;
  // the Living Background tab only appears while the living background is on.
  // Built by explicit reference (never by index — index-based construction
  // silently drops any tab appended to BASE_TABS, e.g. Settings).
  // Agora Lab is intentionally absent: it is the high-interaction entry point
  // for the guide's "New to modding" tier, reached from Help & Guide rather
  // than owning a permanent sidebar slot. `'lab'` remains a valid destination.
  const [tabHome, tabBrowse, tabInstances, tabGovernance, tabGuide, tabFieldGuide, tabAbout, tabSettings] = BASE_TABS;
  const tabs = [
    tabHome,
    tabBrowse,
    tabInstances,
    tabGovernance,
    ...(aiChatEnabled ? [AI_TAB] : []),
    tabGuide,
    tabFieldGuide,
    ...(ambienceEnabled ? [{ id: 'living-background' as Tab, label: 'Living Background', icon: Mountain }] : []),
    tabAbout,
    tabSettings,
  ];

  // Resolve the current UI state from the destination. A tab that became
  // unavailable (e.g. living background toggled off while open) falls back.
  const effectiveTab: Tab =
    destination.type === 'tab' && destination.tab === 'ai' && !aiChatEnabled
      ? 'home'
      : destination.type === 'tab' && destination.tab === 'living-background' && !ambienceEnabled
        ? 'home'
        : destToTab(destination);

  // Validate destination type — corrupt state or future versions must not
  // silently fall to home. This is a defense-in-depth check; the type system
  // already prevents invalid Destination types at compile time.
  const isKnownDestType = KNOWN_DEST_TYPES.has(destination.type);

  const showModDetail = destination.type === 'mod-detail';
  const previousDestination = previousDestinationRef.current;
  const shouldRenderBrowse =
    effectiveTab === 'browse'
    || (browseVisited
      && showModDetail
      && previousDestination.type === 'tab'
      && previousDestination.tab === 'browse');
  const shouldRenderInstanceEditor =
    destination.type === 'instance-detail'
    || (showModDetail
      && instanceEditorVisited
      && previousDestination.type === 'instance-detail');
  const browseInstanceId =
    destination.type === 'tab' && destination.tab === 'browse'
      ? destination.browseInstanceId
      : undefined;
  const browseContentType =
    destination.type === 'tab' && destination.tab === 'browse'
      ? destination.browseContentType
      : undefined;
  const modDetailBrowseInstanceId =
    destination.type === 'mod-detail'
      ? destination.browseInstanceId
      : undefined;
  const instanceEditorId =
    destination.type === 'instance-detail'
      ? destination.instanceId
      : showModDetail && previousDestination.type === 'instance-detail'
        ? previousDestination.instanceId
        : undefined;

  // Render the HealthDialog at the App level so it survives page navigation.
  const {
    state: processState,
    logs: processLogs,
    startLaunch,
    startLaunchDetailed,
    approveLaunch,
    cancelLaunch,
    kill: killProcess,
    clearError,
    repairAndRetry,
    switchLoaderAndRetry,
    useDelegatedLaunch,
  } = processController;

  const resolveDirectLaunch = async (instanceId: string) => {
    let directLaunch = false;
    try {
      directLaunch = (await getSetting('launch_mode')) === 'direct';
      const detail = await getInstanceDetail(instanceId);
      if (detail?.row.launch_mode_override === 'delegated') directLaunch = false;
      if (detail?.row.launch_mode_override === 'direct') directLaunch = true;
    } catch {
      // Delegated launch is the safe default when the setting is unavailable.
    }
    return directLaunch;
  };

  const handleInstanceEditorLaunch = async (instanceId: string) => {
    return startLaunch(instanceId, await resolveDirectLaunch(instanceId));
  };

  const openHealthReview = (instanceId: string, instanceName: string, report: HealthReport) => {
    setHealthReview({ instanceId, instanceName, report });
  };

  const switchLoaderFromHealthReview = async (targetVersion: string): Promise<HealthReport> => {
    const review = healthReview;
    if (!review) throw new Error('No instance health report is open.');
    const result = await changeLoaderVersion(review.instanceId, targetVersion);
    healthMonitor.updateReport(review.instanceId, result.health);
    setHealthReview((current) => current && current.instanceId === review.instanceId
      ? { ...current, report: result.health }
      : current);
    return result.health;
  };

  const handleInstanceEditorInvestigate = (instanceId: string) => {
    void resolveDirectLaunch(instanceId).then((directLaunch) => {
      setCrashInvestigation({
        instanceId,
        crashFilename: null,
        manualLogText: null,
        directLaunch,
      });
    });
  };

  const handleModDetailBack = () => {
    if (canGoBack) {
      goBack();
    } else {
      navigateToTab('browse');
    }
  };

  const handleBrowseSelectMod = (id: string, instanceId?: string) => {
    browseScrollTopRef.current = mainRef.current?.scrollTop ?? 0;
    navigateToModDetail(id, instanceId);
  };

  const handleInstanceEditorOpenMod = (id: string) => {
    instanceEditorScrollTopRef.current = mainRef.current?.scrollTop ?? 0;
    navigateToModDetail(id);
  };

  // Agora Lab handoffs: end the simulation by navigating to the real feature.
  const handleOpenGuide = (topicId: string) => {
    setGuideTopic(topicId);
    navigateToTab('guide');
  };

  const handleNavigateStandard = (dest: StandardDestination) => {
    if (dest.type === 'tab') {
      navigateToTab(dest.tab);
    } else if (dest.type === 'instance-detail') {
      navigateToInstanceDetail(dest.instanceId);
    } else if (dest.type === 'mod-detail') {
      navigateToModDetail(dest.itemId, dest.browseInstanceId);
    }
  };

  return (
    <PackInstallProvider>
      <TourProvider onStart={handleTourStart}>
      <AmbienceProvider>
      <AmbienceEnabledBridge onChange={setAmbienceEnabled} />
      <QueuedTourStarter />
      <div className="app-shell flex h-screen w-screen overflow-hidden">
        <OfflineBanner />
        <SandboxBanner />
        <Sidebar
          tabs={tabs}
          activeTab={effectiveTab}
          onSelectTab={navigateToTab}
          onOpenCommandPalette={() => setCommandPaletteOpen(true)}
          collapsed={shellLayout.sidebar.collapsed}
          width={shellLayout.sidebar.width}
          onCollapsedChange={(collapsed) => {
            setShellLayout((current) => {
              const next = {
                ...current,
                sidebar: {
                  ...current.sidebar,
                  collapsed,
                  width: collapsed ? current.sidebar.width : current.sidebar.lastExpandedWidth,
                  lastExpandedWidth: collapsed ? current.sidebar.width : current.sidebar.lastExpandedWidth,
                },
              };
              storeShellLayout(next);
              return next;
            });
          }}
          onWidthChange={(width) => {
            setShellLayout((current) => ({
              ...current,
              sidebar: { ...current.sidebar, width, lastExpandedWidth: width },
            }));
          }}
          onWidthCommit={(width) => {
            setShellLayout((current) => {
              const next = {
                ...current,
                sidebar: { ...current.sidebar, width, lastExpandedWidth: width },
              };
              storeShellLayout(next);
              return next;
            });
          }}
          registryStatus={registry.status}
        />

        {processState.phase === 'failed' && processState.healthReport ? (
          <HealthDialog
            instanceId={processState.instanceId!}
            instanceName={processState.instanceId!}
            initialReport={processState.healthReport}
            onConfirm={approveLaunch}
            onCancel={cancelLaunch}
            onSwitchLoader={switchLoaderAndRetry}
          />
        ) : healthReview && (
          <HealthDialog
            instanceId={healthReview.instanceId}
            instanceName={healthReview.instanceName}
            initialReport={healthReview.report}
            onCancel={() => setHealthReview(null)}
            onSwitchLoader={switchLoaderFromHealthReview}
            reviewOnly
            onReportChanged={(report) => {
              healthMonitor.updateReport(healthReview.instanceId, report);
              setHealthReview((current) => current && current.instanceId === healthReview.instanceId
                ? { ...current, report }
                : current);
            }}
          />
        )}

        <main ref={mainRef} className={`flex-1 overflow-y-auto bg-background p-6 text-background-foreground ${effectiveTab === 'living-background' ? 'living-bg-active' : ''}`}>
          <div className="contents">
            {!isKnownDestType ? (
              <NotFoundView
                canGoBack={canGoBack}
                onGoHome={() => navigateToTab('home')}
                onGoBack={goBack}
              />
            ) : showModDetail ? (
              <ModDetail
                itemId={destination.itemId}
                initialInstanceId={modDetailBrowseInstanceId}
                onBack={handleModDetailBack}
                onOpenInstanceEditor={(id) => {
                  navigateToInstanceDetail(id);
                }}
              />
            ) : destination.type === 'instance-detail' ? null : (
              <>
                {effectiveTab === 'home' && (
                  <Home
                    onNavigateTab={navigateToTab}
                    onOpenInstance={navigateToInstanceDetail}
                    onOpenMod={navigateToModDetail}
                    onLaunch={startLaunch}
                    processState={processState}
                    onKillProcess={killProcess}
                  />
                )}
                {effectiveTab === 'instances' && (
                  <Instances
                    onEditInstance={(id) => navigateToInstanceDetail(id)}
                    processState={processState}
                    onStartLaunch={startLaunch}
                    onKillProcess={killProcess}
                    onStartCrashInvestigation={setCrashInvestigation}
                    onRepairAndRetry={repairAndRetry}
                    onUseDelegatedLaunch={useDelegatedLaunch}
                    onClearError={clearError}
                    healthReports={healthMonitor.reports}
                    healthErrors={healthMonitor.errors}
                    onReviewHealth={openHealthReview}
                    onRefreshHealth={healthMonitor.refresh}
                  />
                )}
                {effectiveTab === 'governance' && <Governance />}
                {effectiveTab === 'ai' && aiChatEnabled && <AiChatPage />}
                {effectiveTab === 'guide' && (
                  <Guide
                    key={guideTopic ?? 'default'}
                    onNavigateTab={navigateToTab}
                    initialTopicId={guideTopic ?? undefined}
                  />
                )}
                {effectiveTab === 'field-guide' && <FieldGuide />}
                {effectiveTab === 'living-background' && ambienceEnabled && <LivingBackground />}
                {effectiveTab === 'lab' && (
                  <LabShellWithAmbience
                    onOpenGuide={handleOpenGuide}
                    onNavigateStandard={handleNavigateStandard}
                    onExit={() => navigateToTab('guide')}
                    guideTopicLabels={GUIDE_TOPIC_LABELS}
                  />
                )}
                {effectiveTab === 'about' && <About />}
                {effectiveTab === 'settings' && (
                  <Settings
                    onResetLayout={() => {
                      const reset = {
                        ...DEFAULT_SHELL_LAYOUT,
                        sidebar: { ...DEFAULT_SHELL_LAYOUT.sidebar },
                      };
                      setShellLayout(reset);
                      storeShellLayout(reset);
                    }}
                  />
                )}
              </>
            )}
          </div>
          <div className="contents">
            {shouldRenderBrowse && (
              <div className={showModDetail ? 'hidden' : undefined}>
                <Browse
                  onSelectMod={handleBrowseSelectMod}
                  onOpenInstance={navigateToInstanceDetail}
                  initialInstanceId={browseInstanceId}
                  initialContentType={browseContentType}
                />
              </div>
            )}
            {shouldRenderInstanceEditor && instanceEditorId && (
              <div className={showModDetail ? 'hidden' : undefined}>
                <InstanceEditor
                  instanceId={instanceEditorId}
                  onBack={() => navigateToTab('instances')}
                  onOpenInstanceEditor={(id) => navigateToInstanceDetail(id)}
                  onOpenModDetail={handleInstanceEditorOpenMod}
                  onOpenBrowseForInstance={navigateToBrowse}
                  onLaunch={handleInstanceEditorLaunch}
                  processState={processState}
                  onKillProcess={killProcess}
                  onInvestigate={handleInstanceEditorInvestigate}
                  processLogs={processLogs}
                  healthReport={healthMonitor.reports[instanceEditorId] ?? null}
                  onReviewHealth={openHealthReview}
                />
              </div>
            )}
          </div>
        </main>

        <CommandPalette
          open={commandPaletteOpen}
          onOpenChange={setCommandPaletteOpen}
          onNavigate={handleNavigate}
        />
        <ToastContainer />
        {crashInvestigation && (
          <CrashInvestigator
            instanceId={crashInvestigation.instanceId}
            crashFilename={crashInvestigation.crashFilename}
            manualLogText={crashInvestigation.manualLogText}
            onClose={() => setCrashInvestigation(null)}
            onLaunch={(onAwaitingHealth) => startLaunchDetailed(
              crashInvestigation.instanceId,
              crashInvestigation.directLaunch,
              onAwaitingHealth,
            )}
            processState={processState}
          />
        )}
      </div>
      <AmbienceToasts />
      <AmbienceCoordinator activeTab={effectiveTab} />
      <TourOverlay />
      </AmbienceProvider>
      </TourProvider>
    </PackInstallProvider>
  );
}

