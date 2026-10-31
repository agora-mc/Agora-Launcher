/**
 * What a not-yet-built slot shows. Each slice replaces its own placeholder in its
 * `index.tsx`; the shell never has to change for that.
 */

interface SlotPlaceholderProps {
  title: string;
  /** The slice that will build this slot, named for whoever picks it up. */
  slice: string;
}

export function SlotPlaceholder({ title, slice }: SlotPlaceholderProps) {
  return (
    <section
      className="rounded-xl border border-dashed border-border bg-card/60 p-6 text-sm text-muted-foreground"
      data-testid="game-slot-placeholder"
      data-slice={slice}
    >
      <h2 className="text-base font-semibold text-foreground">{title}</h2>
      <p className="mt-2">This part of the game view is not built yet.</p>
    </section>
  );
}
