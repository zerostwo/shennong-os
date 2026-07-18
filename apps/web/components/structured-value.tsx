import type { ReactNode } from "react";

type StructuredValueProps = { value: unknown; emptyLabel?: string; depth?: number };

function fieldLabel(value: string) {
  return value.replaceAll("_", " ").replace(/\b\w/g, (character) => character.toUpperCase());
}

function PrimitiveValue({ value }: { value: unknown }): ReactNode {
  if (value === null || value === undefined || value === "") return <span className="structured-empty">Not available</span>;
  if (typeof value === "boolean") return <span className={`structured-boolean ${value ? "is-true" : "is-false"}`}>{value ? "Yes" : "No"}</span>;
  if (typeof value === "number") return <span className="structured-number">{value.toLocaleString()}</span>;
  return <span className="structured-text">{String(value)}</span>;
}

export function StructuredValue({ value, emptyLabel = "No values declared.", depth = 0 }: StructuredValueProps) {
  if (Array.isArray(value)) {
    if (value.length === 0) return <p className="structured-empty">{emptyLabel}</p>;
    if (value.every((item) => item === null || typeof item !== "object")) {
      return <ul className="structured-chip-list">{value.map((item, index) => <li key={`${String(item)}-${index}`}><PrimitiveValue value={item} /></li>)}</ul>;
    }
    return <ol className="structured-record-list">{value.map((item, index) => <li key={index}><span className="structured-index">{index + 1}</span><StructuredValue value={item} depth={depth + 1} /></li>)}</ol>;
  }
  if (value !== null && typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length === 0) return <p className="structured-empty">{emptyLabel}</p>;
    return <dl className={`structured-object structured-depth-${Math.min(depth, 3)}`}>{entries.map(([key, item]) => <div key={key} className="structured-field"><dt>{fieldLabel(key)}</dt><dd><StructuredValue value={item} emptyLabel={emptyLabel} depth={depth + 1} /></dd></div>)}</dl>;
  }
  return <PrimitiveValue value={value} />;
}
