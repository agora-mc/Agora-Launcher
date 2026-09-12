import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PluginSurface } from './PluginSurface';

/**
 * This component decides what the first screen of the launcher is, so every
 * way it can go wrong ends with the user staring at the app.
 *
 * The rule under test is the one the whole replacement design rests on: a
 * plugin offering to render a surface does not get it, and anything uncertain
 * resolves to Agora's own view rather than to nothing. A blank home screen
 * reads as a broken install, which is a far worse outcome than a plugin's
 * view not appearing.
 */

const mocks = vi.hoisted(() => ({
  surfaces: vi.fn(),
  plugins: vi.fn(),
}));

vi.mock('./api', () => ({ pluginSurfaces: mocks.surfaces }));
vi.mock('./PluginProvider', () => ({ usePlugins: mocks.plugins }));
vi.mock('./PluginView', () => ({
  PluginView: ({ pluginId, exportName }: { pluginId: string; exportName: string }) => (
    <div data-testid="plugin-view">{`${pluginId}:${exportName}`}</div>
  ),
}));

function offer(overrides: Record<string, unknown> = {}) {
  return {
    id: 'acme.compact/home',
    pluginId: 'acme.compact',
    localId: 'home',
    pluginName: 'Compact home',
    title: 'Compact',
    description: null,
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

async function renderSurface() {
  render(<PluginSurface surface="home" fallback={<div>built-in home</div>} />);
  // The built-in renders synchronously; a plugin view only after the fetch.
  await waitFor(() => expect(mocks.surfaces).toHaveBeenCalledTimes(1));
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.plugins.mockReturnValue({ enabled: true, ready: true, refreshToken: 0 });
  mocks.surfaces.mockResolvedValue([choice()]);
});

describe('a plugin does not get a surface by offering to render it', () => {
  it('renders the built-in when an offer exists but nothing was chosen', async () => {
    await renderSurface();
    expect(screen.getByText('built-in home')).toBeInTheDocument();
    expect(screen.queryByTestId('plugin-view')).not.toBeInTheDocument();
  });

  it('renders the plugin only once core reports it as effective', async () => {
    mocks.surfaces.mockResolvedValue([
      choice({ selected: 'acme.compact/home', effective: offer() }),
    ]);
    await renderSurface();
    await waitFor(() =>
      expect(screen.getByTestId('plugin-view')).toHaveTextContent('acme.compact:home'),
    );
    expect(screen.queryByText('built-in home')).not.toBeInTheDocument();
  });

  it('renders the built-in when a selection exists but core fell back', async () => {
    // The shape core produces for a chosen-but-disabled plugin: a selection is
    // recorded, and `effective` is deliberately null.
    mocks.surfaces.mockResolvedValue([
      choice({
        selected: 'acme.compact/home',
        effective: null,
        fallbackReason: '`Compact home` is not running, so Agora’s own view is being shown.',
      }),
    ]);
    await renderSurface();
    expect(screen.getByText('built-in home')).toBeInTheDocument();
  });
});

describe('every uncertain answer falls through to the built-in', () => {
  it('survives a null answer from a backend that does not implement the command', async () => {
    mocks.surfaces.mockResolvedValue(null);
    await renderSurface();
    expect(screen.getByText('built-in home')).toBeInTheDocument();
  });

  it('survives the command rejecting outright', async () => {
    mocks.surfaces.mockRejectedValue(new Error('no such command'));
    await renderSurface();
    expect(screen.getByText('built-in home')).toBeInTheDocument();
  });

  it('ignores an effective entry that names no plugin to call', async () => {
    mocks.surfaces.mockResolvedValue([
      choice({ effective: offer({ pluginId: '', export: '' }) }),
    ]);
    await renderSurface();
    expect(screen.getByText('built-in home')).toBeInTheDocument();
  });

  it('ignores an answer about some other surface', async () => {
    mocks.surfaces.mockResolvedValue([
      { ...choice({ effective: offer() }), surface: 'something-else' },
    ]);
    await renderSurface();
    expect(screen.getByText('built-in home')).toBeInTheDocument();
  });

  it('does not ask at all while the plugin system is switched off', async () => {
    mocks.plugins.mockReturnValue({ enabled: false, ready: true, refreshToken: 0 });
    render(<PluginSurface surface="home" fallback={<div>built-in home</div>} />);
    expect(screen.getByText('built-in home')).toBeInTheDocument();
    expect(mocks.surfaces).not.toHaveBeenCalled();
  });
});
