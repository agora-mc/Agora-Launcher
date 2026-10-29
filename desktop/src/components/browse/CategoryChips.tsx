import { useRef, useState, type KeyboardEvent } from 'react';

export interface CategoryChip {
  id: string;
  label: string;
}

/**
 * The Browse category chips as one radio group with a single tab stop.
 *
 * A source can offer well over a hundred categories. As plain buttons each was a
 * Tab stop between the search box and the results, so reaching the first result
 * took a hundred key presses. Now Tab enters the group once (on the chosen chip,
 * or "All"), the arrow keys move between chips, Home / End jump to the ends, and
 * Enter or Space chooses. Arrows move focus without choosing, because choosing
 * runs a search.
 *
 * `role="radio"` on a `<button>` also keeps the chips reachable with a
 * controller: its navigation treats roving radios as candidates.
 */
export function CategoryChips({
  chips,
  selected,
  onSelect,
  label,
  children,
}: {
  chips: CategoryChip[];
  /** The chosen chip id, or null for "All". */
  selected: string | null;
  onSelect: (id: string | null) => void;
  label: string;
  children?: React.ReactNode;
}) {
  const items: Array<{ id: string | null; label: string }> = [{ id: null, label: 'All' }, ...chips];
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const [focusedIndex, setFocusedIndex] = useState<number | null>(null);

  const selectedIndex = Math.max(0, items.findIndex((item) => item.id === selected));
  const stopIndex = focusedIndex !== null && focusedIndex < items.length ? focusedIndex : selectedIndex;

  const move = (to: number) => {
    const next = (to + items.length) % items.length;
    setFocusedIndex(next);
    refs.current[next]?.focus();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    switch (event.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        event.preventDefault();
        move(index + 1);
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        event.preventDefault();
        move(index - 1);
        break;
      case 'Home':
        event.preventDefault();
        move(0);
        break;
      case 'End':
        event.preventDefault();
        move(items.length - 1);
        break;
      default:
    }
  };

  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="flex flex-wrap gap-2"
      onBlur={(event) => {
        // Leaving the group puts the tab stop back on the chosen chip.
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocusedIndex(null);
      }}
    >
      {items.map((item, index) => (
        <button
          type="button"
          role="radio"
          aria-checked={item.id === selected}
          key={item.id ?? '__all'}
          ref={(node) => { refs.current[index] = node; }}
          tabIndex={index === stopIndex ? 0 : -1}
          onClick={() => onSelect(item.id)}
          onFocus={() => setFocusedIndex(index)}
          onKeyDown={(event) => onKeyDown(event, index)}
          className={[
            'px-3 py-1 rounded-full text-sm border transition-colors',
            item.id === selected
              ? 'bg-primary text-primary-foreground border-primary'
              : 'border-border hover:bg-accent',
          ].join(' ')}
        >
          {item.label}
        </button>
      ))}
      {children}
    </div>
  );
}
