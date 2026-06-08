import type { UserInfo } from './api';

export function isLoggedIn(): boolean {
  if (typeof localStorage === 'undefined') return false;
  return !!localStorage.getItem('cleanclaw_token');
}

export async function getCurrentUser(): Promise<UserInfo | null> {
  if (!isLoggedIn()) return null;
  try {
    const r = await fetch('/api/me', { credentials: 'same-origin' });
    if (!r.ok) return null;
    const j = await r.json();
    return j.user ?? null;
  } catch {
    return null;
  }
}
