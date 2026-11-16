import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Plan text as a document (ADR 0079). The text is untrusted: a plan may come from an imported
 * branch. So no raw HTML (it shows as text), no images (a link with the alt text, so opening a
 * plan asks no other host for anything), links through react-markdown's URL filter and into a
 * new tab, and code blocks, Mermaid included, as code.
 *
 * Loaded lazily by `PlanText`, so the first paint does not pay for the parser.
 */
export default function Markdown({ text }: { text: string }) {
  return (
    <div className="md-body" data-testid="md-body">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children }) => (
            <a href={href} target="_blank" rel="noopener noreferrer">
              {children}
            </a>
          ),
          img: ({ src, alt }) => (
            <a href={typeof src === "string" ? src : undefined} target="_blank" rel="noopener noreferrer">
              {alt || "image"}
            </a>
          ),
          // A wide table scrolls inside its own box; the panel never scrolls sideways.
          table: ({ children }) => (
            <div className="md-table">
              <table>{children}</table>
            </div>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
