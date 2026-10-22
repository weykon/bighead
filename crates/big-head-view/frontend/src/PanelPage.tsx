import { PanelBody } from './panels';
import type { PanelState } from './types';

export function PanelPage({
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
          boxShadow: `inset 0 0 0 ${panel.highlighted.width}px ${panel.highlighted.color}`,
        }
      : undefined;

  return (
    <section className="panel-page" style={outline}>
      <header className="panel-page-header">
        <div className="min-w-0">
          <div className="panel-page-eyebrow">
            {panel.id.startsWith('document-') ? '閱讀' : `${panel.surface} / ${panel.kind}`}
          </div>
          <h2>{panel.title}</h2>
        </div>
        {!panel.id.startsWith('document-') && <div className="panel-page-id" title={panel.id}>{panel.id}</div>}
      </header>
      <div className="panel-page-scroll">
        <div className={`panel-page-content panel-page-content-${panel.kind}`}>
          <PanelBody kind={panel.kind} content={panel.content} onEvent={onEvent} />
        </div>
      </div>
    </section>
  );
}
