import { openUrl } from "@tauri-apps/plugin-opener";
import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { MermaidDiagram } from "./MermaidDiagram";

// Agent output is untrusted: links leave through the system browser instead of
// navigating the app, and images are never fetched.
const components: Components = {
  a({ href, children }) {
    const external = href !== undefined && /^https?:\/\//.test(href);
    return (
      <a
        href={href}
        title={href}
        onClick={(event) => {
          event.preventDefault();
          // A refusal (a scope the capability does not grant, no browser) must not pass silently.
          if (external) openUrl(href).catch((error) => console.error(`cannot open ${href}:`, error));
        }}
      >
        {children}
      </a>
    );
  },
  img({ alt, src }) {
    return <span className="md-image">[image: {alt || src}]</span>;
  },
  // A ```mermaid block is drawn as its diagram; the `pre` around it goes away so the
  // figure is not shown as code.
  pre({ children, node }) {
    const code = node?.children[0];
    if (code?.type === "element" && code.tagName === "code" && isMermaid(code.properties?.className)) {
      return <>{children}</>;
    }
    return <pre>{children}</pre>;
  },
  code({ className, children }) {
    if (isMermaid(className)) return <MermaidDiagram source={String(children).replace(/\n$/, "")} />;
    return <code className={className}>{children}</code>;
  },
};

function isMermaid(className: unknown): boolean {
  const names = Array.isArray(className) ? className : String(className ?? "").split(" ");
  return names.includes("language-mermaid");
}

const plugins = [remarkGfm];

export const Markdown = memo(function Markdown({ text }: { text: string }) {
  return (
    <div className="markdown">
      <ReactMarkdown remarkPlugins={plugins} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
});
