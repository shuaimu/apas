import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { CodeBlock } from "@/components/code/CodeBlock";

// Stable component types let React keep a table's scroll container mounted
// while the surrounding conversation updates.
const components: Components = {
  code({ className, children, ...props }) {
    const match = /language-(\w+)/.exec(className || "");
    const isInline = !match;

    if (isInline) {
      return (
        <code
          className="bg-gray-200 dark:bg-gray-700 px-1 py-0.5 rounded text-sm"
          {...props}
        >
          {children}
        </code>
      );
    }

    return (
      <CodeBlock
        code={String(children).replace(/\n$/, "")}
        language={match[1]}
      />
    );
  },
  table({ children }) {
    return (
      <div className="max-w-full overflow-x-auto my-2">
        <table className="min-w-full border-collapse text-sm">
          {children}
        </table>
      </div>
    );
  },
  thead({ children }) {
    return (
      <thead className="bg-gray-200 dark:bg-gray-700">
        {children}
      </thead>
    );
  },
  th({ children, style }) {
    return (
      <th scope="col" style={style} className="border border-gray-300 dark:border-gray-600 px-3 py-2 text-left font-semibold">
        {children}
      </th>
    );
  },
  td({ children, style }) {
    return (
      <td style={style} className="border border-gray-300 dark:border-gray-600 px-3 py-2">
        {children}
      </td>
    );
  },
  tr({ children }) {
    return (
      <tr className="even:bg-gray-50 dark:even:bg-gray-800">
        {children}
      </tr>
    );
  },
};

export function ConversationMarkdown({ content }: { content: string }) {
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
      {content}
    </ReactMarkdown>
  );
}
