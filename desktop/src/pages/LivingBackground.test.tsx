import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';

const ambience = {
  soundOn: false, setSoundOn: vi.fn(), soundVolume: 0.5, setSoundVolume: vi.fn(),
  musicVolume: 0.35, setMusicVolume: vi.fn(), musicOn: true, setMusicOn: vi.fn(),
  clearBackground: true, setClearBackground: vi.fn(),
  setTod: vi.fn(), setWeather: vi.fn(), setTodLocked: vi.fn(), setWeatherLocked: vi.fn(),
  readClock: () => ({ tod: 0.5, weather: 'rain', todLocked: true, weatherLocked: true }),
  readView: () => ({ zoom: 1.6, tx: 0, ty: 0 }),
  setZoom: vi.fn(), setRainbow: vi.fn(), isRainbowPinned: () => false,
  setTrack: vi.fn(), setInstrument: vi.fn(), currentTrackId: () => null,
  setMusicAuto: vi.fn(), shuffleMusic: vi.fn(), setBuddy: vi.fn(), ready: true,
};
vi.mock('../features/ambience/AmbienceProvider', () => ({ useAmbience: () => ambience }));

import { LivingBackground } from './LivingBackground';

describe('LivingBackground reset scene', () => {
  it('shows the zoom readout and resets zoom, locks, visibility and companion', () => {
    render(<LivingBackground />);
    expect(screen.getByTestId('living-bg-zoom-readout').textContent).toBe('1.6×');
    fireEvent.click(screen.getByLabelText('Cursor companion')); // companion off
    expect(ambience.setBuddy).toHaveBeenLastCalledWith(false);

    fireEvent.click(screen.getByTestId('living-bg-reset'));

    expect(screen.getByTestId('living-bg-zoom-readout').textContent).toBe('0.9×');
    expect(ambience.setZoom).toHaveBeenLastCalledWith(0.9);
    expect(ambience.setTodLocked).toHaveBeenLastCalledWith(false);
    expect(ambience.setWeatherLocked).toHaveBeenLastCalledWith(false);
    expect(ambience.setClearBackground).toHaveBeenLastCalledWith(false);
    expect(ambience.setBuddy).toHaveBeenLastCalledWith(true);
    expect((screen.getByLabelText('Cursor companion') as HTMLInputElement).checked).toBe(true);
  });
});
