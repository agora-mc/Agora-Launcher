/**
 * Live region announcements for interactive visuals.
 *
 * Sol-0 contract: `docs/interactive/VISUAL_LANGUAGE.md` §11 — live regions
 * announce summaries rather than streaming decorative movement. Under reduced
 * motion the same concise announcement is used.
 */

export function Announcement({ message, id }: { message: string | null; id?: string }) {
  if (!message) return null;
  return (
    <div id={id} aria-live="polite" className="sr-only">
      {message}
    </div>
  );
}
