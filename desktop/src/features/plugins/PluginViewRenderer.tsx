import { useCallback, useState } from 'react';
import { AlertTriangle, Check, Info, Minus, ShieldAlert } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import type { ActionButton, Cell, Stat, Tone, ViewBlock, ViewModel } from './types';

/**
 * Renders a plugin's view using Agora's own components.
 *
 * The security property is structural rather than defensive: a plugin never
 * produces markup, so there is nothing here for `dangerouslySetInnerHTML` to
 * receive and no sanitiser to get wrong. A plugin describes *what* it wants
 * shown — a stat, a table, a warning — and this file decides what that looks
 * like, which is also why plugin views are themed, accessible and keyboard
 * navigable without any plugin author doing anything about it.
 *
 * A plugin picks a `Tone`, never a colour, so it cannot produce unreadable
 * contrast or ignore the user's theme.
 */

const TONE_TEXT: Record<Tone, string> = {
  neutral: 'text-foreground',
  info: 'text-sky-600 dark:text-sky-400',
  success: 'text-emerald-600 dark:text-emerald-400',
  warning: 'text-amber-600 dark:text-amber-400',
  danger: 'text-destructive',
};

const TONE_SURFACE: Record<Tone, string> = {
  neutral: 'border-border bg-muted/40',
  info: 'border-sky-500/40 bg-sky-500/10',
  success: 'border-emerald-500/40 bg-emerald-500/10',
  warning: 'border-amber-500/40 bg-amber-500/10',
  danger: 'border-destructive/40 bg-destructive/10',
};

const TONE_BADGE: Record<Tone, 'default' | 'secondary' | 'destructive' | 'outline'> = {
  neutral: 'secondary',
  info: 'outline',
  success: 'default',
  warning: 'outline',
  danger: 'destructive',
};

function ToneIcon({ tone }: { tone: Tone }) {
  const className = cn('h-4 w-4 shrink-0', TONE_TEXT[tone]);
  if (tone === 'danger') return <ShieldAlert className={className} aria-hidden />;
  if (tone === 'warning') return <AlertTriangle className={className} aria-hidden />;
  if (tone === 'success') return <Check className={className} aria-hidden />;
  return <Info className={className} aria-hidden />;
}

function StatTile({ stat }: { stat: Stat }) {
  return (
    <div className="rounded-lg border border-border bg-card px-4 py-3">
      <div className="text-xs uppercase tracking-wide text-muted-foreground">{stat.label}</div>
      <div className={cn('mt-1 text-2xl font-semibold tabular-nums', TONE_TEXT[stat.tone ?? 'neutral'])}>
        {stat.value}
      </div>
      {stat.hint ? <div className="mt-1 text-xs text-muted-foreground">{stat.hint}</div> : null}
    </div>
  );
}

function CellView({ cell }: { cell: Cell }) {
  if (cell.type === 'badge') {
    return <Badge variant={TONE_BADGE[cell.tone ?? 'neutral']}>{cell.text}</Badge>;
  }
  if (cell.type === 'flag') {
    return cell.value ? (
      <Check className="h-4 w-4 text-emerald-600 dark:text-emerald-400" aria-label="Yes" />
    ) : (
      <Minus className="h-4 w-4 text-muted-foreground" aria-label="No" />
    );
  }
  return <span>{cell.text}</span>;
}

export interface PluginViewRendererProps {
  model: ViewModel;
  /** Invoked when the user presses an action. The caller routes it back into
   *  the owning plugin; this component never invokes anything itself. */
  onAction: (action: ActionButton) => void | Promise<void>;
  /** Disables every action, e.g. while one is already running. */
  busy?: boolean;
}

export function PluginViewRenderer({ model, onAction, busy = false }: PluginViewRendererProps) {
  const [pending, setPending] = useState<string | null>(null);

  const press = useCallback(
    async (action: ActionButton) => {
      // A plugin may ask for confirmation; the host asks it, so the wording
      // cannot be styled to look like something else.
      if (action.confirm && !window.confirm(action.confirm)) return;
      setPending(action.id);
      try {
        await onAction(action);
      } finally {
        setPending(null);
      }
    },
    [onAction],
  );

  return (
    <div className="space-y-4">
      {model.title ? (
        <div>
          <h2 className="text-xl font-semibold">{model.title}</h2>
          {model.subtitle ? (
            <p className="text-sm text-muted-foreground">{model.subtitle}</p>
          ) : null}
        </div>
      ) : null}

      {model.blocks.map((block, index) => (
        <BlockView
          key={index}
          block={block}
          onPress={press}
          busy={busy}
          pending={pending}
        />
      ))}
    </div>
  );
}

function BlockView({
  block,
  onPress,
  busy,
  pending,
}: {
  block: ViewBlock;
  onPress: (action: ActionButton) => void;
  busy: boolean;
  pending: string | null;
}) {
  switch (block.type) {
    case 'heading':
      return <h3 className="text-base font-semibold">{block.text}</h3>;

    case 'text':
      return (
        <p className={cn('text-sm leading-relaxed', TONE_TEXT[block.tone ?? 'neutral'])}>
          {block.text}
        </p>
      );

    case 'stats':
      return (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          {block.items.map((stat, index) => (
            <StatTile key={index} stat={stat} />
          ))}
        </div>
      );

    case 'table':
      if (block.rows.length === 0) {
        return (
          <p className="rounded-lg border border-dashed border-border px-4 py-6 text-center text-sm text-muted-foreground">
            {block.emptyMessage ?? 'Nothing to show.'}
          </p>
        );
      }
      return (
        <div className="overflow-x-auto rounded-lg border border-border">
          <table className="w-full text-sm">
            <thead className="bg-muted/50 text-left">
              <tr>
                {block.columns.map((column, index) => (
                  <th
                    key={index}
                    scope="col"
                    className={cn(
                      'px-3 py-2 font-medium',
                      column.align === 'end' && 'text-right',
                    )}
                  >
                    {column.label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((row, rowIndex) => (
                <tr key={rowIndex} className="border-t border-border">
                  {row.map((cell, cellIndex) => (
                    <td
                      key={cellIndex}
                      className={cn(
                        'px-3 py-2',
                        block.columns[cellIndex]?.align === 'end' && 'text-right',
                      )}
                    >
                      <CellView cell={cell} />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );

    case 'list':
      return (
        <ul className="space-y-2">
          {block.items.map((item, index) => (
            <li
              key={index}
              className="rounded-lg border border-border bg-card px-3 py-2 text-sm"
            >
              <div className={cn('font-medium', TONE_TEXT[item.tone ?? 'neutral'])}>
                {item.title}
              </div>
              {item.detail ? (
                <div className="text-xs text-muted-foreground">{item.detail}</div>
              ) : null}
            </li>
          ))}
        </ul>
      );

    case 'status':
      return (
        <div
          className={cn(
            'flex items-start gap-3 rounded-lg border px-4 py-3',
            TONE_SURFACE[block.tone],
          )}
          role={block.tone === 'danger' || block.tone === 'warning' ? 'alert' : 'status'}
        >
          <ToneIcon tone={block.tone} />
          <div className="min-w-0">
            <div className="text-sm font-medium">{block.title}</div>
            {block.message ? (
              <div className="text-sm text-muted-foreground">{block.message}</div>
            ) : null}
          </div>
        </div>
      );

    case 'actions':
      return (
        <div className="flex flex-wrap gap-2">
          {block.items.map((action) => (
            <Button
              key={action.id}
              variant={action.tone === 'danger' ? 'destructive' : 'secondary'}
              size="sm"
              disabled={busy || pending !== null}
              onClick={() => onPress(action)}
            >
              {pending === action.id ? 'Working…' : action.label}
            </Button>
          ))}
        </div>
      );

    case 'divider':
      return <hr className="border-border" />;

    default:
      // A view block this build does not understand: say so rather than
      // rendering nothing, so a plugin written for a newer Agora produces a
      // visible, explicable gap instead of a mysteriously empty panel.
      return (
        <p className="text-xs text-muted-foreground">
          This plugin used a view element this version of Agora does not know how to draw.
        </p>
      );
  }
}
