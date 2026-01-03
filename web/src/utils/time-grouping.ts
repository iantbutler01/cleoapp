import { ContentItem } from '../api';

export interface TimelineSection {
  label: string;
  date: string; // ISO date for sorting
  collapsed: boolean;
  items: ContentItem[];
}

function getCreatedAt(item: ContentItem): string {
  return item.type === 'tweet' ? item.created_at : item.thread.created_at;
}

function getItemId(item: ContentItem): number {
  return item.type === 'tweet' ? item.id : item.thread.id;
}

/**
 * Groups content items by time period (Today, Yesterday, This Week, Earlier)
 *
 * Note: created_at comes from the API as ISO 8601 UTC (e.g., "2025-12-17T10:30:00Z").
 * We group by the user's local day, so a tweet at "2025-12-17T02:00:00Z" (2 AM UTC)
 * would be "Today" for someone in UTC but "Yesterday" for someone in PST at 6 PM.
 */
export function groupContentByTime(items: ContentItem[]): TimelineSection[] {
  const now = new Date();
  const today = startOfLocalDay(now);
  const yesterday = new Date(today.getTime() - 24 * 60 * 60 * 1000);
  const weekAgo = new Date(today.getTime() - 7 * 24 * 60 * 60 * 1000);

  const sections: Map<string, TimelineSection> = new Map();

  for (const item of items) {
    // Parse UTC timestamp, convert to local for day comparison
    const itemDate = new Date(getCreatedAt(item));
    const itemLocalDay = startOfLocalDay(itemDate);

    let label: string;
    let dateKey: string;
    let collapsed: boolean;

    if (itemLocalDay.getTime() >= today.getTime()) {
      label = 'Today';
      dateKey = today.toISOString();
      collapsed = false;
    } else if (itemLocalDay.getTime() >= yesterday.getTime()) {
      label = 'Yesterday';
      dateKey = yesterday.toISOString();
      collapsed = false;
    } else if (itemLocalDay.getTime() >= weekAgo.getTime()) {
      label = 'This Week';
      dateKey = weekAgo.toISOString();
      collapsed = true;
    } else {
      label = 'Earlier';
      dateKey = '1970-01-01T00:00:00.000Z';
      collapsed = true;
    }

    let section = sections.get(dateKey);
    if (!section) {
      section = { label, date: dateKey, collapsed, items: [] };
      sections.set(dateKey, section);
    }
    section.items.push(item);
  }

  // Sort sections by date (newest first)
  const result = Array.from(sections.values()).sort(
    (a, b) => new Date(b.date).getTime() - new Date(a.date).getTime()
  );

  // Sort items within each section (newest first)
  for (const section of result) {
    section.items.sort(
      (a, b) => new Date(getCreatedAt(b)).getTime() - new Date(getCreatedAt(a)).getTime()
    );
  }

  return result;
}

export { getItemId, getCreatedAt };

/**
 * Returns midnight of the given date in local timezone.
 * Used for grouping by "user's day".
 */
function startOfLocalDay(date: Date): Date {
  const d = new Date(date);
  d.setHours(0, 0, 0, 0);
  return d;
}

/**
 * Format a date as relative time (e.g., "2 hours ago", "just now")
 */
export function formatRelativeTime(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / (1000 * 60));
  const diffHours = Math.floor(diffMs / (1000 * 60 * 60));
  const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));

  if (diffMins < 1) return 'just now';
  if (diffMins < 60) return `${diffMins}m ago`;
  if (diffHours < 24) return `${diffHours}h ago`;
  if (diffDays < 7) return `${diffDays}d ago`;

  return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

/**
 * Format time of day (e.g., "3:45 PM")
 */
export function formatTimeOfDay(dateStr: string): string {
  const date = new Date(dateStr);
  return date.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
}
