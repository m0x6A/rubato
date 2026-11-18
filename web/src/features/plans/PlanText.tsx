import { Suspense, lazy, useState } from "react";

const Markdown = lazy(() => import("./Markdown.tsx"));

const isMarkdown = (path: string) => /\.(md|markdown)$/i.test(path);

/** One file of a plan draft: Markdown rendered as a document (ADR 0079), with the raw text one
 * click away; anything else, and the text while the renderer loads, as it was written. */
export function PlanText({ path, text }: { path: string; text: string }) {
  const [raw, setRaw] = useState(false);
  const source = <pre className="draft-text">{text}</pre>;
  if (!isMarkdown(path)) {
    return source;
  }
  return (
    <div className="plan-text">
      <p className="plan-text-mode">
        <button type="button" className="link" aria-pressed={raw} data-testid="plan-text-raw" onClick={() => setRaw((r) => !r)}>
          {raw ? "Rendered" : "Raw"}
        </button>
      </p>
      {raw ? source : <Suspense fallback={source}><Markdown text={text} /></Suspense>}
    </div>
  );
}
