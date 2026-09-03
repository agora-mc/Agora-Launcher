import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { ControlifyOffer } from '../../lib/tauri';
import { ControlifyOfferDialog } from './ControlifyOfferDialog';

function offer(overrides: Partial<ControlifyOffer> = {}): ControlifyOffer {
  return {
    instance_id: 'instance-1',
    decision: 'offer',
    modrinth_slug: 'controlify',
    reason: 'Minecraft has no built-in controller support. Controlify adds it.',
    ...overrides,
  };
}

describe('ControlifyOfferDialog', () => {
  it('passes the backend-provided slug to the accept callback', () => {
    const onAccept = vi.fn();
    render(
      <ControlifyOfferDialog
        offer={offer({ modrinth_slug: 'controlify-custom-slug' })}
        onAccept={onAccept}
        onDecline={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Install Controlify' }));

    expect(onAccept).toHaveBeenCalledWith('controlify-custom-slug');
  });

  it('does not render an install action for a non-offer decision', () => {
    const onAccept = vi.fn();
    render(
      <ControlifyOfferDialog
        offer={offer({ decision: 'already_installed', modrinth_slug: null })}
        onAccept={onAccept}
        onDecline={vi.fn()}
      />,
    );

    expect(screen.queryByRole('button', { name: 'Install Controlify' })).not.toBeInTheDocument();
    expect(screen.getByText('Controlify is not available to install for this decision.')).toBeInTheDocument();
    expect(onAccept).not.toHaveBeenCalled();
  });

  it('does not render an install action when an offer has no slug', () => {
    render(
      <ControlifyOfferDialog
        offer={offer({ modrinth_slug: null })}
        onAccept={vi.fn()}
        onDecline={vi.fn()}
      />,
    );

    expect(screen.queryByRole('button', { name: 'Install Controlify' })).not.toBeInTheDocument();
  });
});
