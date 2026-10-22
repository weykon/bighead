// Per-PanelKind content renderers. Each takes the `content` payload and
// returns a small React element. These get embedded inside drei <Html>.

import { useEffect, useRef, useState } from 'react';
import type {
  FormField,
  PanelContent,
  PanelKind,
} from './types';

export function PanelBody({
  kind,
  content,
  onEvent,
}: {
  kind: PanelKind;
  content: PanelContent;
  onEvent?: (data: unknown) => void;
}) {
  switch (kind) {
    case 'table':
      return content.kind === 'table' ? (
        <TableView content={content} />
      ) : null;
    case 'chart':
      return content.kind === 'chart' ? (
        <ChartView content={content} />
      ) : null;
    case 'card':
      return content.kind === 'card' ? (
        <CardView content={content} />
      ) : null;
    case 'form':
      return content.kind === 'form' ? (
        <FormView content={content} onEvent={onEvent} />
      ) : null;
    case 'html':
      return content.kind === 'html' ? (
        <HtmlView content={content} />
      ) : null;
    case 'scene3d':
      return content.kind === 'scene3d' ? (
        <Scene3DView content={content} />
      ) : null;
    case 'a2ui':
      return content.kind === 'a2ui' ? (
        <A2UIView content={content} onEvent={onEvent} />
      ) : null;
    case 'inspector':
      return content.kind === 'inspector' ? (
        <InspectorView content={content} />
      ) : null;
  }
}

function InspectorView({
  content,
}: {
  content: Extract<PanelContent, { kind: 'inspector' }>;
}) {
  const summary = content.summary ?? [];
  const entries = content.entries ?? [];

  return (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex items-center justify-between gap-3 border-b border-slate-800 pb-2">
        <span className="min-w-0 truncate font-mono text-xs text-cyan-300">
          {content.component}
        </span>
        <span className="shrink-0 text-[10px] uppercase text-slate-500">
          {entries.length} entities
        </span>
      </div>

      {summary.length > 0 && (
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          {summary.map((metric, index) => (
            <div key={`${metric.label}-${index}`} className="min-w-0 border-l-2 border-slate-700 pl-2">
              <div className="truncate text-[10px] uppercase text-slate-500">{metric.label}</div>
              <div className={`truncate text-sm font-semibold ${inspectorToneClass(metric.tone)}`}>
                {formatInspectorValue(metric.value)}
              </div>
            </div>
          ))}
        </div>
      )}

      {entries.length === 0 ? (
        <div className="py-4 text-center text-xs text-slate-500">
          {content.empty_message ?? `No ${content.component} entities`}
        </div>
      ) : (
        <div className="flex min-w-0 flex-col divide-y divide-slate-800">
          {entries.map((entry, index) => (
            <div key={`${entry.entity_id ?? 'entry'}-${index}`} className="grid min-w-0 gap-2 py-2 md:grid-cols-[minmax(140px,0.8fr)_minmax(0,2fr)]">
              <div className="flex min-w-0 items-start gap-2">
                <span className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${inspectorStatusDot(entry.status)}`} />
                <div className="min-w-0">
                  <div className="truncate text-xs font-semibold text-slate-200">{entry.label}</div>
                  <div className="font-mono text-[10px] text-slate-500">
                    {entry.entity_id == null ? 'unbound' : `eid ${entry.entity_id}`}
                    {' · '}{entry.status ?? 'idle'}
                  </div>
                </div>
              </div>
              <dl className="grid min-w-0 grid-cols-2 gap-x-3 gap-y-1 lg:grid-cols-3">
                {(entry.fields ?? []).map((field, fieldIndex) => (
                  <div key={`${field.label}-${fieldIndex}`} className="min-w-0">
                    <dt className="truncate text-[10px] text-slate-500">{field.label}</dt>
                    <dd className="truncate font-mono text-[11px] text-slate-300" title={formatInspectorValue(field.value)}>
                      {formatInspectorValue(field.value)}
                    </dd>
                  </div>
                ))}
              </dl>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function formatInspectorValue(value: unknown): string {
  if (value == null) return '—';
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function inspectorToneClass(tone: string | undefined): string {
  switch (tone) {
    case 'info': return 'text-cyan-300';
    case 'success': return 'text-emerald-300';
    case 'warning': return 'text-amber-300';
    case 'danger': return 'text-rose-300';
    default: return 'text-slate-200';
  }
}

function inspectorStatusDot(status: string | undefined): string {
  switch (status) {
    case 'healthy':
    case 'done': return 'bg-emerald-400';
    case 'pending': return 'bg-amber-300';
    case 'running': return 'bg-cyan-300';
    case 'warning': return 'bg-orange-400';
    case 'error': return 'bg-rose-400';
    default: return 'bg-slate-500';
  }
}

function TableView({ content }: { content: Extract<PanelContent, { kind: 'table' }> }) {
  if (content.kind !== 'table') return null;
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-xs">
        <thead>
          <tr className="text-left border-b border-slate-700">
            {content.columns.map((c, i) => (
              <th key={i} className="py-1 pr-3 font-semibold text-slate-300">
                {c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {content.rows.map((row, ri) => (
            <tr key={ri} className="border-b border-slate-800/50">
              {row.map((cell, ci) => (
                <td key={ci} className="py-1 pr-3 text-slate-200">
                  {String(cell)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ChartView({ content }: { content: Extract<PanelContent, { kind: 'chart' }> }) {
  if (content.kind !== 'chart') return null;
  // Tiny inline SVG. Real impl would use a charting lib.
  const W = 280;
  const H = 140;
  const PAD = 16;
  const allPoints = content.series.flatMap((s) => s.data);
  const yMin = Math.min(...allPoints.map((p) => p.y), 0);
  const yMax = Math.max(...allPoints.map((p) => p.y), 1);
  const yRange = yMax - yMin || 1;
  const xCount = Math.max(...content.series.map((s) => s.data.length), 1);

  return (
    <div>
      <svg width={W} height={H} className="bg-slate-900/40 rounded">
        {content.series.map((s, si) => {
          const stepX = (W - 2 * PAD) / Math.max(xCount - 1, 1);
          const points = s.data.map((p, i) => {
            const x = PAD + i * stepX;
            const y = H - PAD - ((p.y - yMin) / yRange) * (H - 2 * PAD);
            return `${x},${y}`;
          });
          const color = s.color ?? ['#22d3ee', '#facc15', '#f87171', '#a78bfa'][si % 4];
          if (content.chart_type === 'bar') {
            const barW = stepX * 0.6;
            return (
              <g key={si}>
                {s.data.map((p, i) => {
                  const x = PAD + i * stepX;
                  const y = H - PAD - ((p.y - yMin) / yRange) * (H - 2 * PAD);
                  return (
                    <rect
                      key={i}
                      x={x - barW / 2}
                      y={y}
                      width={barW}
                      height={H - PAD - y}
                      fill={color}
                      opacity={0.85}
                    />
                  );
                })}
              </g>
            );
          }
          return (
            <polyline
              key={si}
              points={points.join(' ')}
              fill="none"
              stroke={color}
              strokeWidth={2}
            />
          );
        })}
      </svg>
      <div className="flex gap-3 mt-1 text-[10px] text-slate-400">
        {content.series.map((s, i) => (
          <span key={i} className="flex items-center gap-1">
            <span
              className="inline-block w-2 h-2 rounded-full"
              style={{ background: s.color ?? ['#22d3ee', '#facc15', '#f87171', '#a78bfa'][i % 4] }}
            />
            {s.name}
          </span>
        ))}
      </div>
    </div>
  );
}

function CardView({ content }: { content: Extract<PanelContent, { kind: 'card' }> }) {
  if (content.kind !== 'card') return null;
  const trendIcon = content.trend
    ? content.trend.direction === 'up' ? '↑'
    : content.trend.direction === 'down' ? '↓'
    : '→'
    : null;
  const trendColor = content.trend
    ? content.trend.direction === 'up' ? 'text-emerald-400'
    : content.trend.direction === 'down' ? 'text-rose-400'
    : 'text-slate-400'
    : '';
  return (
    <div className="flex flex-col gap-1">
      <div className="text-xs text-slate-400 uppercase tracking-wider">
        {content.label}
      </div>
      <div className="flex items-baseline gap-2">
        <span className="text-3xl font-semibold text-slate-50">
          {String(content.value)}
        </span>
        {content.unit && <span className="text-sm text-slate-400">{content.unit}</span>}
        {trendIcon && (
          <span className={`text-sm ${trendColor}`}>
            {trendIcon} {content.trend?.magnitude}
          </span>
        )}
      </div>
    </div>
  );
}

function FormView({
  content,
  onEvent,
}: {
  content: Extract<PanelContent, { kind: 'form' }>;
  onEvent?: (data: unknown) => void;
}) {
  if (content.kind !== 'form') return null;
  const [values, setValues] = useState<Record<string, unknown>>(() => {
    const init: Record<string, unknown> = {};
    for (const f of content.fields) {
      if (f.default !== undefined) init[f.name] = f.default;
    }
    return init;
  });

  function update(name: string, val: unknown) {
    setValues((v) => ({ ...v, [name]: val }));
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onEvent?.(values);
      }}
      className="flex flex-col gap-2"
    >
      {content.fields.map((f) => (
        <FieldRow key={f.name} field={f} value={values[f.name]} onChange={(v) => update(f.name, v)} />
      ))}
      <button
        type="submit"
        className="mt-2 px-3 py-1.5 rounded bg-cyan-600 hover:bg-cyan-500 text-white text-sm font-medium"
      >
        Submit
      </button>
    </form>
  );
}

function FieldRow({
  field,
  value,
  onChange,
}: {
  field: FormField;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const labelEl = (
    <label className="text-xs text-slate-400">{field.label}</label>
  );
  if (typeof field.field_type === 'object' && 'select' in field.field_type) {
    return (
      <div className="flex flex-col gap-1">
        {labelEl}
        <select
          className="px-2 py-1 rounded bg-slate-800 border border-slate-700 text-sm"
          value={(value as string) ?? ''}
          onChange={(e) => onChange(e.target.value)}
        >
          <option value=""></option>
          {field.field_type.select.options.map((o) => (
            <option key={o} value={o}>{o}</option>
          ))}
        </select>
      </div>
    );
  }
  const t = field.field_type;
  if (t === 'boolean') {
    return (
      <div className="flex items-center gap-2">
        <input
          type="checkbox"
          checked={Boolean(value)}
          onChange={(e) => onChange(e.target.checked)}
        />
        {labelEl}
      </div>
    );
  }
  if (t === 'number') {
    return (
      <div className="flex flex-col gap-1">
        {labelEl}
        <input
          type="number"
          className="px-2 py-1 rounded bg-slate-800 border border-slate-700 text-sm"
          value={(value as number) ?? ''}
          onChange={(e) => onChange(Number(e.target.value))}
        />
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-1">
      {labelEl}
      <input
        type="text"
        className="px-2 py-1 rounded bg-slate-800 border border-slate-700 text-sm"
        placeholder={field.placeholder}
        value={(value as string) ?? ''}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}

function HtmlView({ content }: { content: Extract<PanelContent, { kind: 'html' }> }) {
  if (content.kind !== 'html') return null;
  return <ShadowHtmlView markup={content.markup} />;
}

function ShadowHtmlView({ markup }: { markup: string }) {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const shadow = host.shadowRoot ?? host.attachShadow({ mode: 'open' });
    shadow.replaceChildren();

    const style = document.createElement('style');
    style.textContent = SAFE_AGENT_HTML_CSS;
    const root = document.createElement('div');
    root.className = 'agent-html-root';
    root.innerHTML = sanitizeAgentHtml(markup);

    shadow.append(style, root);
  }, [markup]);

  return <div ref={hostRef} className="agent-html-shadow-host" />;
}

const SAFE_AGENT_HTML_CSS = `
  :host {
    display: block;
    max-width: 100%;
    color: #e2e8f0;
    font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC", system-ui, sans-serif;
    contain: content;
  }
  .agent-html-root {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    font-size: 12px;
    line-height: 1.5;
  }
  .agent-html-root, .agent-html-root * {
    box-sizing: border-box;
    max-width: 100%;
    overflow-wrap: anywhere;
    word-break: normal;
  }
  .agent-html-root * {
    position: static !important;
    inset: auto !important;
    z-index: auto !important;
    transform: none !important;
    float: none !important;
  }
  h1, h2, h3, h4, p, ul, ol, pre, blockquote {
    margin: 0;
  }
  h1 { font-size: 18px; line-height: 1.2; color: #f8fafc; }
  h2 { font-size: 15px; line-height: 1.25; color: #f8fafc; }
  h3, h4 { font-size: 13px; line-height: 1.3; color: #e2e8f0; }
  p, li { color: #cbd5e1; }
  a { color: #67e8f9; }
  img, video { display: block; height: auto; border-radius: 8px; }
  table { display: block; width: 100%; overflow-x: auto; border-collapse: collapse; }
  th, td { border-bottom: 1px solid rgba(148, 163, 184, .18); padding: 6px 8px; text-align: left; vertical-align: top; }
  pre { overflow: auto; border-radius: 8px; background: rgba(2, 6, 23, .72); padding: 10px; color: #e2e8f0; }
  code { font-family: ui-monospace, "SF Mono", Menlo, monospace; color: #c4b5fd; }
  .bh-document { width: 100%; max-width: 760px; margin: 0 auto; padding: 20px 12px 48px; letter-spacing: 0; }
  .bh-document header { padding-bottom: 24px; border-bottom: 1px solid #3e4246; }
  .bh-document .bh-document-summary { font-size: 18px; line-height: 1.8; color: #edf2ef; }
  .bh-document section { padding-top: 28px; }
  .bh-document h2 { font-size: 20px; line-height: 1.5; color: #95dfbd; margin-bottom: 14px; }
  .bh-document p, .bh-document li { font-size: 16px; line-height: 1.95; color: #e0e2e4; white-space: pre-wrap; }
  .bh-document p + p { margin-top: 16px; }
  .bh-document ul { padding-left: 24px; margin-top: 12px; }
  .bh-document li + li { margin-top: 8px; }
  @media (max-width: 600px) { .bh-document { padding: 8px 0 28px; } }
  button, input, select, textarea {
    max-width: 100%;
    border: 1px solid rgba(148, 163, 184, .28);
    border-radius: 6px;
    background: rgba(15, 23, 42, .86);
    color: #e2e8f0;
    padding: 6px 8px;
    font: inherit;
  }
`;

function sanitizeAgentHtml(markup: string): string {
  if (typeof DOMParser === 'undefined') {
    return '';
  }

  const doc = new DOMParser().parseFromString(markup, 'text/html');
  doc
    .querySelectorAll('script, style, link, meta, base, iframe, object, embed')
    .forEach((node) => node.remove());

  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_ELEMENT);
  let current = walker.currentNode as Element | null;
  while (current) {
    for (const attr of Array.from(current.attributes)) {
      const name = attr.name.toLowerCase();
      if (
        name === 'style' ||
        name.startsWith('on') ||
        name === 'srcdoc' ||
        name === 'formaction' ||
        attr.value.trim().toLowerCase().startsWith('javascript:')
      ) {
        current.removeAttribute(attr.name);
      }
    }
    current = walker.nextNode() as Element | null;
  }

  return doc.body.innerHTML;
}

function Scene3DView({ content }: { content: Extract<PanelContent, { kind: 'scene3d' }> }) {
  if (content.kind !== 'scene3d') return null;
  return (
    <div className="flex flex-col gap-1 text-xs">
      <div className="text-slate-400">Nodes: {content.nodes.length}</div>
      <div className="text-slate-400">Edges: {content.edges.length}</div>
      <div className="text-[10px] text-slate-500 italic">
        (rendered as 3D nodes in the scene)
      </div>
    </div>
  );
}

function A2UIView({
  content,
  onEvent,
}: {
  content: Extract<PanelContent, { kind: 'a2ui' }>;
  onEvent?: (data: unknown) => void;
}) {
  if (content.kind !== 'a2ui') return null;
  // Minimal A2UI v0.9 renderer. Recognized: `Button` (clickable, fires view_event
  // with the component's other props as data). Unknown components fall through
  // to a placeholder so future additions are visible.
  return (
    <div className="flex flex-col gap-1 text-xs">
      {content.components.map((c) => {
        if (c.component === 'Button') {
          const { id, component: _ignored, label, ...rest } = c as Record<string, unknown> & {
            id: string;
            component: string;
            label?: string;
          };
          return (
            <button
              key={id}
              type="button"
              className="px-3 py-1.5 rounded bg-cyan-600 hover:bg-cyan-500 text-white text-sm font-medium self-start"
              onClick={() => {
                const payload = { component_id: id, ...rest };
                console.log('[A2UIView] button clicked, payload:', payload, 'onEvent:', !!onEvent);
                onEvent?.(payload);
              }}
            >
              {label ?? id}
            </button>
          );
        }
        return (
          <div key={c.id} className="flex items-baseline gap-2">
            <span className="font-semibold text-cyan-300">{c.component}</span>
            <span className="text-slate-500 text-[10px]">id={c.id}</span>
          </div>
        );
      })}
    </div>
  );
}
