// The Tidycraft mark ("Gleam"): four-facet amber gem + cyan companion spark. Same
// geometry and framing as the app icon, whose master is app-icon.svg at the repo
// root. Facet colours are baked per theme and toggled by [data-theme] in CSS.
export function BrandMark({ size = 24 }: { size?: number }) {
  return (
    <svg
      className="tc-brand-mark"
      width={size}
      height={size}
      viewBox="15.02 15.02 65.96 65.96"
      aria-hidden="true"
    >
      <g className="bm-dark">
        <polygon points="17,48 48,17 42,42" fill="#f8d376" />
        <polygon points="48,17 79,48 42,42" fill="#ffb330" />
        <polygon points="48,79 17,48 42,42" fill="#e69825" />
        <polygon points="79,48 48,79 42,42" fill="#b45c03" />
        <path
          d="M17,48 L48,17 L79,48"
          fill="none"
          stroke="#f8d376"
          strokeWidth="1.4"
          opacity="0.5"
          strokeLinejoin="round"
        />
        <polygon points="69,16.5 76.5,24 69,31.5 61.5,24" fill="#6cd9d8" />
      </g>
      <g className="bm-light">
        <polygon points="17,48 48,17 42,42" fill="#e0a63f" />
        <polygon points="48,17 79,48 42,42" fill="#d18a1c" />
        <polygon points="48,79 17,48 42,42" fill="#b57210" />
        <polygon points="79,48 48,79 42,42" fill="#8f5104" />
        <polygon points="69,16.5 76.5,24 69,31.5 61.5,24" fill="#008a92" />
      </g>
    </svg>
  );
}
