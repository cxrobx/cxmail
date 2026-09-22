import { describe, it, expect } from 'vitest';
import { normalizeLinkHref } from '@/lib/composeLink';

describe('normalizeLinkHref', () => {
  it('keeps an explicit safe scheme', () => {
    expect(normalizeLinkHref('https://cxventures.io/a?b=1')).toBe('https://cxventures.io/a?b=1');
    expect(normalizeLinkHref('http://x.com')).toBe('http://x.com');
    expect(normalizeLinkHref('mailto:a@b.com')).toBe('mailto:a@b.com');
    expect(normalizeLinkHref('tel:+16155551212')).toBe('tel:+16155551212');
  });

  it('adds https to a bare host, including one with a port', () => {
    expect(normalizeLinkHref('  cxventures.io/proposals ')).toBe('https://cxventures.io/proposals');
    expect(normalizeLinkHref('localhost:3000/x')).toBe('https://localhost:3000/x');
  });

  it('turns a bare address into mailto', () => {
    expect(normalizeLinkHref('dana@northwind.example')).toBe('mailto:dana@northwind.example');
  });

  it('refuses unsafe schemes and junk', () => {
    expect(normalizeLinkHref('javascript:alert(1)')).toBeNull();
    expect(normalizeLinkHref('data:text/html,hi')).toBeNull();
    expect(normalizeLinkHref('')).toBeNull();
    expect(normalizeLinkHref('two words')).toBeNull();
  });
});
