import { openUrl } from "@tauri-apps/plugin-opener";
import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

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
          if (external) void openUrl(href);
        }}
      >
        {children}
      </a>
    );
  },
  img({ alt, src }) {
    return <span className="md-image">[image: {alt || src}]</span>;
  },
};

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
