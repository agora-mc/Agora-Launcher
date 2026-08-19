import { useEffect, useState } from 'react';
import {
  APPEARANCE_PRESETS,
  useUiPreferences,
} from '../../components/theme/theme-provider';
import {
  loadPreference,
  resumeHighInteractionView,
  savePreference,
  suspendHighInteraction,
  type InteractionPreference,
} from '../../features/interactive/live/presentationPreference';
import { pinnedMotion } from '../../components/presentation-capabilities';
import { useAmbience } from '../../features/ambience/AmbienceProvider';

const selectClass = 'rounded-md border border-input bg-background px-2.5 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-ring';

export function AppearanceSettings({ onResetLayout }: { onResetLayout: () => void }) {
  const { preferences, setPreferences, resetPreferences } = useUiPreferences();
  // The interaction mode is a presentation preference with its own versioned
  // key (MASTER_ARCHITECTURE §5.3), so it is read/written directly rather than
  // folded into the theme blob. §5.2 requires it to be selectable from a
  // clearly named interaction control — this is that control.
  const [interaction, setInteraction] = useState<InteractionPreference>(() => loadPreference());
  const applyInteraction = (value: InteractionPreference) => {
    setInteraction(value);
    savePreference(value);
    // Simple and High Interaction both render the live instance view, so both
    // resume it; only Standard suspends.
    if (value === 'standard') suspendHighInteraction();
    else resumeHighInteractionView();
  };
  // Simple pins motion to `reduced` (PresentationMotionCoordinator applies it).
  // Showing the control as disabled beats letting the user pick a value that is
  // silently written back a moment later.
  const motionPin = pinnedMotion(interaction);

  return (
    <div id="settings-appearance" className="scroll-mt-24 rounded-xl border border-border bg-card p-4 space-y-4" data-testid="appearance-settings">
      <div>
        <h3 className="font-semibold">Appearance</h3>
        <p className="mt-1 text-xs text-muted-foreground">Color, readability, spacing, and motion preferences apply immediately.</p>
      </div>

      <label className="block space-y-1 text-sm">
        <span className="font-medium">Appearance preset</span>
        <select
          aria-label="Appearance preset"
          defaultValue=""
          onChange={(event) => {
            const preset = APPEARANCE_PRESETS[event.target.value];
            if (preset) setPreferences(preset.preferences);
            event.currentTarget.value = '';
          }}
          className={`${selectClass} block w-full sm:w-72`}
        >
          <option value="" disabled>Choose a preset…</option>
          {Object.entries(APPEARANCE_PRESETS).map(([id, preset]) => <option key={id} value={id}>{preset.label}</option>)}
        </select>
      </label>

      <div className="grid gap-4 sm:grid-cols-2">
        <label className="space-y-1 text-sm">
          <span className="font-medium">Color mode</span>
          <select
            aria-label="Color mode"
            value={preferences.colorMode}
            onChange={(event) => setPreferences({ colorMode: event.target.value as typeof preferences.colorMode })}
            className={`${selectClass} block w-full`}
          >
            <option value="system">Follow system</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </label>

        <label className="space-y-1 text-sm">
          <span className="font-medium">Accent</span>
          <select
            aria-label="Accent source"
            value={preferences.accentMode}
            onChange={(event) => setPreferences({ accentMode: event.target.value as typeof preferences.accentMode })}
            className={`${selectClass} block w-full`}
          >
            <option value="agora">Agora teal</option>
            <option value="system">Windows accent</option>
            <option value="custom">Custom</option>
          </select>
        </label>
      </div>

      {preferences.accentMode === 'custom' && (
        <label className="flex items-center justify-between gap-4 text-sm">
          <span>
            <span className="block font-medium">Custom accent color</span>
            <span className="text-xs text-muted-foreground">Used for primary actions, selection, focus, and hover surfaces.</span>
          </span>
          <input
            type="color"
            aria-label="Custom accent color"
            value={preferences.customAccent}
            onChange={(event) => setPreferences({ customAccent: event.target.value })}
            className="h-9 w-14 cursor-pointer rounded border border-input bg-background p-1"
          />
        </label>
      )}

      <details className="group rounded-lg border border-border bg-muted">
        <summary aria-label="Toggle custom colors" className="cursor-pointer select-none px-3 py-2.5 text-sm font-semibold">
          Custom colors
          <span className="ml-2 text-xs font-normal text-muted-foreground">Block, navigation, background, and text colors</span>
        </summary>
        <div className="grid gap-4 border-t border-border p-3 lg:grid-cols-2">
          <div className="space-y-3 rounded-lg border border-border bg-card p-3">
          <label className="flex items-center justify-between gap-3 text-sm">
            <span>
              <span className="block font-medium">Custom block color</span>
              <span className="text-xs text-muted-foreground">Cards, panels, dialogs, and content blocks.</span>
            </span>
            <input type="checkbox" aria-label="Use custom block color" checked={preferences.surfaceMode === 'custom'} onChange={(event) => setPreferences({ surfaceMode: event.target.checked ? 'custom' : 'theme' })} className="h-5 w-5 accent-primary" />
          </label>
          {preferences.surfaceMode === 'custom' && (
            <input type="color" aria-label="Block color" value={preferences.customSurface} onChange={(event) => setPreferences({ customSurface: event.target.value })} className="h-9 w-full cursor-pointer rounded border border-input bg-background p-1" />
          )}
          <label className="block space-y-1 text-xs">
            <span className="flex justify-between"><span>Block opacity</span><span>{Math.round(preferences.surfaceOpacity * 100)}%</span></span>
            <input type="range" aria-label="Block opacity" min="0.35" max="1" step="0.05" value={preferences.surfaceOpacity} onChange={(event) => setPreferences({ surfaceOpacity: Number(event.target.value) })} className="w-full accent-primary" />
          </label>
          </div>
          <div className="space-y-3 rounded-lg border border-border bg-card p-3">
          <label className="flex items-center justify-between gap-3 text-sm">
            <span>
              <span className="block font-medium">Custom navigation color</span>
              <span className="text-xs text-muted-foreground">Sidebar background and translucency.</span>
            </span>
            <input type="checkbox" aria-label="Use custom navigation color" checked={preferences.navMode === 'custom'} onChange={(event) => setPreferences({ navMode: event.target.checked ? 'custom' : 'theme' })} className="h-5 w-5 accent-primary" />
          </label>
          {preferences.navMode === 'custom' && (
            <input type="color" aria-label="Navigation color" value={preferences.customNav} onChange={(event) => setPreferences({ customNav: event.target.value })} className="h-9 w-full cursor-pointer rounded border border-input bg-background p-1" />
          )}
          <label className="block space-y-1 text-xs">
            <span className="flex justify-between"><span>Navigation opacity</span><span>{Math.round(preferences.navOpacity * 100)}%</span></span>
            <input type="range" aria-label="Navigation opacity" min="0.35" max="1" step="0.05" value={preferences.navOpacity} onChange={(event) => setPreferences({ navOpacity: Number(event.target.value) })} className="w-full accent-primary" />
          </label>
          </div>
          <div className="space-y-2 rounded-lg border border-border bg-muted p-3">
          <label className="flex items-center justify-between gap-3 text-sm">
            <span>
              <span className="block font-medium">Custom background</span>
              <span className="text-xs text-muted-foreground">Override the page background color.</span>
            </span>
            <input type="checkbox" aria-label="Use custom background" checked={preferences.backgroundMode === 'custom'} onChange={(event) => setPreferences({ backgroundMode: event.target.checked ? 'custom' : 'theme' })} className="h-5 w-5 accent-primary" />
          </label>
          {preferences.backgroundMode === 'custom' && (
            <input type="color" aria-label="Background color" value={preferences.customBackground} onChange={(event) => setPreferences({ customBackground: event.target.value })} className="h-9 w-full cursor-pointer rounded border border-input bg-background p-1" />
          )}
          </div>
          <div className="space-y-2 rounded-lg border border-border bg-muted p-3">
          <label className="flex items-center justify-between gap-3 text-sm">
            <span>
              <span className="block font-medium">Custom block text</span>
              <span className="text-xs text-muted-foreground">Primary text inside cards and controls.</span>
            </span>
            <input type="checkbox" aria-label="Use custom text color" checked={preferences.textMode === 'custom'} onChange={(event) => setPreferences({ textMode: event.target.checked ? 'custom' : 'theme' })} className="h-5 w-5 accent-primary" />
          </label>
          {preferences.textMode === 'custom' && (
            <input type="color" aria-label="Block text color" value={preferences.customText} onChange={(event) => setPreferences({ customText: event.target.value })} className="h-9 w-full cursor-pointer rounded border border-input bg-background p-1" />
          )}
          </div>
          <div className="space-y-2 rounded-lg border border-border bg-muted p-3 lg:col-span-2">
          <label className="flex items-center justify-between gap-3 text-sm">
            <span>
              <span className="block font-medium">Custom background text</span>
              <span className="text-xs text-muted-foreground">Headings and text directly on the page background.</span>
            </span>
            <input type="checkbox" aria-label="Use custom background text color" checked={preferences.backgroundTextMode === 'custom'} onChange={(event) => setPreferences({ backgroundTextMode: event.target.checked ? 'custom' : 'theme' })} className="h-5 w-5 accent-primary" />
          </label>
          {preferences.backgroundTextMode === 'custom' && (
            <input type="color" aria-label="Background text color" value={preferences.customBackgroundText} onChange={(event) => setPreferences({ customBackgroundText: event.target.value })} className="h-9 w-full cursor-pointer rounded border border-input bg-background p-1" />
          )}
          </div>
        </div>
      </details>

      <div className="grid gap-4 sm:grid-cols-3">
        <label className="space-y-1 text-sm">
          <span className="font-medium">Font</span>
          <select aria-label="Interface font" value={preferences.fontFamily} onChange={(event) => setPreferences({ fontFamily: event.target.value as typeof preferences.fontFamily })} className={`${selectClass} block w-full`}>
            <option value="system">System</option>
            <option value="readable">High readability</option>
            <option value="rounded">Rounded</option>
            <option value="serif">Bookish serif</option>
            <option value="mono">Terminal mono</option>
            <option value="playful">Playful</option>
            <option value="typewriter">Typewriter</option>
          </select>
        </label>
        <label className="space-y-1 text-sm">
          <span className="font-medium">Density</span>
          <select aria-label="Information density" value={preferences.density} onChange={(event) => setPreferences({ density: event.target.value as typeof preferences.density })} className={`${selectClass} block w-full`}>
            <option value="compact">Compact</option>
            <option value="comfortable">Comfortable</option>
            <option value="spacious">Spacious</option>
          </select>
        </label>
        <label className="space-y-1 text-sm">
          <span className="font-medium">Corners</span>
          <select aria-label="Corner style" value={preferences.cornerStyle} onChange={(event) => setPreferences({ cornerStyle: event.target.value as typeof preferences.cornerStyle })} className={`${selectClass} block w-full`}>
            <option value="square">Square</option>
            <option value="soft">Soft</option>
            <option value="round">Round</option>
          </select>
        </label>
      </div>

      <label className="block space-y-1 text-sm">
        <span className="flex justify-between font-medium"><span>Text scale</span><span>{Math.round(preferences.fontScale * 100)}%</span></span>
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
      </label>

      <div className="grid gap-3 sm:grid-cols-2">
        <label className="flex items-center justify-between gap-3 text-sm sm:col-span-2">
          <span>High contrast</span>
          <input type="checkbox" aria-label="High contrast" checked={preferences.highContrast} onChange={(event) => setPreferences({ highContrast: event.target.checked })} className="h-5 w-5 accent-primary" />
        </label>
        <label className="space-y-1 text-sm sm:col-span-2">
          <span className="font-medium">Motion</span>
          <select
            aria-label="Motion preference"
            value={motionPin ?? preferences.motion}
            disabled={motionPin !== null}
            onChange={(event) => setPreferences({ motion: event.target.value as typeof preferences.motion })}
            className={`${selectClass} block w-full sm:w-64 disabled:opacity-50`}
          >
            <option value="system">Follow system</option>
            <option value="reduced">Reduce motion</option>
            <option value="full">Full motion</option>
          </select>
          <span className="block text-xs text-muted-foreground">
            Controls nonessential animations, transitions, and smooth scrolling throughout the app.
            {motionPin === 'reduced' ? ' Simple interaction mode keeps this on Reduce motion.' : ''}
          </span>
        </label>
      </div>

      <div className="space-y-2 rounded-lg border border-border bg-muted p-3">
        <label className="block space-y-1 text-sm">
          <span className="font-medium">Interaction mode</span>
          <select
            aria-label="Interaction mode"
            value={interaction}
            onChange={(event) => applyInteraction(event.target.value as InteractionPreference)}
            className={`${selectClass} block w-full sm:w-72`}
          >
            <option value="standard">Standard</option>
            <option value="simple">Simple</option>
            <option value="high-interaction">High Interaction</option>
          </select>
          <span className="block text-xs text-muted-foreground">
            Standard is the full launcher. High Interaction turns an instance into a visual workbench and
            Browse into the Bazaar. Simple is the quiet version of that workbench — the same big Play
            button, shelf and pre-flight check with the decoration, scores and surprises removed, plain
            Browse sorted by Best, and reduced motion (so the living background stays off). Reviews and
            content changes always open the Standard screens, which remain the only place a change is
            applied.
          </span>
        </label>
      </div>

      <LivingBackgroundSettings />

      <div className="flex flex-wrap gap-2">
        <button type="button" onClick={resetPreferences} className="rounded-md border border-input px-3 py-1.5 text-sm font-medium hover:bg-accent">
          Reset appearance
        </button>
        <button type="button" onClick={onResetLayout} className="rounded-md border border-input px-3 py-1.5 text-sm font-medium hover:bg-accent">
          Reset layout
        </button>
      </div>
    </div>
  );
}

/**
 * Living background (ambience) settings. The ambience layer is the only thing
 * allowed to touch these keys. One toggle, one music volume, one sound
 * toggle, and a reduce-motion note that points at the OS/app motion setting.
 */
function LivingBackgroundSettings() {
  const { enabled, setEnabled, soundOn, setSoundOn, soundVolume, setSoundVolume, musicVolume, setMusicVolume, musicOn, setMusicOn, clearBackground, setClearBackground, motionSuppressed } = useAmbience();
  const [loaded, setLoaded] = useState(false);
  useEffect(() => { setLoaded(true); }, []);
  if (!loaded) return null;

  return (
    <div className="space-y-3 rounded-lg border border-border bg-muted p-3">
      <label className="flex items-start gap-3 text-sm">
        <input
          type="checkbox"
          aria-label="Living background"
          checked={enabled}
          disabled={motionSuppressed}
          onChange={(event) => setEnabled(event.target.checked)}
          className="mt-0.5 h-5 w-5 accent-primary disabled:opacity-50"
        />
        <span>
          <span className="block font-medium">Living background</span>
          <span className="block text-xs text-muted-foreground">
            A gentle living world — hills, weather, animals and small discoveries — behind every page.
            Purely decorative.
          </span>
          {motionSuppressed ? (
            <span className="mt-1 block text-xs text-muted-foreground">
              Off because motion is reduced — the world is movement, so there is nothing honest to show
              while it is held still. Set Motion to Full (or leave Simple interaction mode) to bring it
              back; your other ambience settings are kept.
            </span>
          ) : null}
        </span>
      </label>

      {enabled && (
        <>
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-1 text-sm">
              <label className="flex items-center gap-3">
                <input
                  type="checkbox"
                  aria-label="Ambient sounds"
                  checked={soundOn}
                  onChange={(event) => setSoundOn(event.target.checked)}
                  className="h-5 w-5 accent-primary"
                />
                <span>Ambient sounds</span>
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="range"
                  aria-label="Ambient sound volume"
                  min="0"
                  max="100"
                  step="5"
                  disabled={!soundOn}
                  value={Math.round(soundVolume * 100)}
                  onChange={(event) => setSoundVolume(Number(event.target.value) / 100)}
                  className="w-full accent-primary disabled:opacity-40"
                />
                <span className="w-10 text-right text-xs text-muted-foreground">{Math.round(soundVolume * 100)}%</span>
              </div>
            </div>
            <div className="space-y-1 text-sm">
              <span className="flex items-center gap-2">
                <span>Music</span>
                <button
                  type="button"
                  onClick={() => setMusicOn(!musicOn)}
                  aria-pressed={musicOn}
                  aria-label="Toggle music"
                  className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-semibold ${
                    musicOn
                      ? 'border-primary bg-primary text-primary-foreground'
                      : 'border-border bg-muted text-muted-foreground hover:text-foreground'
                  }`}
                >
                  {musicOn ? 'Playing' : 'Muted'}
                </button>
                <span className="ml-auto text-xs text-muted-foreground">{Math.round(musicVolume * 100)}%</span>
              </span>
              <input
                type="range"
                aria-label="Music volume"
                min="0"
                max="100"
                step="5"
                disabled={!musicOn}
                value={Math.round(musicVolume * 100)}
                onChange={(event) => setMusicVolume(Number(event.target.value) / 100)}
                className="w-full accent-primary disabled:opacity-40"
              />
            </div>
          </div>

          <label className="flex items-start gap-3 text-sm">
            <input
              type="checkbox"
              aria-label="Hide the standard background"
              checked={clearBackground}
              onChange={(event) => setClearBackground(event.target.checked)}
              className="mt-0.5 h-5 w-5 accent-primary"
            />
            <span>
              <span className="block font-medium">Hide the standard background</span>
              <span className="block text-xs text-muted-foreground">
                Sets the page background to 0% opacity so the living world shows through
                unobstructed (cards and panels stay readable). Turning the living background off turns this off
                with it.
              </span>
            </span>
          </label>

          <p className="text-xs text-muted-foreground">
            Reducing motion switches the living background off entirely.
          </p>
        </>
      )}
    </div>
  );
}
