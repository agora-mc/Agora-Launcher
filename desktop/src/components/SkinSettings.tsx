import { useCallback, useEffect, useRef, useState } from 'react';
import {
  addSkinToLibrary,
  formatError,
  removeSkinFromLibrary,
  resetSkin,
  setCape,
  skinLibrary,
  skinProfile,
  wearSavedSkin,
  type SavedSkin,
  type SkinProfile,
  type SkinVariant,
} from '@/lib/tauri';

/** Front-view regions of a 64×64 skin: [sx, sy, w, h, dx, dy] with overlays after the base. */
function frontParts(slim: boolean, legacy: boolean): number[][] {
  const arm = slim ? 3 : 4;
  const base = [
    [8, 8, 8, 8, 4, 0], // head
    [20, 20, 8, 12, 4, 8], // body
    [44, 20, arm, 12, 4 - arm, 8], // right arm (viewer's left)
    // Legacy 64×32 skins mirror the right limbs for the left ones.
    legacy ? [44, 20, arm, 12, 12, 8] : [36, 52, arm, 12, 12, 8], // left arm
    [4, 20, 4, 12, 4, 20], // right leg
    legacy ? [4, 20, 4, 12, 8, 20] : [20, 52, 4, 12, 8, 20], // left leg
  ];
  const overlay = legacy
    ? [[40, 8, 8, 8, 4, 0]]
    : [
        [40, 8, 8, 8, 4, 0],
        [20, 36, 8, 12, 4, 8],
        [44, 36, arm, 12, 4 - arm, 8],
        [52, 52, arm, 12, 12, 8],
        [4, 36, 4, 12, 4, 20],
        [4, 52, 4, 12, 8, 20],
      ];
  return [...base, ...overlay];
}

/** A flat front view of a skin, drawn pixel-for-pixel. */
export function SkinPreview({ src, variant, scale = 8 }: { src: string | null; variant: SkinVariant; scale?: number }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const target = canvas.current;
    const context = target?.getContext('2d');
    if (!target || !context) return;
    context.clearRect(0, 0, target.width, target.height);
    if (!src) return;
    const image = new Image();
    image.onload = () => {
      context.imageSmoothingEnabled = false;
      const legacy = image.height === 32;
      for (const [sx, sy, w, h, dx, dy] of frontParts(variant === 'slim', legacy)) {
        context.drawImage(image, sx, sy, w, h, dx * scale, dy * scale, w * scale, h * scale);
      }
    };
    image.src = src;
  }, [src, variant, scale]);
  return <canvas ref={canvas} width={16 * scale} height={32 * scale} role="img" aria-label="Skin preview" className="[image-rendering:pixelated]" />;
}

/** Skin library, current skin and cape choice for the active Microsoft account. */
export function SkinSettings() {
  const [profile, setProfile] = useState<SkinProfile | null>(null);
  const [library, setLibrary] = useState<SavedSkin[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [newVariant, setNewVariant] = useState<SkinVariant>('classic');

  const reload = useCallback(async () => {
    setError(null);
    try {
      const saved = await skinLibrary();
      setLibrary(Array.isArray(saved) ? saved : []);
    } catch {
      setLibrary([]);
    }
    try {
      setProfile(await skinProfile());
    } catch (e) {
      setProfile(null);
      setError(formatError(e));
    }
  }, []);

  useEffect(() => { void reload(); }, [reload]);

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await reload();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const button = 'rounded-md border border-input px-2 py-1 text-xs font-medium hover:bg-accent disabled:opacity-50';

  return (
    <section className="rounded-xl border border-border bg-card p-4 space-y-4" aria-labelledby="skin-heading">
      <div>
        <h3 id="skin-heading" className="font-semibold text-sm">Skin and cape</h3>
        <p className="text-xs text-muted-foreground">
          For the Microsoft account Agora launches with. Changes apply to your account everywhere, like changing them on minecraft.net.
        </p>
      </div>

      {profile ? (
        <div className="flex flex-wrap items-start gap-6">
          <div className="space-y-2 text-center">
            <SkinPreview src={profile.skin_data_url} variant={profile.variant} />
            <p className="text-xs text-muted-foreground">{profile.username} · {profile.variant === 'slim' ? 'Slim arms' : 'Classic arms'}</p>
            <button type="button" className={button} disabled={busy} onClick={() => void run(resetSkin)}>Reset to default</button>
          </div>
          <div className="min-w-48 flex-1 space-y-2">
            <p className="text-sm font-medium">Cape</p>
            {profile.capes.length === 0 ? (
              <p className="text-xs text-muted-foreground">This account has no capes.</p>
            ) : (
              <div className="flex flex-wrap gap-2" role="radiogroup" aria-label="Cape">
                <label className="flex items-center gap-1.5 text-sm">
                  <input type="radio" name="cape" disabled={busy} checked={!profile.capes.some((c) => c.active)} onChange={() => void run(() => setCape(null))} className="accent-primary" />
                  None
                </label>
                {profile.capes.map((cape) => (
                  <label key={cape.id} className="flex items-center gap-1.5 text-sm">
                    <input type="radio" name="cape" disabled={busy} checked={cape.active} onChange={() => void run(() => setCape(cape.id))} className="accent-primary" />
                    {cape.image_data_url && (
                      <span className="inline-block h-8 w-5 overflow-hidden rounded-sm" aria-hidden="true">
                        {/* Cape front: 10×16 at (1, 1) of the 64×32 texture. */}
                        <img src={cape.image_data_url} alt="" className="max-w-none [image-rendering:pixelated]" style={{ width: 128, height: 64, marginLeft: -2, marginTop: -2 }} />
                      </span>
                    )}
                    {cape.alias}
                  </label>
                ))}
              </div>
            )}
          </div>
        </div>
      ) : (
        !error && <p className="text-xs text-muted-foreground">Loading your skin…</p>
      )}

      <div className="space-y-2">
        <p className="text-sm font-medium">Saved skins</p>
        {library.length === 0 && <p className="text-xs text-muted-foreground">Add skin files here to switch between them in one click.</p>}
        <ul className="flex flex-wrap gap-3">
          {library.map((skin) => (
            <li key={skin.id} className="space-y-1 rounded-lg border border-border p-2 text-center">
              <SkinPreview src={skin.data_url} variant={skin.variant} scale={3} />
              <p className="max-w-24 truncate text-xs" title={skin.name}>{skin.name}</p>
              <div className="flex justify-center gap-1">
                <button type="button" className={button} disabled={busy || !profile} onClick={() => void run(() => wearSavedSkin(skin.id))}>Wear</button>
                <button type="button" className={button} disabled={busy} onClick={() => void run(() => removeSkinFromLibrary(skin.id))} aria-label={`Remove ${skin.name}`}>✕</button>
              </div>
            </li>
          ))}
        </ul>
        <div className="flex flex-wrap items-center gap-2">
          <select aria-label="Arm width for the new skin" value={newVariant} onChange={(e) => setNewVariant(e.target.value as SkinVariant)} className="rounded-lg border border-input bg-background px-2 py-1 text-xs">
            <option value="classic">Classic arms</option>
            <option value="slim">Slim arms</option>
          </select>
          <button type="button" className={button} disabled={busy} onClick={() => void run(() => addSkinToLibrary('', newVariant))}>Add skin file…</button>
        </div>
      </div>

      {error && <p className="text-xs text-destructive">{error}</p>}
    </section>
  );
}
