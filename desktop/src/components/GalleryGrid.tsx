import { useState } from 'react';
import { HideOnErrorImage } from './HideOnErrorImage';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from './ui/dialog';

/**
 * Screenshot grid with a larger view. Thumbnails and the enlarged image both go
 * through HideOnErrorImage, so they take the same verified-image route as every
 * other community image. The dialog is the app's own (Radix) one: focus stays
 * inside it, Escape closes it, and focus returns to the thumbnail.
 */
export function GalleryGrid({ urls, name }: { urls: string[]; name: string }) {
  const [open, setOpen] = useState<number | null>(null);
  const label = (index: number) => `${name} screenshot ${index + 1}`;
  return (
    <>
      <div className="grid grid-cols-2 gap-3">
        {urls.map((url, index) => (
          <button
            key={index}
            type="button"
            onClick={() => setOpen(index)}
            aria-label={`View ${label(index)} larger`}
            className="block rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring empty:hidden"
          >
            <HideOnErrorImage src={url} alt={label(index)} className="rounded-lg border border-border w-full h-48 object-cover" />
          </button>
        ))}
      </div>
      <Dialog open={open !== null} onOpenChange={(next) => { if (!next) setOpen(null); }}>
        <DialogContent className="max-w-[min(64rem,calc(100vw-2rem))]">
          <DialogTitle>{open === null ? '' : label(open)}</DialogTitle>
          <DialogDescription className="sr-only">
            Enlarged screenshot {open === null ? '' : `${open + 1} of ${urls.length}`}. Press Escape to close.
          </DialogDescription>
          {open !== null && (
            <HideOnErrorImage key={urls[open]} src={urls[open]} alt={label(open)} className="mx-auto max-h-[70vh] w-auto max-w-full rounded-lg object-contain" />
          )}
          {urls.length > 1 && open !== null && (
            <div className="flex justify-between gap-2">
              <button
                type="button"
                onClick={() => setOpen((open + urls.length - 1) % urls.length)}
                className="rounded-lg border border-input px-3 py-1.5 text-sm font-medium hover:bg-accent"
              >
                Previous
              </button>
              <button
                type="button"
                onClick={() => setOpen((open + 1) % urls.length)}
                className="rounded-lg border border-input px-3 py-1.5 text-sm font-medium hover:bg-accent"
              >
                Next
              </button>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}
