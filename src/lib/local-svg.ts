import DOMPurify from "dompurify";

// Keep SVG out of the application DOM, with no active or external resources.
// Returns sanitized SVG text (used as a data URL for in-app preview, or written
// straight to a file for the site export).
export function sanitizeSvg(source: string): string {
  const clean = DOMPurify.sanitize(source, {
    USE_PROFILES: { svg: true, svgFilters: true },
    FORBID_TAGS: ["script", "foreignObject", "image", "feImage", "a", "use", "style", "animate", "animateMotion", "animateTransform", "set"],
    FORBID_ATTR: ["href", "xlink:href", "style"],
  });
  const document = new DOMParser().parseFromString(clean, "image/svg+xml");
  if (document.querySelector("parsererror") || document.documentElement.localName !== "svg") throw new Error("Invalid SVG from local renderer.");
  for (const element of document.querySelectorAll("*")) {
    for (const attr of [...element.attributes]) {
      // url(#id) points inside this same SVG (arrowhead markers, gradients) and fetches
      // nothing; any other url() goes, including one spelled with CSS escapes.
      const external = attr.value.replace(/url\(\s*(["']?)#[\w.:-]+\1\s*\)/gi, "");
      if (/url\s*\(|\\/i.test(external)) element.removeAttributeNode(attr);
    }
  }
  return new XMLSerializer().serializeToString(document.documentElement);
}

export function localSvgUrl(source: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(sanitizeSvg(source))}`;
}
