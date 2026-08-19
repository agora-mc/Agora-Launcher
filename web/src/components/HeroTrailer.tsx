import Image from 'next/image';
import { TRAILER_SRC, TRAILER_POSTER } from '@/lib/site';

/**
 * HeroTrailer — the 16:9 centrepiece of the homepage hero.
 *
 * Renders the trailer when `TRAILER_SRC` is set (see lib/site.ts), and a
 * framed placeholder until then. Both states occupy exactly the same box, so
 * dropping the real video in causes no layout shift.
 *
 * No autoplay: the visitor presses play. An auto-playing video over a living
 * animated background would be two things competing for attention, and muted
 * autoplay still costs mobile visitors bandwidth they did not ask for.
 */
export function HeroTrailer() {
  return (
    <div className="mx-auto mt-10 w-full max-w-3xl">
      <div className="relative overflow-hidden rounded-2xl border border-gold/35 bg-canvas-deep/85 shadow-glow">
        {/* 16:9 */}
        <div className="relative aspect-video w-full">
          {TRAILER_SRC ? (
            <video
              className="absolute inset-0 h-full w-full object-cover"
              controls
              preload="metadata"
              playsInline
              poster={TRAILER_POSTER ?? undefined}
            >
              <source src={TRAILER_SRC} type="video/mp4" />
            </video>
          ) : (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-4 bg-[radial-gradient(120%_100%_at_50%_0%,rgba(194,139,40,0.16),transparent_65%)]">
              <Image
                src="/agora-logo.png"
                alt=""
                width={72}
                height={72}
                className="h-16 w-16 rounded-xl opacity-70 ring-1 ring-gold/30"
              />
              <div className="flex items-center gap-2.5 text-gold-bright">
                <span
                  aria-hidden="true"
                  className="flex h-9 w-9 items-center justify-center rounded-full border border-gold/50 pl-0.5 text-sm"
                >
                  &#9654;
                </span>
                <span className="ui-text text-sm font-semibold uppercase tracking-[0.16em]">
                  Trailer coming soon
                </span>
              </div>
              <p className="ui-text max-w-xs text-center text-xs leading-5 text-ink-muted/80">
                A look at instances, dependency-aware installs, and recovery points &mdash; in motion.
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
