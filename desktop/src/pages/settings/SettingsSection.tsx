import { useId, type ComponentType, type ReactNode } from 'react';

/**
 * One settings card. Every card in the redesigned settings page uses this so
 * the icon badge, title, and description line up across tabs — previously each
 * card hand-rolled a bare `<h3>` and they drifted apart.
 *
 * The heading keeps its plain accessible name (the icon is decorative) and the
 * optional `id` stays on the outer card, so existing anchors such as
 * `#settings-launching` and `#settings-privacy` still resolve to the whole
 * section rather than to a header fragment.
 */
export function SettingsSection({
  id,
  icon: Icon,
  title,
  description,
  action,
  className = '',
  contentClassName = 'space-y-3',
  children,
  ...rest
}: {
  id?: string;
  icon?: ComponentType<{ className?: string }>;
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  className?: string;
  contentClassName?: string;
  children?: ReactNode;
  'data-testid'?: string;
}) {
  // A bare <section> is an unnamed landmark; naming it from its own heading is
  // what makes the card navigable rather than "region, region, region".
  const headingId = useId();

  return (
    <section
      id={id}
      aria-labelledby={headingId}
      className={`settings-card scroll-mt-24 rounded-xl border border-border bg-card p-4 ${className}`}
      {...rest}
    >
      <div className="mb-3 flex items-start gap-3">
        {Icon && (
          <span className="settings-card-icon" aria-hidden="true">
            <Icon className="h-4 w-4" />
          </span>
        )}
        <div className="min-w-0 flex-1">
          <h3 id={headingId} className="font-semibold leading-tight">{title}</h3>
          {description && (
            <p className="mt-1 text-xs text-muted-foreground">{description}</p>
          )}
        </div>
        {action}
      </div>
      <div className={contentClassName}>{children}</div>
    </section>
  );
}
