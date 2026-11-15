import type { Page } from "@playwright/test";

/**
 * Add a rule to the page through the CSSOM, as a constructed stylesheet. The server's
 * Content-Security-Policy (`default-src 'self'`, ADR 0066) refuses an inline `<style>`, so
 * `page.addStyleTag` throws; a constructed sheet is not inline markup and is allowed.
 * Returns a function that takes the sheet out again.
 */
export async function addCss(page: Page, css: string): Promise<() => Promise<void>> {
  const id = await page.evaluate((text) => {
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(text);
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
    const w = window as unknown as { __e2eSheets?: CSSStyleSheet[] };
    w.__e2eSheets = [...(w.__e2eSheets ?? []), sheet];
    return w.__e2eSheets.length - 1;
  }, css);
  return async () => {
    await page.evaluate((i) => {
      const sheet = (window as unknown as { __e2eSheets: CSSStyleSheet[] }).__e2eSheets[i];
      document.adoptedStyleSheets = document.adoptedStyleSheets.filter((s) => s !== sheet);
    }, id);
  };
}
