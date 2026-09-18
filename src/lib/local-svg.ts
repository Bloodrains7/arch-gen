import DOMPurify from "dompurify";

// Keep SVG out of the application DOM, with no active or external resources.
export function localSvgUrl(source: string): string {
  const clean = DOMPurify.sanitize(source, {
    USE_PROFILES: { svg: true, svgFilters: true },
    FORBID_TAGS: ["script", "foreignObject", "image", "feImage", "a", "use", "style", "animate", "animateMotion", "animateTransform", "set"],
    FORBID_ATTR: ["href", "xlink:href", "style"],
  });
  const document = new DOMParser().parseFromString(clean, "image/svg+xml");
  if (document.querySelector("parsererror") || document.documentElement.localName !== "svg") throw new Error("Invalid SVG from local renderer.");
  for (const element of document.querySelectorAll("*")) {
    for (const attr of [...element.attributes]) {
      if (/url\s*\(/i.test(attr.value)) element.removeAttributeNode(attr);
    }
  }
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(new XMLSerializer().serializeToString(document.documentElement))}`;
}
