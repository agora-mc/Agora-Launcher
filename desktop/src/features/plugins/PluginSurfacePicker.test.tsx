import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PluginSurfacePicker } from './PluginSurfacePicker';

/**
 * The control that turns a plugin's *offer* into a choice.
 *
 * What is worth pinning here is not that radio buttons work, but that the
 * screen tells the truth: Agora's own view is a first-class option rather than
 * an afterthought, a choice that cannot currently render says so, and a
 * surface nobody has offered to render does not appear as a fake decision.
 */

const mocks = vi.hoisted(() => ({
  surfaces: vi.fn(),
  setSurface: vi.fn(),
  refresh: vi.fn(),
  toast: vi.fn(),
}));

vi.mock('./api', () => ({
  pluginSurfaces: mocks.surfaces,
  setPluginSurface: mocks.setSurface,
}));
vi.mock('./PluginProvider', () => ({
  usePlugins: () => ({ refresh: mocks.refresh, refreshToken: 0 }),
}));
vi.mock('@/components/Toast', () => ({ showToast: mocks.toast }));

function offer(overrides: Record<string, unknown> = {}) {
  return {
    id: 'acme.compact/home',
    pluginId: 'acme.compact',
    localId: 'home',
    pluginName: 'Compact home',
    title: 'Compact',
    description: 'One dense list instead of cards.',
    surface: 'home',
    export: 'home',
    ...overrides,
  };
}

function choice(overrides: Record<string, unknown> = {}) {
  return {
    surface: 'home',
    title: 'Home',
    offers: [offer()],
    selected: null,
    effective: null,
    fallbackReason: null,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.surfaces.mockResolvedValue([choice()]);
  mocks.setSurface.mockResolvedValue(undefined);
  mocks.refresh.mockResolvedValue(undefined);
});

describe('the picker presents the built-in as a real option', () => {
  it('lists Agora alongside each offer, selected by default', async () => {
    render(<PluginSurfacePicker />);
    await waitFor(() => expect(screen.getByText('Home')).toBeInTheDocument());

    const agora = screen.getByRole('radio', { name: /Agora/ });
    expect(agora).toBeChecked();
    expect(screen.getByRole('radio', { name: /Compact/ })).not.toBeChecked();
    expect(screen.getByText('One dense list instead of cards.')).toBeInTheDocument();
  });

  it('sends the contribution id when a plugin is chosen', async () => {
    render(<PluginSurfacePicker />);
    await waitFor(() => expect(screen.getByText('Home')).toBeInTheDocument());

    await userEvent.click(screen.getByRole('radio', { name: /Compact/ }));
    await waitFor(() =>
      expect(mocks.setSurface).toHaveBeenCalledWith('home', 'acme.compact/home'),
    );
    // The surface reads its choice through the provider, so the refresh is
    // what makes the change visible without navigating away and back.
    expect(mocks.refresh).toHaveBeenCalled();
  });

  it('sends null to go back to the built-in', async () => {
    mocks.surfaces.mockResolvedValue([
      choice({ selected: 'acme.compact/home', effective: offer() }),
    ]);
    render(<PluginSurfacePicker />);
    await waitFor(() => expect(screen.getByText('Home')).toBeInTheDocument());

    await userEvent.click(screen.getByRole('radio', { name: /Agora/ }));
    await waitFor(() => expect(mocks.setSurface).toHaveBeenCalledWith('home', null));
  });
});

describe('a choice that cannot render says so', () => {
  it('shows the reason core gave for falling back', async () => {
    mocks.surfaces.mockResolvedValue([
      choice({
        selected: 'acme.compact/home',
        effective: null,
        fallbackReason: '`Compact home` is not running, so Agora’s own view is being shown.',
      }),
    ]);
    render(<PluginSurfacePicker />);
    await waitFor(() =>
      expect(screen.getByText(/is not running/)).toBeInTheDocument(),
    );
    // And the choice is still shown as theirs, so it is clear what will come
    // back when they fix it.
    expect(screen.getByRole('radio', { name: /Compact/ })).toBeChecked();
  });

  it('shows no reason when the built-in was chosen deliberately', async () => {
    render(<PluginSurfacePicker />);
    await waitFor(() => expect(screen.getByText('Home')).toBeInTheDocument());
    expect(screen.queryByText(/being shown/)).not.toBeInTheDocument();
  });
});

describe('it does not invent decisions', () => {
  it('renders nothing when no plugin has offered to replace anything', async () => {
    mocks.surfaces.mockResolvedValue([choice({ offers: [] })]);
    const { container } = render(<PluginSurfacePicker />);
    await waitFor(() => expect(mocks.surfaces).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it('renders nothing when the backend does not implement the command', async () => {
    mocks.surfaces.mockResolvedValue(null);
    const { container } = render(<PluginSurfacePicker />);
    await waitFor(() => expect(mocks.surfaces).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it('survives the command rejecting', async () => {
    mocks.surfaces.mockRejectedValue(new Error('no such command'));
    const { container } = render(<PluginSurfacePicker />);
    await waitFor(() => expect(mocks.surfaces).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });
});
