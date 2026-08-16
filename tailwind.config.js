/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        primary: {
          DEFAULT: "var(--primary)",
          strong: "var(--primary-strong)",
          // `bg-primary/20` emits no rule (see the status tints below), so the
          // two tint steps the design system defines have to be reachable by
          // name.
          soft: "var(--primary-soft)",
          tint: "var(--primary-tint)",
        },
        // Solid tints for status surfaces. Tailwind 3 silently drops opacity
        // modifiers on `var()` colours (`bg-err/10` emits no rule), so a tint
        // has to be its own token — same reason --primary-soft exists.
        "ok-soft": "var(--ok-soft)",
        "warn-soft": "var(--warn-soft)",
        "err-soft": "var(--err-soft)",
        "info-soft": "var(--info-soft)",

        // === Redesign tokens (Forge system, OKLCH) ===
        // Surfaces
        base: "var(--bg)",
        "base-soft": "var(--bg-soft)",
        panel: {
          DEFAULT: "var(--panel)",
          2: "var(--panel-2)",
          hover: "var(--panel-hover)",
          active: "var(--panel-active)",
        },
        // Lines
        line: {
          DEFAULT: "var(--line)",
          soft: "var(--line-soft)",
          strong: "var(--line-strong)",
        },
        // Text (renamed to "ink" so utilities read clean: text-ink, text-ink-2)
        ink: {
          DEFAULT: "var(--text)",
          2: "var(--text-2)",
          3: "var(--text-3)",
          4: "var(--text-4)",
        },
        // Accent (v2: independent teal/cyan; in v1 it aliased --primary,
        // but no component currently uses bg-accent so the cutover is safe)
        accent: {
          DEFAULT: "var(--accent)",
          soft: "var(--accent-soft)",
        },
        "on-accent": "var(--on-accent)",
        // Foreground for anything filled with `error` — flips with the theme,
        // see the --on-err note in redesign-tokens-v2.css.
        "on-error": "var(--on-err)",
        // Asset type colors (11)
        "c-texture": "var(--c-texture)",
        "c-model": "var(--c-model)",
        "c-audio": "var(--c-audio)",
        "c-video": "var(--c-video)",
        "c-animation": "var(--c-animation)",
        "c-material": "var(--c-material)",
        "c-prefab": "var(--c-prefab)",
        "c-scene": "var(--c-scene)",
        "c-script": "var(--c-script)",
        "c-data": "var(--c-data)",
        "c-other": "var(--c-other)",
        // Git status (4)
        "git-new": "var(--git-new)",
        "git-modified": "var(--git-modified)",
        "git-deleted": "var(--git-deleted)",
        "git-renamed": "var(--git-renamed)",
        // Semantic status
        ok: "var(--ok)",
        warn: "var(--warn)",
        err: "var(--err)",
        info: "var(--info)",
      },
      borderRadius: {
        sm: "var(--radius-sm)",
        md: "var(--radius-md)",
        lg: "var(--radius-lg)",
        xl: "var(--radius-xl)",
      },
      boxShadow: {
        sm: "var(--shadow-sm)",
        md: "var(--shadow-md)",
        lg: "var(--shadow-lg)",
        // Redesign
        card: "var(--shadow-card)",
        pop: "var(--shadow-pop)",
      },
      fontFamily: {
        display: ['"Inter Tight"', "system-ui", "-apple-system", "sans-serif"],
        // Override default mono so font-mono → JetBrains Mono (used by metadata cells today,
        // matches Phase 1+ design CSS that requests "JetBrains Mono" directly).
        mono: ['"JetBrains Mono"', "ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
      },
    },
  },
  plugins: [],
};
