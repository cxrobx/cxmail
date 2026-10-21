import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import * as Tooltip from '@radix-ui/react-tooltip';
import RecipientLines from '@/components/mail/RecipientLines';

const SAME_NAME = [
  { name: 'Glendora Robinson', email: 'robinsonglendora@yahoo.example' },
  { name: 'Glendora Robinson', email: 'gfree75@gmail.example' },
];

const renderLines = (props: Parameters<typeof RecipientLines>[0]) =>
  render(
    <Tooltip.Provider>
      <RecipientLines {...props} />
    </Tooltip.Provider>,
  );

describe('RecipientLines', () => {
  it('shows names only until expanded, and each name carries its own address', () => {
    renderLines({ to: SAME_NAME });
    expect(screen.queryByText(/robinsonglendora@/)).toBeNull();
    const names = screen.getAllByText('Glendora Robinson');
    expect(names.map((n) => n.getAttribute('data-address'))).toEqual([
      'robinsonglendora@yahoo.example',
      'gfree75@gmail.example',
    ]);
  });

  it('clicking a name reveals every address, To and Cc alike; the chevron hides them again', () => {
    renderLines({ to: SAME_NAME, cc: [{ name: 'Dana', email: 'dana@acme.example' }] });
    fireEvent.click(screen.getAllByText('Glendora Robinson')[0]);
    expect(screen.getByText('<robinsonglendora@yahoo.example>')).toBeTruthy();
    expect(screen.getByText('<gfree75@gmail.example>')).toBeTruthy();
    expect(screen.getByText('<dana@acme.example>')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Hide addresses' }));
    expect(screen.queryByText('<dana@acme.example>')).toBeNull();
  });

  it('offers no toggle when every recipient is already shown as an address', () => {
    renderLines({ to: [{ name: null, email: 'sam@globex.example' }] });
    expect(screen.getByText('sam@globex.example')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Show addresses' })).toBeNull();
  });
});
