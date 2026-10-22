import { PanelBody } from './panels';
import type { PanelState } from './types';

export function Surface2D({
  panels,
  onPanelEvent,
}: {
  panels: Map<string, PanelState>;
  onPanelEvent?: (panel_id: string, data: unknown) => void;
}) {
  const panelList = Array.from(panels.values());

  if (panelList.length === 0) {
    return null;
  }

  return (
    <div className="pointer-events-none absolute inset-0 overflow-auto p-4 pt-14">
      <div className="pointer-events-auto grid w-full grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
        {panelList.map((panel) => (
          <SurfacePanel
            key={panel.id}
            panel={panel}
            onEvent={(data) => onPanelEvent?.(panel.id, data)}
          />
        ))}
      </div>
    </div>
  );
}

function SurfacePanel({
  panel,
  onEvent,
}: {
  panel: PanelState;
  onEvent?: (data: unknown) => void;
}) {
  const highlighted =
    panel.highlighted && panel.highlightUntil && Date.now() < panel.highlightUntil;
  const outline =
    highlighted && panel.highlighted?.type === 'outline'
      ? {
          borderColor: panel.highlighted.color,
          boxShadow: `0 0 0 ${panel.highlighted.width}px ${panel.highlighted.color}`,
        }
      : undefined;

  return (
    <section
      className="min-h-[120px] rounded border border-slate-700 bg-slate-950/90 p-3 text-slate-100 shadow-xl backdrop-blur"
      style={outline}
    >
      <div className="mb-2 flex items-center justify-between gap-2 border-b border-slate-800 pb-2">
        <div className="min-w-0">
          <h2 className="truncate text-sm font-semibold text-slate-100">{panel.title}</h2>
          <div className="mt-0.5 text-[10px] uppercase text-slate-500">
            {panel.kind} · {panel.id}
          </div>
        </div>
      </div>
      <PanelBody kind={panel.kind} content={panel.content} onEvent={onEvent} />
    </section>
  );
}
