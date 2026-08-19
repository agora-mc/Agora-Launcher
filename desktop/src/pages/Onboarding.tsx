import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { open as openUrl } from '@tauri-apps/plugin-shell';
import {
  cancelJavaRuntime,
  ensureJavaRuntime,
  formatError,
  getSetting,
  githubLogin,
  githubLoginPoll,
  msaGetStatus,
  msaLogin,
  msaLogout,
  setSetting,
  type DeviceFlowResponse,
  type JavaRuntimeProgressEvent,
  type MsaAccountStatus,
} from '../lib/tauri';
import { useRegistryState } from '../lib/useRegistryState';
import { RegistryStatusView } from '../components/registry-status-view';
import { DeviceFlowPanel } from '../components/DeviceFlowPanel';
import { LauncherImportWizard } from '../components/LauncherImportWizard';
import { queueTourStart } from '../features/tour/tourHandoff';
import {
  APPEARANCE_PRESETS,
  useUiPreferences,
  type UiPreferences,
} from '../components/theme/theme-provider';
import {
  loadPreference,
  resumeHighInteractionView,
  savePreference,
  suspendHighInteraction,
  type InteractionPreference,
} from '../features/interactive/live/presentationPreference';
import { pinnedMotion } from '../components/presentation-capabilities';

type Step = 'welcome' | 'appearance' | 'services' | 'java' | 'launch' | 'github' | 'registry' | 'import';

const STEP_ORDER: Step[] = ['welcome', 'appearance', 'services', 'java', 'launch', 'github', 'registry', 'import'];

function isStep(value: unknown): value is Step {
  return typeof value === 'string' && (STEP_ORDER as string[]).includes(value);
}

interface OnboardingProps {
  onComplete: () => void;
}

export function Onboarding({ onComplete }: OnboardingProps) {
  const [step, setStep] = useState<Step>('welcome');
  const [services, setServices] = useState({
    modrinth: false,
    technic: false,
    allowUnverifiedPacks: false,
    aiMcp: false,
    aiChat: false,
  });
  const [servicesLoading, setServicesLoading] = useState(true);
  const [directLaunch, setDirectLaunch] = useState(false);
  const [settingsLoaded, setSettingsLoaded] = useState(false);
  // Persisted across Back/Forward so a registry auto-download triggered on
  // the first entry is not re-triggered when the user revisits the step.
  const registryAutoDownloaded = useRef(false);

  useEffect(() => {
    let cancelled = false;
    Promise.allSettled([
      getSetting('modrinth_enabled'),
      getSetting('technic_enabled'),
      getSetting('allow_unverified_packs'),
      getSetting('ai_mcp_enabled'),
      getSetting('ai_chat_enabled'),
      getSetting('launch_mode'),
      getSetting('onboarding_step'),
    ]).then(([modrinth, technic, allowUnverifiedPacks, aiMcp, aiChat, launchMode, savedStep]) => {
      if (cancelled) return;
      setServices({
        modrinth: modrinth.status === 'fulfilled' ? parseBooleanSetting(modrinth.value) : false,
        technic: technic.status === 'fulfilled' ? parseBooleanSetting(technic.value) : false,
        allowUnverifiedPacks: allowUnverifiedPacks.status === 'fulfilled'
          ? parseBooleanSetting(allowUnverifiedPacks.value)
          : false,
        aiMcp: aiMcp.status === 'fulfilled' ? parseBooleanSetting(aiMcp.value) : false,
        aiChat: aiChat.status === 'fulfilled' ? parseBooleanSetting(aiChat.value) : false,
      });
      setDirectLaunch(
        launchMode.status === 'fulfilled' && launchMode.value === 'direct',
      );
      // Resume an interrupted wizard at the step it was on, so closing or
      // restarting during onboarding does not silently discard progress.
      if (savedStep.status === 'fulfilled' && isStep(savedStep.value)) {
        setStep(savedStep.value);
      }
      setServicesLoading(false);
      setSettingsLoaded(true);
    });
    return () => { cancelled = true; };
  }, []);

  // Persist the current step so an interrupted onboarding can be resumed.
  // Gated on the initial load: persisting before the saved step is read would
  // race the resume and could clobber it back to 'welcome'.
  useEffect(() => {
    if (!settingsLoaded) return;
    void setSetting('onboarding_step', step).catch(() => {});
  }, [step, settingsLoaded]);

  const finish = async () => {
    try {
      await setSetting('onboarding_complete', true);
    } catch {
      // best-effort persistence; still let the user proceed
    } finally {
      onComplete();
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
        <div className="w-full max-w-xl max-h-[calc(100vh-2rem)] overflow-y-auto rounded-2xl border border-border bg-card shadow-xl">
        <div className="p-6 sm:p-8">
          {step === 'welcome' && <WelcomeStep onContinue={() => setStep('appearance')} />}
          {step === 'appearance' && (
            <AppearanceStep onContinue={() => setStep('services')} onBack={() => setStep('welcome')} />
          )}
          {step === 'services' && (
            <ServicesStep
              values={services}
              loading={servicesLoading}
              onChange={setServices}
              onContinue={() => setStep('java')}
              onBack={() => setStep('appearance')}
            />
          )}
          {step === 'java' && (
            <JavaStep onContinue={() => setStep('launch')} onBack={() => setStep('services')} />
          )}
          {step === 'launch' && (
            <LaunchStep
              directLaunch={directLaunch}
              onChange={setDirectLaunch}
              onContinue={() => setStep('github')}
              onBack={() => setStep('java')}
            />
          )}
          {step === 'github' && (
            <GithubStep onContinue={() => setStep('registry')} onBack={() => setStep('launch')} />
          )}
          {step === 'registry' && (
            <RegistryStep onFinish={() => setStep('import')} onBack={() => setStep('github')} hasAutoDownloaded={registryAutoDownloaded} />
          )}
          {step === 'import' && (
            <ImportStep onFinish={finish} onBack={() => setStep('registry')} />
          )}
        </div>
      </div>
    </div>
  );
}

function parseBooleanSetting(value: unknown): boolean {
  return value === true || value === 1 || value === 'true' || value === '1';
}

function Stepper({ current }: { current: Step }) {
  const steps: { id: Step; label: string }[] = [
    { id: 'welcome', label: 'Welcome' },
    { id: 'appearance', label: 'Appearance' },
    { id: 'services', label: 'Services' },
    { id: 'java', label: 'Java' },
    { id: 'launch', label: 'Launch' },
    { id: 'github', label: 'GitHub' },
    { id: 'registry', label: 'Registry' },
    { id: 'import', label: 'Import' },
  ];
  const currentIndex = steps.findIndex((s) => s.id === current);
  return (
    <div className="mb-6 flex flex-wrap items-center gap-x-2 gap-y-1.5">
      {steps.map((s, i) => (
        <div key={s.id} className="flex items-center gap-2">
          <span
            className={`h-2 w-2 rounded-full ${
              i <= currentIndex ? 'bg-primary' : 'bg-muted'
            }`}
          />
          <span
            className={`text-xs ${
              i === currentIndex ? 'font-semibold' : 'text-muted-foreground'
            }`}
          >
            {s.label}
          </span>
          {i < steps.length - 1 && (
            <span className="mx-0.5 h-px w-4 bg-gray-300 dark:bg-gray-600" />
          )}
        </div>
      ))}
    </div>
  );
}

function ImportStep({ onFinish, onBack }: { onFinish: () => void; onBack: () => void }) {
  const [showImport, setShowImport] = useState(false);
  return (
    <div>
      <Stepper current="import" />
      <h2 className="text-2xl font-bold mb-2">Bring Your Instances</h2>
      <p className="text-muted-foreground mb-6">
        Agora can detect Prism Launcher, CurseForge, and Modrinth App instances, then copy the
        instances you choose without changing the originals.
      </p>
      <div className="rounded-xl border border-border bg-card p-4">
        <p className="text-sm font-medium">Import from another launcher</p>
        <p className="mt-1 text-xs text-muted-foreground">
          Review versions, loaders, disk usage, and launch settings before anything is copied.
          Accounts and launcher credentials are never imported.
        </p>
        <button
          onClick={() => setShowImport(true)}
          className="mt-4 rounded-lg bg-primary px-5 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
        >
          Find My Instances
        </button>
      </div>
      <div className="mt-4 rounded-xl border border-primary/30 bg-card p-4">
        <p className="text-sm font-medium">New to Agora? Take the guided tour</p>
        <p className="mt-1 text-xs text-muted-foreground">
          A step-by-step walkthrough that highlights one part of the screen at a time while you make
          your first instance and install your first mod. You can also start it later from Settings.
        </p>
        <button
          onClick={() => {
            // Onboarding unmounts on finish, so the tour is requested here and
            // started by the app shell once it mounts.
            queueTourStart();
            onFinish();
          }}
          className="mt-4 rounded-lg bg-primary px-5 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
        >
          Finish and start the tour
        </button>
      </div>
      <div className="mt-8 flex justify-between">
        <button
          onClick={onBack}
          className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground hover:underline"
        >
          Back
        </button>
        <button
          onClick={onFinish}
          className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground hover:underline"
        >
          Skip for now
        </button>
      </div>
      <LauncherImportWizard
        open={showImport}
        onClose={() => setShowImport(false)}
        onComplete={() => setShowImport(false)}
      />
    </div>
  );
}

function WelcomeStep({ onContinue }: { onContinue: () => void }) {
  return (
    <div>
      <Stepper current="welcome" />
      <h2 className="text-2xl font-bold mb-2">Welcome to Agora</h2>
      <p className="text-muted-foreground mb-4">
        A decentralized, ad-free, open-source Minecraft mod launcher and discovery platform.
      </p>
      <p className="text-sm mb-6">
        Agora returns platform control to the community. The GitHub repository itself is the
        database — flat-file manifests are compiled into a signed SQLite registry. Agora can launch
        directly with optional in-app Microsoft authentication, while delegation to the official
        Mojang launcher remains available as the default fallback. GitHub governance sign-in and
        GitHub Copilot sign-in are separate optional accounts.
      </p>
      <div className="flex justify-end">
        <button
          onClick={onContinue}
          className="rounded-lg bg-primary px-5 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
        >
          Get Started
        </button>
      </div>
    </div>
  );
}

function AppearanceStep({
  onContinue,
  onBack,
}: {
  onContinue: () => void;
  onBack: () => void;
}) {
  const { preferences, setPreferences } = useUiPreferences();
  const activePreset = (preset: UiPreferences) =>
    JSON.stringify(preset) === JSON.stringify(preferences);
  // The interaction mode is a presentation preference with its own versioned
  // key (mirrors Settings → Appearance → Interaction mode), so it is
  // read/written directly rather than folded into the theme blob.
  const [interaction, setInteraction] = useState<InteractionPreference>(() => loadPreference());
  const applyInteraction = (value: InteractionPreference) => {
    setInteraction(value);
    savePreference(value);
    // Simple and High Interaction both render the live instance view, so both
    // resume it; only Standard suspends.
    if (value === 'standard') suspendHighInteraction();
    else resumeHighInteractionView();
    // Simple pins motion to `reduced`. The app-level coordinator enforces this
    // continuously, but onboarding runs before the user reaches Settings, so
    // applying it here makes the choice visible on the very next screen.
    const pin = pinnedMotion(value);
    if (pin) setPreferences({ motion: pin });
  };

  return (
    <div>
      <Stepper current="appearance" />
      <h2 className="text-2xl font-bold mb-2">Make it yours</h2>
      <p className="text-muted-foreground mb-5">
        Choose a look you can read and enjoy before you dig in. Everything here can be changed at any
        time in Settings → Appearance.
      </p>

      <p className="text-sm font-medium mb-2">Appearance preset</p>
      <div className="grid grid-cols-2 gap-2">
        {Object.entries(APPEARANCE_PRESETS).map(([id, preset]) => {
          const active = activePreset(preset.preferences);
          const swatches = presetSwatches(preset.preferences);
          return (
            <button
              key={id}
              type="button"
              aria-pressed={active}
              onClick={() => setPreferences(preset.preferences)}
              className={`rounded-lg border p-3 text-left transition-colors ${
                active
                  ? 'border-primary bg-primary/5'
                  : 'border-border bg-card hover:border-primary/40 hover:bg-accent'
              }`}
            >
              <span className="flex gap-1">
                <span className="h-3 w-3 rounded-full ring-1 ring-black/10" style={{ backgroundColor: swatches.bg }} />
                <span className="h-3 w-3 rounded-full ring-1 ring-black/10" style={{ backgroundColor: swatches.surface }} />
                <span className="h-3 w-3 rounded-full ring-1 ring-black/10" style={{ backgroundColor: swatches.accent }} />
              </span>
              <span className="mt-2 block text-sm font-medium">{preset.label}</span>
            </button>
          );
        })}
      </div>

      <label className="mt-6 block space-y-1 text-sm">
        <span className="flex justify-between font-medium">
          <span>Text scale</span>
          <span>{Math.round(preferences.fontScale * 100)}%</span>
        </span>
        <input
          type="range"
          aria-label="Text scale"
          min="0.85"
          max="2"
          step="0.05"
          value={preferences.fontScale}
          onChange={(event) => setPreferences({ fontScale: Number(event.target.value) })}
          className="w-full accent-primary"
        />
        <span className="block text-xs text-muted-foreground">
          The whole app resizes instantly — including this onboarding screen. Pick whatever is
          comfortable to read.
        </span>
      </label>

      <div className="mt-6 rounded-xl border border-border bg-card p-4">
        <label className="block space-y-1 text-sm">
          <span className="font-medium">Interaction mode</span>
          <select
            aria-label="Interaction mode"
            value={interaction}
            onChange={(event) => applyInteraction(event.target.value as InteractionPreference)}
            className="mt-1 block w-full rounded-md border border-input bg-background px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
          >
            <option value="standard">Standard</option>
            <option value="simple">Simple</option>
            <option value="high-interaction">High Interaction</option>
          </select>
          <span className="block text-xs text-muted-foreground">
            Standard is the full launcher. High Interaction turns an instance into a visual workbench
            and Browse into the Bazaar. Simple is the quiet version of that workbench — the same big
            Play button, shelf and pre-flight check with the decoration, scores and surprises removed,
            plain Browse sorted by Best, and reduced motion. Reviews and content changes always open
            the Standard screens, which remain the only place a change is applied. You can change this
            at any time in Settings → Appearance.
          </span>
        </label>
      </div>

      <div className="mt-8 flex justify-between">
        <button
          onClick={onBack}
          className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground hover:underline"
        >
          Back
        </button>
        <button
          onClick={onContinue}
          className="rounded-lg bg-primary px-5 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
        >
          Continue
        </button>
      </div>
    </div>
  );
}

/** Effective representative colors for an appearance-preset preview swatch. */
function presetSwatches(preferences: UiPreferences): { bg: string; surface: string; accent: string } {
  const dark =
    preferences.colorMode === 'dark' ||
    (preferences.colorMode === 'system' &&
      (typeof window === 'undefined' ||
        window.matchMedia('(prefers-color-scheme: dark)').matches));
  return {
    bg: preferences.backgroundMode === 'custom'
      ? preferences.customBackground
      : dark ? '#091321' : '#f8f7f3',
    surface: preferences.surfaceMode === 'custom'
      ? preferences.customSurface
      : dark ? '#121a2b' : '#ffffff',
    accent: preferences.accentMode === 'custom'
      ? preferences.customAccent
      : '#247786',
  };
}

function ServicesStep({
  values,
  loading,
  onChange,
  onContinue,
  onBack,
}: {
  values: {
    modrinth: boolean;
    technic: boolean;
    allowUnverifiedPacks: boolean;
    aiMcp: boolean;
    aiChat: boolean;
  };
  loading: boolean;
  onChange: (value: {
    modrinth: boolean;
    technic: boolean;
    allowUnverifiedPacks: boolean;
    aiMcp: boolean;
    aiChat: boolean;
  }) => void;
  onContinue: () => void;
  onBack: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const handleContinue = async () => {
    setSaving(true);
    setError(null);
    try {
      await setSetting('modrinth_enabled', values.modrinth);
      await setSetting('technic_enabled', values.technic);
      await setSetting('allow_unverified_packs', values.allowUnverifiedPacks);
      await setSetting('ai_mcp_enabled', values.aiMcp);
      await setSetting('ai_chat_enabled', values.aiChat);
      onContinue();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSaving(false);
    }
  };

  // Persist each toggle as it changes so an interrupted onboarding does not
  // lose choices the user already made. Best-effort; the Continue handler is
  // still the authoritative save.
  const handleToggle = (
    key:
      | 'modrinth_enabled'
      | 'technic_enabled'
      | 'allow_unverified_packs'
      | 'ai_mcp_enabled'
      | 'ai_chat_enabled',
    value: boolean,
  ) => {
    void setSetting(key, value).catch(() => {});
  };

  return (
    <div>
      <Stepper current="services" />
      <h2 className="text-2xl font-bold mb-2">Connect External Services</h2>
      <p className="text-muted-foreground mb-6">
        Optional integrations. All are disabled by default and can be changed later in Settings.
      </p>

      <div className="space-y-4">
        <ServiceToggle
          title="Modrinth live browsing"
          description="Enable live Modrinth search and category browsing when permitted by Privacy settings. Curated Modrinth-sourced catalog entries are always visible — this only controls live third-party browsing."
          checked={values.modrinth}
          onChange={(modrinth) => {
            onChange({ ...values, modrinth });
            handleToggle('modrinth_enabled', modrinth);
          }}
        />
        <ServiceToggle
          title="Technic modpacks"
          description="Browse and install modpacks from Technic. Solder-backed and zip packs are downloaded from third-party hosts with the integrity the pack's author provides."
          checked={values.technic}
          onChange={(technic) => {
            onChange({ ...values, technic });
            handleToggle('technic_enabled', technic);
          }}
        />
        <ServiceToggle
          title="Unverified zip packs"
          description="More packs become available, but Agora cannot verify these files: no hash, no curator review, no per-file audit. You are accepting files on the pack author's word."
          checked={values.allowUnverifiedPacks}
          onChange={(allowUnverifiedPacks) => {
            onChange({ ...values, allowUnverifiedPacks });
            handleToggle('allow_unverified_packs', allowUnverifiedPacks);
          }}
        />
        <ServiceToggle
          title="AI / MCP Server"
          description="Enable the local MCP server for external AI tools to interact with Agora."
          checked={values.aiMcp}
          onChange={(aiMcp) => {
            onChange({ ...values, aiMcp });
            handleToggle('ai_mcp_enabled', aiMcp);
          }}
        />
        <ServiceToggle
          title="Integrated AI Assistant (ALPHA)"
          description="Built-in AI chat using free GitHub Copilot. Get instant crash analysis and mod help without any external setup."
          checked={values.aiChat}
          onChange={(aiChat) => {
            onChange({ ...values, aiChat });
            handleToggle('ai_chat_enabled', aiChat);
          }}
        />
      </div>

      <p className="mt-3 text-xs text-muted-foreground">
        <strong>MCP Server</strong> connects your existing AI agent to Agora.{' '}
        <strong>Integrated AI</strong> gives you a built-in chat — simpler, no setup, but less
        powerful. You can use either, both, or neither.
      </p>

      {error && <p className="mt-4 text-xs text-destructive">{error}</p>}

      <div className="mt-8 flex justify-between">
        <button
          onClick={onBack}
          className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground hover:underline"
        >
          Back
        </button>
        <button
          onClick={handleContinue}
          disabled={saving || loading}
          className="rounded-lg bg-primary px-5 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
        >
          {loading ? 'Loading…' : saving ? 'Saving…' : 'Continue'}
        </button>
      </div>
    </div>
  );
}

function ServiceToggle({
  title,
  description,
  checked,
  onChange,
}: {
  title: string;
  description: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div className="rounded-xl border border-border bg-card p-4">
      <div className="flex items-center justify-between gap-4">
        <span className="font-medium text-sm">{title}</span>
        <button
          type="button"
          role="switch"
          aria-checked={checked}
          aria-label={title}
          onClick={() => onChange(!checked)}
          className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors ${
            checked ? 'bg-primary' : 'bg-muted'
          }`}
        >
          <span
            className={`inline-block h-5 w-5 transform rounded-full bg-white shadow transition-transform ${
              checked ? 'translate-x-5' : 'translate-x-0.5'
            }`}
          />
        </button>
      </div>
      <p className="mt-2 text-xs text-muted-foreground">{description}</p>
    </div>
  );
}

function JavaStep({
  onContinue,
  onBack,
}: {
  onContinue: () => void;
  onBack: () => void;
}) {
  const [checked, setChecked] = useState(true);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<string | null>(null);
  const [percent, setPercent] = useState<number | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  const OPERATION_ID = 'onboarding-java-21';

  // Listen for java-runtime-progress events for onboarding
  useEffect(() => {
    const unlisten = listen<JavaRuntimeProgressEvent>(
      'java-runtime-progress',
      (event) => {
        // Only track onboarding progress
        if (event.payload.instance_id !== '') return;
        setProgress(event.payload.message || `Java ${event.payload.major}: ${event.payload.stage}`);
        setPercent(event.payload.percent);
        if (event.payload.stage === 'ready') {
          setDone(true);
          setProgress('Java 21 is ready.');
        }
      },
    );
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const handleCancelJava = async () => {
    setCancelling(true);
    setProgress('Cancelling…');
    try {
      await cancelJavaRuntime(OPERATION_ID);
    } catch {
      // Operation may already be complete
    }
    // Allow continue without Java even if cancel API fails
    setBusy(false);
    setCancelling(false);
    setProgress(null);
    setPercent(null);
    onContinue();
  };

  const handleContinue = async () => {
    if (!checked) {
      onContinue();
      return;
    }
    setBusy(true);
    setError(null);
    setProgress('Preparing Java 21 runtime…');
    setPercent(0);
    try {
      await ensureJavaRuntime(21, OPERATION_ID);
      setProgress('Java 21 is ready.');
      setPercent(100);
      setDone(true);
      setTimeout(() => onContinue(), 800);
    } catch (e) {
      setError(formatError(e));
      setProgress(null);
      setPercent(null);
      // Allow Continue anyway
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <Stepper current="java" />
      <h2 className="text-2xl font-bold mb-2">Prepare Java for Minecraft</h2>
      <p className="text-muted-foreground mb-6">
        Modern Minecraft (1.17+) requires Java 17 or higher. Agora can download Java 21 — the
        latest long-term support version — so your instances work out of the box.
      </p>

      <div className="rounded-xl border border-border bg-card p-4">
        <div className="flex items-center justify-between gap-4">
          <div>
            <p className="font-medium text-sm">Prepare Java 21 for modern Minecraft</p>
            <p className="text-xs text-muted-foreground mt-1">
              Downloads and manages a private Java 21 runtime in Agora's app data directory.
              Older exact versions download automatically when needed for specific instances.
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={checked}
            aria-label="Prepare Java 21 for modern Minecraft"
            onClick={() => {
              if (!busy) setChecked(!checked);
            }}
            className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors ${
              checked ? 'bg-primary' : 'bg-muted'
            }`}
          >
            <span
              className={`inline-block h-5 w-5 transform rounded-full bg-white shadow transition-transform ${
                checked ? 'translate-x-5' : 'translate-x-0.5'
              }`}
            />
          </button>
        </div>
      </div>

      {busy && progress && !done && (
        <div className="rounded-lg bg-muted px-3 py-3 mt-4 space-y-2">
          <div className="flex items-center gap-2">
            <div className="flex-1 h-2 bg-background rounded-full overflow-hidden">
              <div
                className="h-full bg-primary rounded-full transition-all duration-300"
                style={{ width: `${Math.min(percent ?? 0, 100)}%` }}
              />
            </div>
            <button
              onClick={handleCancelJava}
              disabled={cancelling}
              className="rounded-lg border border-border px-2 py-1 text-xs font-medium hover:bg-accent disabled:opacity-50 shrink-0"
            >
              {cancelling ? 'Cancelling…' : 'Cancel'}
            </button>
          </div>
          <p className="text-xs text-muted-foreground">{progress}</p>
        </div>
      )}

      {done && progress && (
        <div className="rounded-lg bg-muted px-3 py-2 mt-4">
          <p className="text-xs text-muted-foreground">{progress}</p>
        </div>
      )}

      {error && (
        <div className="rounded-lg bg-destructive/10 px-3 py-2 mt-4">
          <p className="text-xs text-destructive">{error}</p>
          <p className="text-xs text-muted-foreground mt-1">
            You can continue without Java and download it later from Settings.
          </p>
        </div>
      )}

      <div className="mt-8 flex justify-between">
        <button
          onClick={onBack}
          disabled={busy}
          className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground hover:underline disabled:opacity-50"
        >
          Back
        </button>
        <button
          onClick={handleContinue}
          disabled={busy && !done}
          className="rounded-lg bg-primary px-5 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
        >
          {busy ? 'Downloading…' : done ? 'Continuing…' : 'Continue'}
        </button>
      </div>
    </div>
  );
}

function LaunchStep({
  directLaunch,
  onChange,
  onContinue,
  onBack,
}: {
  directLaunch: boolean;
  onChange: (value: boolean) => void;
  onContinue: () => void;
  onBack: () => void;
}) {
  const [msa, setMsa] = useState<MsaAccountStatus | null>(null);
  const [msaLoading, setMsaLoading] = useState(true);
  const [msaBusy, setMsaBusy] = useState(false);
  const [msaError, setMsaError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    msaGetStatus()
      .then((creds) => {
        if (!cancelled) setMsa(creds);
      })
      .catch(() => {
        // Not signed in; the sign-in card below is the default view.
      })
      .finally(() => {
        if (!cancelled) setMsaLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Persist as it changes so an interrupted onboarding does not lose the
  // choice. Best-effort; the Continue handler is the authoritative save.
  const handleToggle = (value: boolean) => {
    onChange(value);
    void setSetting('launch_mode', value ? 'direct' : 'delegation').catch(() => {});
  };

  const handleMsaSignIn = async () => {
    setMsaError(null);
    setMsaBusy(true);
    try {
      setMsa(await msaLogin());
    } catch (e) {
      setMsaError(formatError(e));
    } finally {
      setMsaBusy(false);
    }
  };

  const handleMsaSignOut = async () => {
    setMsaError(null);
    try {
      await msaLogout();
      setMsa(null);
    } catch (e) {
      setMsaError(formatError(e));
    }
  };

  const handleContinue = async () => {
    setSaving(true);
    setError(null);
    try {
      await setSetting('launch_mode', directLaunch ? 'direct' : 'delegation');
      onContinue();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div>
      <Stepper current="launch" />
      <h2 className="text-2xl font-bold mb-2">Choose How to Launch</h2>
      <p className="text-muted-foreground mb-6">
        Decide whether Agora or the official Mojang launcher runs Minecraft. This can be changed at
        any time in Settings.
      </p>

      <div className="space-y-4">
        <ServiceToggle
          title="Direct launch (in-app launcher)"
          description="On: Agora launches Minecraft directly — the game runs inside Agora with in-app console output and process control. Requires the Microsoft sign-in below for full online play. Off (default): Agora prepares your instance and hands off to the official Mojang launcher, which handles Microsoft authentication and starts the game itself."
          checked={directLaunch}
          onChange={handleToggle}
        />

        <div className="rounded-xl border border-border bg-card p-4">
          <p className="font-medium text-sm">Microsoft Account</p>
          {msaLoading ? (
            <p className="mt-2 text-xs text-muted-foreground">Checking connection…</p>
          ) : msa ? (
            <div className="mt-2 space-y-1">
              <p className="text-sm text-green-600 dark:text-green-400">
                ● Signed in as <strong>{msa.username}</strong>
              </p>
              <p className="text-xs text-muted-foreground">
                Used to authenticate with Minecraft services during direct launch.
              </p>
              <button
                onClick={handleMsaSignOut}
                className="text-xs text-muted-foreground hover:text-foreground underline"
              >
                Sign out
              </button>
            </div>
          ) : (
            <div className="mt-2 space-y-3">
              <p className="text-xs text-muted-foreground">
                Sign in with your Microsoft account to use direct launch with full online play.
                Optional — delegated launch keeps authentication in the official Mojang launcher.
              </p>
              {msaError && <p className="text-xs text-destructive">{msaError}</p>}
              <button
                onClick={handleMsaSignIn}
                disabled={msaBusy}
                className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                {msaBusy ? 'Signing in…' : 'Sign in with Microsoft'}
              </button>
            </div>
          )}
          {directLaunch && !msaLoading && !msa && (
            <p className="mt-3 text-xs text-muted-foreground">
              Direct launch is enabled without a Microsoft account — you can still launch, but
              online play and skins will not work until you sign in.
            </p>
          )}
        </div>
      </div>

      {error && <p className="mt-4 text-xs text-destructive">{error}</p>}

      <div className="mt-8 flex justify-between">
        <button
          onClick={onBack}
          className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground hover:underline"
        >
          Back
        </button>
        <button
          onClick={handleContinue}
          disabled={saving || msaBusy}
          className="rounded-lg bg-primary px-5 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
        >
          {saving ? 'Saving…' : 'Continue'}
        </button>
      </div>
    </div>
  );
}

function GithubStep({
  onContinue,
  onBack,
}: {
  onContinue: () => void;
  onBack: () => void;
}) {
  const [device, setDevice] = useState<DeviceFlowResponse | null>(null);
  const [polling, setPolling] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Per-sign-in-attempt guard. Each call to `signIn` captures the current
  // value; if a later `signIn` call starts (or the user navigates away and
  // back), the earlier attempt sees the changed value and aborts.
  //
  // NOTE: do NOT use an unmount-ref pattern here. React <StrictMode> (active
  // in dev) re-runs effect cleanups on every render in development, which
  // flips an unmount ref to true mid-await and aborts the OAuth flow even
  // though the component is still mounted. Using a changing counter avoids
  // that false positive — only a *new* sign-in attempt invalidates the
  // in-flight one.
  const sessionIdRef = useRef(0);

  const signIn = async () => {
    setError(null);
    setResult(null);
    setPolling(true);
    const mySession = ++sessionIdRef.current;
    const isStale = () => sessionIdRef.current !== mySession;
    try {
      const flow = await githubLogin();
      if (isStale()) return;
      setDevice(flow);

      // Auto-launch the user's default browser at the verification URL.
      // Wrapped in its own try/catch AND fire-and-forget. If the shell plugin
      // throws synchronously, the inner catch absorbs it so the outer flow
      // continues to githubLoginPoll. URL+code remain displayed for manual
      // fallback.
      try {
        const p = openUrl(flow.verification_uri);
        Promise.resolve(p).catch(() => {
          /* best-effort: URL shown in panel below */
        });
      } catch {
        // URL and code remain visible for manual fallback.
      }

      const token = await githubLoginPoll(flow.device_code, flow.interval);
      if (isStale()) return;
      if (token) {
        setResult('Signed in successfully.');
        setTimeout(() => {
          if (!isStale()) onContinue();
        }, 800);
      } else {
        setResult('Authentication did not complete.');
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : formatError(e);
      if (!isStale()) setError(`Sign-in failed: ${msg}`);
    } finally {
      if (!isStale()) setPolling(false);
    }
  };

  return (
    <div>
      <Stepper current="github" />
      <h2 className="text-2xl font-bold mb-2">Connect GitHub</h2>
      <p className="text-muted-foreground mb-6">
        Sign in with GitHub to participate in community governance (voting, proposals). This is
        optional and can be completed later in Settings.
      </p>

      {device && (
        <DeviceFlowPanel
          device={device}
          polling={polling}
          className="mb-4"
          onCancel={() => {
            sessionIdRef.current += 1;
            setPolling(false);
            setDevice(null);
          }}
        />
      )}

      {result && <p className="mb-4 text-sm text-primary">{result}</p>}
      {error && <p className="mb-4 text-xs text-destructive">{error}</p>}

      <div className="flex justify-between">
        <button
          onClick={() => {
            // Invalidate the polling session before navigating away so
            // an in-flight poll cannot auto-advance onboarding later.
            sessionIdRef.current += 1;
            onBack();
          }}
          className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground hover:underline"
        >
          Back
        </button>
        <div className="flex gap-2">
          {!polling && (
            <button
              onClick={() => {
                // Invalidate the polling session before navigating away.
                sessionIdRef.current += 1;
                onContinue();
              }}
              className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground hover:underline"
            >
              I'll do this later
            </button>
          )}
          <button
            onClick={signIn}
            disabled={polling}
          className="rounded-lg bg-primary px-5 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {polling ? 'Waiting…' : 'Sign in with GitHub'}
          </button>
        </div>
      </div>
    </div>
  );
}

function RegistryStep({
  onFinish,
  onBack,
  hasAutoDownloaded,
}: {
  onFinish: () => void;
  onBack: () => void;
  /** Shared with the parent so the flag survives Back/Forward navigation. */
  hasAutoDownloaded: { current: boolean };
}) {
  const { state, status, loading, error, actions } = useRegistryState();
  const syncRegistry = actions.sync;

  // Auto-download once when we first detect the registry is missing.
  // The effect must react to state changes because on the first render
  // state is 'loading' or 'unknown', and the download should fire when
  // it transitions to 'missing'.
  useEffect(() => {
    if (
      !hasAutoDownloaded.current &&
      state === 'missing' &&
      !loading &&
      !status?.has_cached_db
    ) {
      hasAutoDownloaded.current = true;
      syncRegistry();
    }
  }, [state, loading, status?.has_cached_db, syncRegistry, hasAutoDownloaded]);

  return (
    <div>
      <Stepper current="registry" />
      <h2 className="text-2xl font-bold mb-2">Download Registry</h2>
      <p className="text-muted-foreground mb-6">
        Agora needs the curated registry database to show mods, packs, shaders, and more.
      </p>

      <RegistryStatusView
        variant="fullscreen"
        state={state}
        status={status}
        error={error}
        actions={actions}
        onContinue={onFinish}
        allowMissingContinue
        missingWarning="The registry is required to browse curated content. You can continue but the catalog will be empty until the registry is downloaded."
      />

      <div className="mt-8 flex justify-between">
        <button
          onClick={onBack}
          disabled={loading}
          className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground hover:underline disabled:opacity-50"
        >
          Back
        </button>
      </div>
    </div>
  );
}
