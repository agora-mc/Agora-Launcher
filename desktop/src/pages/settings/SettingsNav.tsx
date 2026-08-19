import type { ComponentType, KeyboardEvent } from 'react';

export interface SettingsNavItem {
  id: string;
  label: string;
  icon?: ComponentType<{ className?: string }>;
  /** Short line under the label in the rail. Omitted from the sub-nav pills. */
  hint?: string;
}

/**
 * Roving-focus arrow-key handling shared by both nav levels. `role="tablist"`
 * owns exactly one tab stop; Arrow/Home/End move the selection *and* the focus,
 * which is what a screen-reader user expects from a tab strip.
 */
function useArrowKeys(
  items: SettingsNavItem[],
  activeId: string,
  onSelect: (id: string) => void,
  orientation: 'vertical' | 'horizontal',
  idPrefix: string,
) {
  const prevKey = orientation === 'vertical' ? 'ArrowUp' : 'ArrowLeft';
  const nextKey = orientation === 'vertical' ? 'ArrowDown' : 'ArrowRight';

  return (event: KeyboardEvent<HTMLDivElement>) => {
    const index = items.findIndex((item) => item.id === activeId);
    if (index < 0) return;
    let target = -1;
    if (event.key === prevKey) target = (index - 1 + items.length) % items.length;
    else if (event.key === nextKey) target = (index + 1) % items.length;
    else if (event.key === 'Home') target = 0;
    else if (event.key === 'End') target = items.length - 1;
    if (target < 0) return;
    event.preventDefault();
    const next = items[target];
    onSelect(next.id);
    // The newly selected tab is the only one with tabIndex 0 after this
    // render, so focus has to follow it explicitly.
    requestAnimationFrame(() => {
      document.getElementById(`${idPrefix}${next.id}`)?.focus();
    });
  };
}

/**
 * Primary settings navigation — a vertical rail on wide windows, a horizontal
 * scroller when the content column gets narrow.
 */
export function SettingsTabRail({
  items,
  activeId,
  onSelect,
}: {
  items: SettingsNavItem[];
  activeId: string;
  onSelect: (id: string) => void;
}) {
  const handleKeyDown = useArrowKeys(items, activeId, onSelect, 'vertical', 'settings-tab-');

  return (
    <nav aria-label="Settings sections" className="settings-rail">
      <div
        role="tablist"
        aria-orientation="vertical"
        aria-label="Settings sections"
        onKeyDown={handleKeyDown}
        className="settings-rail-list"
      >
        {items.map(({ id, label, icon: Icon, hint }) => {
          const selected = id === activeId;
          return (
            <button
              key={id}
              id={`settings-tab-${id}`}
              type="button"
              role="tab"
              aria-selected={selected}
              aria-controls={`settings-panel-${id}`}
              // The hint is a description, not part of the name: folded into
              // the name it makes every tab match half the words on the page.
              aria-label={label}
              aria-describedby={hint ? `settings-tab-hint-${id}` : undefined}
              tabIndex={selected ? 0 : -1}
              onClick={() => onSelect(id)}
              className="settings-tab"
            >
              {Icon && (
                <span className="settings-tab-icon" aria-hidden="true">
                  <Icon className="h-4 w-4" />
                </span>
              )}
              <span className="min-w-0">
                <span className="block truncate font-medium">{label}</span>
                {hint && <span id={`settings-tab-hint-${id}`} className="settings-tab-hint">{hint}</span>}
              </span>
            </button>
          );
        })}
      </div>
    </nav>
  );
}

/** Secondary navigation — the sub-pages of the selected section. */
export function SettingsSubNav({
  items,
  activeId,
  onSelect,
  label,
}: {
  items: SettingsNavItem[];
  activeId: string;
  onSelect: (id: string) => void;
  label: string;
}) {
  const handleKeyDown = useArrowKeys(items, activeId, onSelect, 'horizontal', 'settings-subtab-');

  return (
    <div
      role="tablist"
      aria-label={label}
      onKeyDown={handleKeyDown}
      className="settings-subnav"
    >
      {items.map(({ id, label: itemLabel, icon: Icon }) => {
        const selected = id === activeId;
        return (
          <button
            key={id}
            id={`settings-subtab-${id}`}
            type="button"
            role="tab"
            aria-selected={selected}
            aria-controls={`settings-subpanel-${id}`}
            tabIndex={selected ? 0 : -1}
            onClick={() => onSelect(id)}
            className="settings-subtab"
          >
            {Icon && <Icon className="h-3.5 w-3.5" aria-hidden="true" />}
            {itemLabel}
          </button>
        );
      })}
    </div>
  );
}
