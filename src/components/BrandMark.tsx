import { useId } from "react";

// The Tidycraft mark ("Gleam"): four-facet amber gem + cyan companion spark.
// Same geometry as the app icon set (src-tauri/icons/, generated from
// app-icon.png at 1024px). Facet colors are baked per theme — SVG presentation
// attributes can't resolve CSS var() — so a dark and a light group coexist and
// [data-theme] toggles them via .tc-brand-mark rules in redesign-components.css.
export function BrandMark({ size = 24 }: { size?: number }) {
  const uid = useId();
  const am = `${uid}-am`;
  const cy = `${uid}-cy`;
  const amL = `${uid}-amL`;
  const cyL = `${uid}-cyL`;
  return (
    <svg
      className="tc-brand-mark"
      width={size}
      height={size}
      viewBox="0 0 96 96"
      aria-hidden="true"
    >
      <defs>
        <radialGradient id={am} cx="50%" cy="50%" r="50%">
          <stop offset="0" stopColor="#e69825" stopOpacity="0.4" />
          <stop offset="1" stopColor="#e69825" stopOpacity="0" />
        </radialGradient>
        <radialGradient id={cy} cx="50%" cy="50%" r="50%">
          <stop offset="0" stopColor="#14bbc2" stopOpacity="0.5" />
          <stop offset="1" stopColor="#14bbc2" stopOpacity="0" />
        </radialGradient>
        <radialGradient id={amL} cx="50%" cy="50%" r="50%">
          <stop offset="0" stopColor="#c26300" stopOpacity="0.15" />
          <stop offset="1" stopColor="#c26300" stopOpacity="0" />
        </radialGradient>
        <radialGradient id={cyL} cx="50%" cy="50%" r="50%">
          <stop offset="0" stopColor="#008a92" stopOpacity="0.28" />
          <stop offset="1" stopColor="#008a92" stopOpacity="0" />
        </radialGradient>
      </defs>
      <g className="bm-dark">
        <circle cx="48" cy="53" r="37" fill={`url(#${am})`} />
        <circle cx="69" cy="24" r="13" fill={`url(#${cy})`} opacity="0.65" />
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
        <circle cx="48" cy="53" r="37" fill={`url(#${amL})`} />
        <circle cx="69" cy="24" r="13" fill={`url(#${cyL})`} opacity="0.7" />
        <polygon points="17,48 48,17 42,42" fill="#dd9c42" />
        <polygon points="48,17 79,48 42,42" fill="#c26300" />
        <polygon points="48,79 17,48 42,42" fill="#a74a00" />
        <polygon points="79,48 48,79 42,42" fill="#8c3200" />
        <polygon points="69,16.5 76.5,24 69,31.5 61.5,24" fill="#008a92" />
      </g>
    </svg>
  );
}
