import {
  useCallback,
  useRef,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import { MindView } from './MindView';
import { PanelBody } from './panels';
import { dragRatio, usePersistentNumber } from './resize';
import type { PanelState } from './types';

const MIND_PANEL_IDS = new Set(['mind-view', 'mind_view', 'mind']);

export function Workspace2D({
  panels,
  onPanelEvent,
}: {
  panels: Map<string, PanelState>;
  onPanelEvent?: (panel_id: string, data: unknown) => void;
}) {
  const panelList = Array.from(panels.values());
  const mindPanel = panelList.find(isMindPanel);
  const infoPanels = panelList.filter((panel) => !isMindPanel(panel));
  const [mindPanePct, setMindPanePct] = usePersistentNumber('big-head:view:mind-pane-pct', 62);
  const leftRef = useRef<HTMLDivElement>(null);

  const handleMindBoardResize = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const container = leftRef.current;
    if (!container) return;
    event.preventDefault();
    dragRatio(event, container, 'y', 35, 78, setMindPanePct);
  }, [setMindPanePct]);

  return (
    <div
      ref={leftRef}
      className="workspace-left"
      style={{ '--mind-pane': `${mindPanePct}%` } as CSSProperties}
    >
      <MindView panel={mindPanel} />
      <div
        className="split-resizer split-resizer-horizontal"
        role="separator"
        aria-label="Resize Mind View and agent board"
        aria-orientation="horizontal"
        onPointerDown={handleMindBoardResize}
      />
      <section className="info-board">
        <div className="board-header">
          <div>
            <div className="board-eyebrow">agent board</div>
            <h2>Information panels</h2>
          </div>
          <span>{infoPanels.length}</span>
        </div>
        {infoPanels.length === 0 ? (
          <EmptyBoard />
        ) : (
          <div className="board-list">
            {infoPanels.map((panel) => (
              <InfoPanel
                key={panel.id}
                panel={panel}
                onEvent={(data) => onPanelEvent?.(panel.id, data)}
              />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}

function isMindPanel(panel: PanelState): boolean {
  return (
    panel.surface === 'html2d' &&
    panel.kind === 'html' &&
    (MIND_PANEL_IDS.has(panel.id) || panel.title.toLowerCase().includes('mind-view'))
  );
}

function InfoPanel({
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
    <article className="info-panel" style={outline}>
      <div className="info-panel-title">
        <div className="min-w-0">
          <h3>{panel.title}</h3>
          <p>
            {panel.kind} · {panel.id}
          </p>
        </div>
      </div>
      <PanelBody kind={panel.kind} content={panel.content} onEvent={onEvent} />
    </article>
  );
}

function EmptyBoard() {
  return (
    <div className="empty-board">
      <p>Agent can publish tables, forms, cards, charts, or A2UI controls here.</p>
      <code>surface: "html2d"</code>
    </div>
  );
}
