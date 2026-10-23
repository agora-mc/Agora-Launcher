interface BrandMarkProps {
  className?: string;
  compact?: boolean;
}

export function BrandMark({ className = '', compact = false }: BrandMarkProps) {
  return (
    <div className={`flex items-center gap-3 ${className}`.trim()}>
      <img
        src="/brand/agora-launcher-icon.png"
        alt=""
        aria-hidden="true"
        className="h-10 w-10 rounded-lg object-cover shadow-xs ring-1 ring-midnight/10"
      />
      {!compact && (
        <div className="min-w-0">
          {/* The wordmark scales with the text-size setting but at 60% of the rate: it is a logo,
              and at 200% the full rate made "Launcher" wider than the sidebar and broke it
              mid-word. Words wrap whole; a word is never split. */}
          <p className="text-[length:calc(1rem*(1+(var(--font-scale)-1)*0.6))] font-semibold leading-tight tracking-[0.08em] text-foreground [overflow-wrap:normal]">Agora Launcher</p>
          <p className="mt-0.5 text-[10px] font-medium uppercase tracking-[0.2em] text-primary">Open to all</p>
        </div>
      )}
    </div>
  );
}
