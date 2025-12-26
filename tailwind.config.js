/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        // Dark theme colors from design spec
        background: "#1a1a2e",
        "card-bg": "#252542",
        border: "#3a3a5c",
        "text-primary": "#e4e4e7",
        "text-secondary": "#a1a1aa",
        primary: "#6366f1",
        success: "#22c55e",
        warning: "#f59e0b",
        error: "#ef4444",
        info: "#3b82f6",
      },
    },
  },
  plugins: [],
};
