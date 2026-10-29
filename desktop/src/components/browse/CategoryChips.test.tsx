import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { CategoryChips } from './CategoryChips';

const CHIPS = Array.from({ length: 150 }, (_, i) => ({ id: `c${i}`, label: `Category ${i}` }));

function setup(selected: string | null = null) {
  const onSelect = vi.fn();
  render(
    <>
      <input aria-label="Search" />
      <CategoryChips chips={CHIPS} selected={selected} onSelect={onSelect} label="Categories" />
      <a href="#results">First result</a>
    </>,
  );
  return { onSelect, user: userEvent.setup() };
}

describe('CategoryChips', () => {
  it('costs one Tab stop for 150 chips', async () => {
    const { user } = setup();
    await user.click(screen.getByLabelText('Search'));
    await user.tab();
    expect(screen.getByRole('radio', { name: 'All' })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('link', { name: 'First result' })).toHaveFocus();
  });

  it('enters on the chosen chip', async () => {
    const { user } = setup('c7');
    await user.click(screen.getByLabelText('Search'));
    await user.tab();
    expect(screen.getByRole('radio', { name: 'Category 7' })).toHaveFocus();
  });

  it('moves with arrows without choosing, and chooses on Enter', async () => {
    const { user, onSelect } = setup();
    await user.click(screen.getByLabelText('Search'));
    await user.tab();
    await user.keyboard('{ArrowRight}{ArrowRight}');
    expect(screen.getByRole('radio', { name: 'Category 1' })).toHaveFocus();
    expect(onSelect).not.toHaveBeenCalled();
    await user.keyboard('{Enter}');
    expect(onSelect).toHaveBeenCalledWith('c1');
    await user.keyboard('{End}');
    expect(screen.getByRole('radio', { name: 'Category 149' })).toHaveFocus();
    await user.keyboard('{ArrowRight}');
    expect(screen.getByRole('radio', { name: 'All' })).toHaveFocus();
  });

  it('chooses All with null', async () => {
    const { user, onSelect } = setup('c3');
    await user.click(screen.getByRole('radio', { name: 'All' }));
    expect(onSelect).toHaveBeenCalledWith(null);
  });
});
