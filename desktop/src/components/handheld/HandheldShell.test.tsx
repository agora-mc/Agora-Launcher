import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ReactElement } from 'react';
import type { InstanceRow } from '../../lib/tauri';
import type { GamepadIntent } from '../../lib/useGamepad';
import { ControllerProvider } from '../../features/controller/ControllerProvider';
import { HandheldShell } from './HandheldShell';

const gamepadHarness = vi.hoisted(() => ({
  connected: false,
  onIntent: undefined as ((intent: GamepadIntent) => void) | undefined,
  onConnectionChange: undefined as ((connected: boolean) => void) | undefined,
}));

// The shell no longer reads the pad itself: ControllerProvider owns the sampler
// and hands ownership to whichever layer is on top. Mocking at the sampler
// boundary keeps these tests exercising the real dispatch path.
vi.mock('../../lib/useGamepad', () => ({
  useGamepad: (options: {
    onIntent?: (intent: GamepadIntent) => void;
    onConnectionChange?: (connected: boolean) => void;
  }) => {
    gamepadHarness.onIntent = options.onIntent;
    gamepadHarness.onConnectionChange = options.onConnectionChange;
    return {
      connected: gamepadHarness.connected,
      gamepadCount: gamepadHarness.connected ? 1 : 0,
    };
  },
}));

vi.mock('../../lib/tauri', () => ({
  listInstances: vi.fn(),
}));

import { listInstances } from '../../lib/tauri';

const listInstancesMock = vi.mocked(listInstances);

beforeEach(() => {
  gamepadHarness.connected = false;
  gamepadHarness.onIntent = undefined;
  gamepadHarness.onConnectionChange = undefined;
  listInstancesMock.mockReset();
});

function withController(node: ReactElement) {
  return <ControllerProvider>{node}</ControllerProvider>;
}

function send(intent: GamepadIntent) {
  act(() => {
    gamepadHarness.onIntent?.(intent);
  });
}

function instance(id: string, name: string): InstanceRow {
  return {
    instance_id: id,
    name,
    minecraft_version: '1.21.8',
    loader: 'Fabric',
    loader_version: '0.16.14',
    is_modpack: false,
    is_locked: false,
    last_launched_at: null,
    jvm_memory_mb: 4096,
    jvm_memory_mode: 'manual',
    jvm_gc: 'g1gc',
    jvm_custom_args: '',
    jvm_always_pre_touch: false,
    created_at: '2026-01-01T00:00:00Z',
    java_path: null,
    java_incompatible_override: false,
    icon_path: null,
    launch_mode_override: 'auto',
    import_source: null,
  };
}

describe('HandheldShell', () => {
  it('shows a useful empty state and Escape exits without a controller', async () => {
    listInstancesMock.mockResolvedValue([]);
    const onActiveChange = vi.fn();

    render(withController(
      <HandheldShell active onActiveChange={onActiveChange} onLaunch={vi.fn(async () => true)} />,
    ));

    expect(await screen.findByText('No instances yet')).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'Escape' });

    expect(onActiveChange).toHaveBeenCalledWith(false);
  });

  it('moves between cards and launches the selected instance on accept', async () => {
    listInstancesMock.mockResolvedValue([instance('one', 'Overworld'), instance('two', 'Creative')]);
    const onLaunch = vi.fn(async () => true);

    render(withController(
      <HandheldShell active onActiveChange={vi.fn()} onLaunch={onLaunch} />,
    ));

    await screen.findByRole('button', { name: 'Launch Overworld' });
    send({ type: 'direction', direction: 'right' });
    send({ type: 'button', button: 'south' });

    expect(onLaunch).toHaveBeenCalledWith('two');
  });

  it('leaves handheld mode on cancel', async () => {
    listInstancesMock.mockResolvedValue([instance('one', 'Overworld')]);
    const onActiveChange = vi.fn();

    render(withController(
      <HandheldShell active onActiveChange={onActiveChange} onLaunch={vi.fn(async () => true)} />,
    ));

    await screen.findByRole('button', { name: 'Launch Overworld' });
    send({ type: 'button', button: 'east' });

    expect(onActiveChange).toHaveBeenCalledWith(false);
  });

  it('re-enters handheld mode from the menu button while closed', () => {
    listInstancesMock.mockResolvedValue([]);
    const onActiveChange = vi.fn();

    render(withController(
      <HandheldShell active={false} onActiveChange={onActiveChange} onLaunch={vi.fn(async () => true)} />,
    ));

    send({ type: 'button', button: 'start' });

    expect(onActiveChange).toHaveBeenCalledWith(true);
  });

  it('does not swallow other intents while handheld mode is closed', () => {
    listInstancesMock.mockResolvedValue([]);
    const onActiveChange = vi.fn();

    render(withController(
      <HandheldShell active={false} onActiveChange={onActiveChange} onLaunch={vi.fn(async () => true)} />,
    ));

    expect(() => {
      send({ type: 'direction', direction: 'down' });
      send({ type: 'button', button: 'south' });
      send({ type: 'button', button: 'east' });
    }).not.toThrow();
    expect(onActiveChange).not.toHaveBeenCalled();
  });

  it('keeps the shell usable when the controller disconnects', async () => {
    gamepadHarness.connected = true;
    listInstancesMock.mockResolvedValue([instance('one', 'Overworld')]);

    const shell = (
      <HandheldShell active onActiveChange={vi.fn()} onLaunch={vi.fn(async () => true)} />
    );
    const view = render(withController(shell));

    await waitFor(() => expect(screen.getByText('Controller connected')).toBeInTheDocument());
    act(() => gamepadHarness.onConnectionChange?.(false));
    gamepadHarness.connected = false;
    // The provider's returned state follows the next render after the
    // disconnect. The shell itself remains mounted and keyboard-exitable.
    view.rerender(withController(shell));

    expect(screen.getByText('Controller disconnected')).toBeInTheDocument();
  });
});
