/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        // === Legacy tokens (kept until each component migrates in Phase 1) ===
        background: "var(--color-background)",
        "card-bg": "var(--color-card-bg)",
        "card-bg-hover": "var(--color-card-bg-hover)",
        border: "var(--color-border)",
        "border-subtle": "var(--color-border-subtle)",
        "text-primary": "var(--color-text-primary)",
        "text-secondary": "var(--color-text-secondary)",
        primary: {
          DEFAULT: "var(--color-primary)",
          hover: "var(--color-primary-hover)",
          glow: "var(--color-primary-glow)",
        },
        success: "var(--color-success)",
        warning: "var(--color-warning)",
        error: "var(--color-error)",
        info: "var(--color-info)",

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
        // Accent (Forge amber primary)
        accent: {
          DEFAULT: "var(--primary)",
          strong: "var(--primary-strong)",
          soft: "var(--primary-soft)",
          tint: "var(--primary-tint)",
        },
        "on-accent": "var(--on-primary)",
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
        // Semantic status (new tokens — existing success/warning/error/info above remain for legacy)
        ok: "var(--ok)",
        warn: "var(--warn)",
        err: "var(--err)",
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
        glow: "var(--shadow-glow)",
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
