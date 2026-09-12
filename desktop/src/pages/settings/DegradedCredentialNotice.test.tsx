import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { DegradedCredentialNotice } from './DegradedCredentialNotice';

describe('DegradedCredentialNotice', () => {
  it('warns when credentials are in the encrypted-file fallback', () => {
    render(<DegradedCredentialNotice backend="encrypted-file" />);

    const notice = screen.getByTestId('degraded-credential-storage');
    expect(notice).toBeTruthy();
    expect(notice.textContent).toContain('Credential store unavailable');
    expect(notice.textContent).toContain('less secure than OS keychain storage');
  });

  it('says nothing when the OS keyring is holding the credential', () => {
    render(<DegradedCredentialNotice backend="keyring" />);
    expect(screen.queryByTestId('degraded-credential-storage')).toBeNull();
  });

  // DPAPI holds the key, so "anyone who can read that folder can read your
  // sign-in" would be false here. The keyring is not in use, but the protection
  // is the same one Credential Manager itself relies on.
  it('says nothing when the OS protects the file itself', () => {
    render(<DegradedCredentialNotice backend="os-protected-file" />);
    expect(screen.queryByTestId('degraded-credential-storage')).toBeNull();
  });

  it('says nothing when no credential is stored', () => {
    render(<DegradedCredentialNotice backend="none" />);
    expect(screen.queryByTestId('degraded-credential-storage')).toBeNull();
  });

  // A failed or unmocked lookup must not invent a security warning: telling
  // users their credentials are less protected than they are is its own harm,
  // and it trains them to ignore the warning that matters.
  it('says nothing while the backend is still unknown', () => {
    render(<DegradedCredentialNotice backend={undefined} />);
    expect(screen.queryByTestId('degraded-credential-storage')).toBeNull();
  });

  // The old spec wording claimed a machine-bound key. It is not machine-bound,
  // and the warning must not tell users it is. Nor may it promise that only
  // their own account can read the file: owner-only permissions are set on Unix,
  // but elsewhere this inherits the data directory's, and that directory can be
  // a removable drive with no per-user permissions at all.
  it('does not overstate the protection', () => {
    render(<DegradedCredentialNotice backend="encrypted-file" />);
    const text = (screen.getByTestId('degraded-credential-storage').textContent ?? '').toLowerCase();
    expect(text).not.toContain('machine-bound');
    expect(text).not.toContain('only your user account');
  });
});
