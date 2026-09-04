import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ControlifyOffer } from '../../lib/tauri';
import type { GamepadIntent } from '../../lib/useGamepad';
import { ControllerProvider } from '../../features/controller/ControllerProvider';
import { ControlifyOfferDialog } from './ControlifyOfferDialog';

const harness = vi.hoisted(() => ({
  onIntent: undefined as ((intent: GamepadIntent) => void) | undefined,
}));

vi.mock('../../lib/useGamepad', () => ({
  useGamepad: (options: { onIntent?: (intent: GamepadIntent) => void }) => {
    harness.onIntent = options.onIntent;
    return { connected: true, gamepadCount: 1 };
  },
}));

beforeEach(() => {
  harness.onIntent = undefined;
});

function press(button: string) {
  act(() => {
    harness.onIntent?.({ type: 'button', button } as GamepadIntent);
  });
}

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

/**
 * The offer exists to notice the user is holding a controller. Requiring a
 * mouse to answer it defeated the entire feature, so these are the tests that
 * must never regress.
 */
describe('ControlifyOfferDialog, driven by a controller', () => {
  it('accepts the offer without a mouse', () => {
    const onAccept = vi.fn();
    render(
      <ControllerProvider>
        <ControlifyOfferDialog offer={offer()} onAccept={onAccept} onDecline={vi.fn()} />
      </ControllerProvider>,
    );

    press('south');

    expect(onAccept).toHaveBeenCalledWith('controlify');
  });

  it('declines the offer without a mouse', () => {
    const onDecline = vi.fn();
    render(
      <ControllerProvider>
        <ControlifyOfferDialog offer={offer()} onAccept={vi.fn()} onDecline={onDecline} />
      </ControllerProvider>,
    );

    press('east');

    expect(onDecline).toHaveBeenCalledTimes(1);
  });

  it('never accepts an offer that carries no slug', () => {
    const onAccept = vi.fn();
    render(
      <ControllerProvider>
        <ControlifyOfferDialog
          offer={offer({ modrinth_slug: null })}
          onAccept={onAccept}
          onDecline={vi.fn()}
        />
      </ControllerProvider>,
    );

    press('south');

    expect(onAccept).not.toHaveBeenCalled();
  });

  it('never accepts a decision that is not an offer', () => {
    const onAccept = vi.fn();
    render(
      <ControllerProvider>
        <ControlifyOfferDialog
          offer={offer({ decision: 'already_installed', modrinth_slug: 'controlify' })}
          onAccept={onAccept}
          onDecline={vi.fn()}
        />
      </ControllerProvider>,
    );

    press('south');

    expect(onAccept).not.toHaveBeenCalled();
  });
});
