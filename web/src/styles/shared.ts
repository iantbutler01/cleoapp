import { unsafeCSS, CSSResult } from 'lit';

// Import the compiled Tailwind CSS as a string
// Vite will process this through the Tailwind plugin
import styles from '../styles.css?inline';

// Create a CSSResult that Lit components can use with Shadow DOM
// Note: unsafeCSS is safe here because we're importing our own static CSS file,
// not user-generated content. The "unsafe" refers to bypassing XSS sanitization
// which is only a concern for untrusted input.
export const tailwindStyles = unsafeCSS(styles);

// For components that want to extend with additional styles
export function withTailwind(...additionalStyles: CSSResult[]): CSSResult[] {
  return [tailwindStyles, ...additionalStyles];
}
