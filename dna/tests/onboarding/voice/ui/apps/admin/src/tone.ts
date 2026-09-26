import type { Tone } from '@hale/components';

/** Voice's state words to the instrument's tones. Color is earned: healthy is quiet. */
export function toneOf(status: string): Tone {
  switch (status) {
    case 'starting':
    case 'pending':
    case 'paused':
    case 'draining':
    case 'incomplete':
    case 'connecting':
      return 'wait';
    case 'down':
    case 'refused':
    case 'failed':
    case 'abandoned':
    case 'revoked':
    case 'exhausted':
    case 'rate_limited':
    case 'budget_exhausted':
    case 'no_capacity':
    case 'deadline_exceeded':
      return 'fail';
    default:
      return 'quiet';
  }
}
