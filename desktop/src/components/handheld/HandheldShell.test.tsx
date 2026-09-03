import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { InstanceRow } from '../../lib/tauri';
import type { GamepadIntent } from '../../lib/useGamepad';
import { HandheldShell } from './HandheldShell';

const gamepadHarness = vi.hoisted(() => ({
  connected: false,
  onIntent: undefined as ((intent: GamepadIntent) => void) | undefined,
  onConnectionChange: undefined as ((connected: boolean) => void) | undefined,
}));

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

    render(
      <HandheldShell
        active
        onActiveChange={onActiveChange}
        onGamepadConnectionChange={vi.fn()}
        onLaunch={vi.fn(async () => true)}
      />,
    );

    expect(await screen.findByText('No instances yet')).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'Escape' });

    expect(onActiveChange).toHaveBeenCalledWith(false);
  });

  it('moves between cards and launches the selected instance with A', async () => {
    listInstancesMock.mockResolvedValue([instance('one', 'Overworld'), instance('two', 'Creative')]);
    const onLaunch = vi.fn(async () => true);

    render(
      <HandheldShell
        active
        onActiveChange={vi.fn()}
        onGamepadConnectionChange={vi.fn()}
        onLaunch={onLaunch}
      />,
    );

    await screen.findByRole('button', { name: 'Launch Overworld' });
    act(() => gamepadHarness.onIntent?.({ type: 'direction', direction: 'right' }));
    act(() => gamepadHarness.onIntent?.({ type: 'button', button: 'a' }));

    expect(onLaunch).toHaveBeenCalledWith('two');
  });

  it('keeps the shell usable when the controller disconnects', async () => {
    gamepadHarness.connected = true;
    listInstancesMock.mockResolvedValue([instance('one', 'Overworld')]);
    const onGamepadConnectionChange = vi.fn();

    const view = render(
      <HandheldShell
        active
        onActiveChange={vi.fn()}
        onGamepadConnectionChange={onGamepadConnectionChange}
        onLaunch={vi.fn(async () => true)}
      />,
    );

    await waitFor(() => expect(screen.getByText('Controller connected')).toBeInTheDocument());
    act(() => gamepadHarness.onConnectionChange?.(false));
    gamepadHarness.connected = false;
    // The hook's returned state follows the next render after the disconnect.
    // The shell itself remains mounted and therefore remains keyboard-exitable.
    view.rerender(
      <HandheldShell
        active
        onActiveChange={vi.fn()}
        onGamepadConnectionChange={onGamepadConnectionChange}
        onLaunch={vi.fn(async () => true)}
      />,
    );

    expect(onGamepadConnectionChange).toHaveBeenCalledWith(false);
    expect(screen.getByText('Controller disconnected')).toBeInTheDocument();
  });
});
