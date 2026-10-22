// Mirror of big-head-view Rust protocol types (crates/big-head-view/src/protocol.rs).
// Wire format is JSON; we deserialize `op` and route.

export type PanelKind =
  | 'table'
  | 'chart'
  | 'card'
  | 'form'
  | 'html'
  | 'scene3d'
  | 'a2ui'
  | 'inspector';

export type ViewSurface = 'spatial3d' | 'html2d';

export type ChartType = 'bar' | 'line' | 'pie' | 'scatter';

export interface ChartPoint {
  x: string | number;
  y: number;
}

export interface ChartSeries {
  name: string;
  data: ChartPoint[];
  color?: string;
}

export type TrendDirection = 'up' | 'down' | 'flat';

export interface TrendIndicator {
  direction: TrendDirection;
  magnitude: number;
}

export type FormFieldType =
  | 'text'
  | 'number'
  | 'boolean'
  | { select: { options: string[] } };

export interface FormField {
  name: string;
  label: string;
  field_type: FormFieldType;
  placeholder?: string;
  default?: unknown;
}

export interface EntityNode {
  entity_id: number;
  label: string;
  position: [number, number, number];
}

export interface EntityEdge {
  from: number;
  to: number;
  label?: string;
}

export interface A2UIComponent {
  id: string;
  component: string;
  [key: string]: unknown;
}

export interface A2UIContent {
  catalog_id: string;
  components: A2UIComponent[];
}

export type InspectorStatus =
  | 'idle'
  | 'healthy'
  | 'pending'
  | 'running'
  | 'warning'
  | 'error'
  | 'done';

export type InspectorTone =
  | 'neutral'
  | 'info'
  | 'success'
  | 'warning'
  | 'danger';

export interface InspectorMetric {
  label: string;
  value: unknown;
  tone?: InspectorTone;
}

export interface InspectorField {
  label: string;
  value: unknown;
}

export interface InspectorEntry {
  entity_id?: number;
  label: string;
  status?: InspectorStatus;
  fields?: InspectorField[];
}

export type PanelContent =
  | { kind: 'table'; columns: string[]; rows: unknown[][] }
  | { kind: 'chart'; chart_type: ChartType; series: ChartSeries[] }
  | {
      kind: 'card';
      label: string;
      value: unknown;
      unit?: string;
      trend?: TrendIndicator;
    }
  | { kind: 'form'; fields: FormField[]; submit_action?: string }
  | { kind: 'html'; markup: string }
  | { kind: 'scene3d'; nodes: EntityNode[]; edges: EntityEdge[] }
  | ({ kind: 'a2ui' } & A2UIContent)
  | {
      kind: 'inspector';
      component: string;
      summary?: InspectorMetric[];
      entries?: InspectorEntry[];
      empty_message?: string;
    };

export type ContentPatch =
  | { patch: 'full'; content: PanelContent }
  | { patch: 'data_model'; path: string; value: unknown };

export type HighlightEffect =
  | { type: 'pulse'; color: string }
  | { type: 'outline'; color: string; width: number }
  | { type: 'shake'; intensity: number };

export type ViewCommand =
  | {
      op: 'spawn';
      id: string;
      surface?: ViewSurface;
      kind: PanelKind;
      title: string;
      position: [number, number, number];
      rotation?: [number, number, number];
      content: PanelContent;
    }
  | { op: 'update'; id: string; patch: ContentPatch }
  | {
      op: 'move';
      id: string;
      position: [number, number, number];
      rotation?: [number, number, number];
      duration_ms?: number;
    }
  | { op: 'scale'; id: string; scale: [number, number, number] }
  | { op: 'despawn'; id: string }
  | {
      op: 'highlight';
      id: string;
      effect: HighlightEffect;
      duration_ms: number;
    }
  | { op: 'batch'; commands: ViewCommand[] };

// Local panel state — what the renderer actually tracks.
export interface PanelState {
  id: string;
  surface: ViewSurface;
  kind: PanelKind;
  title: string;
  position: [number, number, number];
  rotation?: [number, number, number];
  scale: [number, number, number];
  content: PanelContent;
  highlighted?: HighlightEffect;
  highlightUntil?: number; // epoch ms
}

// User → Agent events.
//
// `view_event` 是一般用途：用户喺 panel 上做咗样嘢 (form submit,
// button click, future 任何交互). `data` 嘅 schema 由 agent 喺 spawn
// panel 嗰阵定义 —— frontend 唔 hardcode, 净系 pass-through.
export type UserEvent =
  | { type: 'drag_end'; id: string; position: [number, number, number] }
  | { type: 'view_event'; panel_id: string; data: unknown };
