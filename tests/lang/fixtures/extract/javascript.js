/** Fixture docs mention docOnlyJavaScript and should not become code. */
import { widgetSource } from "./widgets.js";

/** Function docs mention docOnlyJavaScript. */
export function makeWidget(source) {
  return source.name;
}

export class GoldenWidget {
  /** Method docs mention docOnlyJavaScript. */
  render(source = widgetSource()) {
    return formatWidget(makeWidget(source));
  }
}

/** Arrow function docs mention docOnlyJavaScript. */
export const formatWidget = (name) => name.trim();
