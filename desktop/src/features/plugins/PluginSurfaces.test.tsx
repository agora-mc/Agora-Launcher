import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PluginPage } from './PluginSurfaces';
import { themeDeclarations } from './PluginTheme';
import { customViewDocument } from './PluginCustomView';

const mocks = vi.hoisted(() => ({ state: {} as Record<string, unknown>, render: vi.fn(), run: vi.fn() }));
vi.mock('./PluginProvider', () => ({ usePlugins: () => mocks.state }));
vi.mock('./api', () => ({ renderPluginView: mocks.render, runPluginCommand: mocks.run }));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.state = { ready: true, refreshToken: 0,
    ofKind: () => [{ id: 'acme.panel/overview', pluginId: 'acme.panel', localId: 'overview', title: 'Overview' }],
    plugins: [{ id: 'acme.panel', definitions: { pages: [{ id: 'overview', title: 'Overview', view: { kind: 'host', export: 'differentExport' } }] } }] };
  mocks.render.mockResolvedValue({ title: 'Working dashboard', blocks: [{ type: 'actions', items: [{ id: 'remember-count', label: 'Remember', export: 'remember' }] }] });
  mocks.run.mockResolvedValue({});
});

describe('plugin destinations', () => {
  it('uses the declared export and re-renders after a behavioral action', async () => {
    render(<PluginPage contributionId="acme.panel/overview" onGoHome={() => {}} />);
    await screen.findByText('Working dashboard');
    expect(mocks.render).toHaveBeenCalledWith('acme.panel', 'differentExport', null);
    fireEvent.click(screen.getByRole('button', { name: 'Remember' }));
    await waitFor(() => expect(mocks.run).toHaveBeenCalledWith('acme.panel', 'remember', null));
    await waitFor(() => expect(mocks.render).toHaveBeenCalledTimes(2));
  });
  it('removes a disabled view and leaves a working home action', async () => {
    const home = vi.fn();
    const view = render(<PluginPage contributionId="acme.panel/overview" onGoHome={home} />);
    await screen.findByText('Working dashboard');
    mocks.state = { ...mocks.state, ofKind: () => [] };
    view.rerender(<PluginPage contributionId="acme.panel/overview" onGoHome={home} />);
    expect(screen.queryByText('Working dashboard')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Go home' }));
    expect(home).toHaveBeenCalledOnce();
  });
});

it('theme values cannot add CSS rules or remote resources', () => {
  expect(themeDeclarations({ primary: '#ffffff', border: 'url(https://example.com)', arbitrary: '#000000' }))
    .toBe('--primary:0.00 0.00% 100.00% !important;');
});

it('places restrictive custom-document policy before all plugin markup', () => {
  const document = customViewDocument('<script>parent.postMessage({}, "*")</script>');
  expect(document.indexOf('Content-Security-Policy')).toBeLessThan(document.indexOf('<script>'));
  expect(document).toContain("connect-src 'none'");
  expect(document).toContain("form-action 'none'");
});
