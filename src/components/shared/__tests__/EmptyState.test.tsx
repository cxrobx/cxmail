import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import EmptyState from '@/components/shared/EmptyState';
import { Inbox } from 'lucide-react';

describe('EmptyState', () => {
  it('renders the title', () => {
    render(<EmptyState icon={Inbox} title="No messages" />);
    expect(screen.getByText('No messages')).toBeInTheDocument();
  });

  it('renders the description when provided', () => {
    render(
      <EmptyState
        icon={Inbox}
        title="No messages"
        description="Your inbox is empty"
      />
    );
    expect(screen.getByText('Your inbox is empty')).toBeInTheDocument();
  });

  it('does not render description when not provided', () => {
    const { container } = render(
      <EmptyState icon={Inbox} title="No messages" />
    );
    // Only one <p> tag (the title), no description paragraph
    const paragraphs = container.querySelectorAll('p');
    expect(paragraphs).toHaveLength(1);
    expect(paragraphs[0].textContent).toBe('No messages');
  });

  it('renders the icon', () => {
    const { container } = render(
      <EmptyState icon={Inbox} title="No messages" />
    );
    // Lucide renders an SVG element
    const svg = container.querySelector('svg');
    expect(svg).toBeInTheDocument();
  });
});
