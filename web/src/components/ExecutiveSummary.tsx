import type { ExecutiveSummary as ExecutiveSummaryData, ExecutiveSummaryItem } from "../types";

type SummarySectionProps = {
  title: string;
  items: ExecutiveSummaryItem[];
  onJumpToFile: (filePath: string) => void;
};

function formatBytes(value?: number | null) {
  if (typeof value !== "number") {
    return "unknown size";
  }
  if (value >= 1024 * 1024) {
    return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  }
  if (value >= 1024) {
    return `${Math.ceil(value / 1024)} KB`;
  }
  return `${value} bytes`;
}

function formatLines(value?: number | null) {
  return typeof value === "number" ? `${value} lines` : "unknown lines";
}

function SummarySection({ title, items, onJumpToFile }: SummarySectionProps) {
  if (items.length === 0) {
    return null;
  }

  return (
    <div className="executive-summary-section">
      <h3>{title}</h3>
      <div className="executive-summary-list">
        {items.map((item) => (
          <button
            key={`${title}:${item.file_path}:${item.label}`}
            className="executive-summary-item"
            type="button"
            onClick={() => onJumpToFile(item.file_path)}
            title={item.file_path}
          >
            <span className="executive-summary-item-main">
              <span className="executive-summary-item-label">{item.label}</span>
              <span className="executive-summary-item-path">{item.file_path}</span>
            </span>
            <span className="executive-summary-item-reason">{item.reason}</span>
            <span className="executive-summary-item-meta">
              <span>{formatBytes(item.byte_size)}</span>
              <span>{formatLines(item.line_count)}</span>
              <span>++{item.added_line_count}</span>
              <span>--{item.removed_line_count}</span>
              <span>{item.hunk_count} hunks</span>
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

export function ExecutiveSummary({
  summary,
  onJumpToFile,
}: {
  summary?: ExecutiveSummaryData | null;
  onJumpToFile: (filePath: string) => void;
}) {
  if (!summary) {
    return null;
  }

  const itemCount =
    summary.large_files.length +
    summary.large_new_files.length +
    summary.hotspots.length +
    summary.complexity_hints.length;
  if (itemCount === 0) {
    return null;
  }

  return (
    <section className="panel panel-plain executive-summary">
      <div className="executive-summary-head">
        <h2>Executive summary</h2>
        <span className="muted">{itemCount} signals</span>
      </div>
      <div className="executive-summary-grid">
        <SummarySection title="Large files" items={summary.large_files} onJumpToFile={onJumpToFile} />
        <SummarySection title="Large new files" items={summary.large_new_files} onJumpToFile={onJumpToFile} />
        <SummarySection title="Hotspots" items={summary.hotspots} onJumpToFile={onJumpToFile} />
        <SummarySection title="Complexity hints" items={summary.complexity_hints} onJumpToFile={onJumpToFile} />
      </div>
    </section>
  );
}
