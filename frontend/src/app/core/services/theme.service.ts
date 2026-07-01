import { Injectable, computed, signal } from '@angular/core';

export type Theme = 'dark' | 'light' | 'retro';

export interface ThemeOption {
  id: Theme;
  label: string;
  /** Inline SVG-free glyph shown in the switcher. */
  icon: string;
}

const STORAGE_KEY = 'hermes-theme';

/**
 * App-wide theme control. Themes are pure CSS-variable palette swaps (see styles.css):
 * this service just persists the choice and reflects it as `data-theme` on <html>, which
 * flips every `zinc-*` colour utility at once. The initial attribute is set by an inline
 * script in index.html (pre-paint) so a reload never flashes the wrong palette.
 */
@Injectable({ providedIn: 'root' })
export class ThemeService {
  readonly options: ThemeOption[] = [
    { id: 'dark', label: 'Dark', icon: '🌙' },
    { id: 'light', label: 'Light', icon: '☀️' },
    { id: 'retro', label: 'Retro', icon: '📺' },
  ];

  readonly theme = signal<Theme>(this.read());

  /** The currently selected option (icon + label), for the switcher trigger. */
  readonly current = computed(() => this.options.find((o) => o.id === this.theme()) ?? this.options[0]);

  constructor() {
    // Reassert on boot in case the pre-paint script and stored value ever diverge.
    this.apply(this.theme());
  }

  setTheme(theme: Theme): void {
    this.theme.set(theme);
    this.apply(theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch {
      /* storage unavailable (private mode) — theme still applies for this session */
    }
  }

  /** Cycle dark → light → retro → dark, for a single-button toggle. */
  cycle(): void {
    const order = this.options.map((o) => o.id);
    const next = order[(order.indexOf(this.theme()) + 1) % order.length];
    this.setTheme(next);
  }

  private apply(theme: Theme): void {
    document.documentElement.setAttribute('data-theme', theme);
  }

  private read(): Theme {
    try {
      const v = localStorage.getItem(STORAGE_KEY);
      if (v === 'light' || v === 'retro' || v === 'dark') return v;
    } catch {
      /* ignore */
    }
    return 'dark';
  }
}
