// The Tidycraft mark ("Gleam"): four-facet amber gem + cyan companion spark.
// Same geometry AND framing as the app icon, whose master is app-icon.svg at
// the repo root — the 96-unit space is inset so the gem's 62-unit diagonal
// spans 94% of the box. Keep the two in step: a mark that sits smaller in its
// box than the icon does reads as a different logo, not a smaller one.
//
// No halo, unlike earlier versions. It spilled out to x[11,85], past the inset
// viewBox, so at this framing it would render with a flat edge sliced off it.
//
// Facet colors are baked per theme — SVG presentation attributes can't resolve
// CSS var() — so a dark and a light group coexist and [data-theme] toggles them
// via .tc-brand-mark rules in redesign-components.css. The light group is NOT
// the dark one darkened: the header's --panel is pure white in that theme,
// where the dark group's two brightest facets sit at 1.6:1 and 1.8:1 and melt
// into the background. These hold the amber hue instead of drifting red-brown
// the way the previous light set did (2.2 / 2.9 / 3.9 / 6.3 against white),
// so the mark still reads as the same amber as the taskbar icon while the
// darkest facet anchors the silhouette.
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
