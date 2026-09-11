import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PluginProvider, usePlugins } from './PluginProvider';

/**
 * This provider wraps the entire application, so a bad value it puts into state
 * is not a plugin bug — it is a blank window.
 *
 * That is not hypothetical: the commands here resolve to `null` whenever the
 * backend does not implement them (an older build, a transport that answers
 * with nothing), and a `null` reaching `plugins` crashed a component at the app
 * root, which showed up as every unrelated page timing out with no sidebar.
 * These tests pin the shape coercion that fixed it.
 */

const mocks = vi.hoisted(() => ({
  enabled: vi.fn(),
  list: vi.fn(),
  contributions: vi.fn(),
  start: vi.fn(),
}));

vi.mock('./api', () => ({
  pluginsEnabled: mocks.enabled,
  listPlugins: mocks.list,
  listPluginContributions: mocks.contributions,
  startPlugins: mocks.start,
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('@/components/Toast', () => ({ showToast: vi.fn() }));

/** Renders the values a consumer at the app root would actually touch. */
function Probe() {
  const { enabled, ready, plugins, contributions, ofKind } = usePlugins();
  if (!ready) return <span>pending</span>;
  return (
    <span data-testid="probe">
      {`enabled=${enabled} plugins=${plugins.length} contributions=${contributions.length} themes=${ofKind('theme').length}`}
    </span>
  );
}

async function renderProvider() {
  render(
    <PluginProvider>
      <Probe />
    </PluginProvider>,
  );
  return waitFor(() => screen.getByTestId('probe'));
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.enabled.mockResolvedValue(false);
  mocks.list.mockResolvedValue([]);
  mocks.contributions.mockResolvedValue([]);
  mocks.start.mockResolvedValue([]);
});

describe('the provider survives a backend that does not answer properly', () => {
  it('treats a null plugin list as an empty one rather than putting null into state', async () => {
    mocks.list.mockResolvedValue(null);
    const probe = await renderProvider();
    expect(probe.textContent).toContain('plugins=0');
    expect(probe.textContent).toContain('contributions=0');
  });

  it('treats a null enabled answer as switched off', async () => {
    mocks.enabled.mockResolvedValue(null);
    const probe = await renderProvider();
    // Not merely falsy-by-accident: `enabled` gates whether any plugin script
    // is started at all, so it has to be a real boolean.
    expect(probe.textContent).toContain('enabled=false');
  });

  it('survives a null contribution list when the subsystem is on', async () => {
    mocks.enabled.mockResolvedValue(true);
    mocks.contributions.mockResolvedValue(null);
    const probe = await renderProvider();
    expect(probe.textContent).toContain('contributions=0');
    expect(probe.textContent).toContain('themes=0');
  });

  it('survives a null start result without losing the rest of the startup path', async () => {
    mocks.enabled.mockResolvedValue(true);
    mocks.start.mockResolvedValue(null);
    const probe = await renderProvider();
    expect(probe.textContent).toContain('enabled=true');
  });

  it('leaves usable empty state when every command rejects', async () => {
    mocks.enabled.mockRejectedValue(new Error('no such command'));
    mocks.list.mockRejectedValue(new Error('no such command'));
    const probe = await renderProvider();
    expect(probe.textContent).toContain('enabled=false');
    expect(probe.textContent).toContain('plugins=0');
  });

  it('reaches ready even when the backend is uncooperative, so the app can render', async () => {
    mocks.enabled.mockResolvedValue(null);
    mocks.list.mockResolvedValue(null);
    const probe = await renderProvider();
    expect(probe).toBeInTheDocument();
  });
});

describe('the ordinary path still works', () => {
  it('exposes the plugins and contributions the backend reported', async () => {
    mocks.enabled.mockResolvedValue(true);
    mocks.list.mockResolvedValue([{ id: 'acme.one' }, { id: 'acme.two' }]);
    mocks.contributions.mockResolvedValue([
      { id: 'acme.one/dusk', pluginId: 'acme.one', localId: 'dusk', kind: 'theme', title: 'Dusk' },
      { id: 'acme.two/page', pluginId: 'acme.two', localId: 'page', kind: 'page', title: 'Page' },
    ]);
    const probe = await renderProvider();
    expect(probe.textContent).toContain('enabled=true');
    expect(probe.textContent).toContain('plugins=2');
    expect(probe.textContent).toContain('contributions=2');
    expect(probe.textContent).toContain('themes=1');
  });

  it('does not start any plugin while the subsystem is switched off', async () => {
    mocks.enabled.mockResolvedValue(false);
    await renderProvider();
    expect(mocks.start).not.toHaveBeenCalled();
    // Management metadata is still readable, so Settings can list what is
    // installed without anything running.
    expect(mocks.list).toHaveBeenCalled();
  });
});
