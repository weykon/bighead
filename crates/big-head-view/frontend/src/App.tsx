// Top-level App. Wires WebSocket -> reducer -> tabbed view surfaces.

import {
  Component,
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { Canvas } from "@react-three/fiber";
import { MindView } from "./MindView";
import { CharacterDemo, CHARACTER_PANEL } from "./CharacterDemo";
import { PanelPage } from "./PanelPage";
import { Scene3D } from "./Scene3D";
import { applyCommand, sendViewEvent, WsClient, type WsSnapshot } from "./ws";
import type { PanelState, ViewCommand } from "./types";

const MIND_PANEL_IDS = new Set(["mind-view", "mind_view", "mind"]);

type WorkspaceTab =
  | {
      key: string;
      type: "panel";
      panelId: string;
      label: string;
      detail: string;
    }
  | {
      key: "spatial";
      type: "spatial";
      label: string;
      detail: string;
    };

function panelsReducer(
  state: Map<string, PanelState>,
  cmd: ViewCommand,
): Map<string, PanelState> {
  return applyCommand(state, cmd);
}

export default function App() {
  const [panels, dispatch] = useReducer(
    panelsReducer,
    new Map<string, PanelState>(),
  );
  const [wsSnapshot, setWsSnapshot] = useState<WsSnapshot>({
    status: "connecting",
    retry: 0,
    maxRetries: 5,
    nextRetryMs: 0,
  });
  const [cameraMode, setCameraMode] = useState<"idle" | "fly" | "orbit">(
    "idle",
  );
  const [selectedTabKey, setSelectedTabKey] = useState<string | null>(() =>
    tabKeyFromHash(window.location.hash),
  );
  const spatialPanels = useMemo(
    () => filterPanels(panels, "spatial3d"),
    [panels],
  );
  const htmlPanels = useMemo(() => {
    const result = filterPanels(panels, "html2d");
    if (!result.has(CHARACTER_PANEL)) result.set(CHARACTER_PANEL, {
      id: CHARACTER_PANEL, surface: 'html2d', kind: 'a2ui', title: '小小世界', position: [0, 0, 0], scale: [1, 1, 1],
      content: { kind: 'a2ui', catalog_id: 'default', components: [] },
    });
    return result;
  }, [panels]);
  const tabs = useMemo(
    () => buildWorkspaceTabs(htmlPanels, spatialPanels.size),
    [htmlPanels, spatialPanels.size],
  );
  const activeTab = useMemo(
    () => resolveActiveTab(tabs, selectedTabKey),
    [tabs, selectedTabKey],
  );

  useEffect(() => {
    const proto = window.location.protocol === "https:" ? "wss" : "ws";
    const url = `${proto}://${window.location.host}/ws`;
    const client = new WsClient(url);
    client.connect((cmd) => {
      dispatch(cmd);
    });
    const id = setInterval(() => setWsSnapshot(client.getSnapshot()), 250);
    return () => {
      clearInterval(id);
      client.close();
    };
  }, []);

  useEffect(() => {
    const syncFromLocation = () =>
      setSelectedTabKey(tabKeyFromHash(window.location.hash));
    window.addEventListener("hashchange", syncFromLocation);
    window.addEventListener("popstate", syncFromLocation);
    return () => {
      window.removeEventListener("hashchange", syncFromLocation);
      window.removeEventListener("popstate", syncFromLocation);
    };
  }, []);

  useEffect(() => {
    if (activeTab?.type !== "spatial") {
      setCameraMode("idle");
    }
  }, [activeTab?.type]);

  const selectTab = useCallback((key: string) => {
    setSelectedTabKey(key);
    const nextHash = hashForTabKey(key);
    if (window.location.hash !== nextHash) {
      window.history.pushState(null, "", nextHash);
    }
  }, []);

  const handleTabKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key))
        return;
      event.preventDefault();
      const currentIndex = Math.max(
        0,
        tabs.findIndex((tab) => tab.key === activeTab?.key),
      );
      let nextIndex = currentIndex;
      if (event.key === "ArrowLeft")
        nextIndex = (currentIndex - 1 + tabs.length) % tabs.length;
      if (event.key === "ArrowRight")
        nextIndex = (currentIndex + 1) % tabs.length;
      if (event.key === "Home") nextIndex = 0;
      if (event.key === "End") nextIndex = tabs.length - 1;
      const nextTab = tabs[nextIndex];
      if (!nextTab) return;
      selectTab(nextTab.key);
      requestAnimationFrame(() => {
        document.getElementById(tabDomId(nextTab.key))?.focus();
      });
    },
    [activeTab?.key, selectTab, tabs],
  );

  const handlePanelMove = useCallback(
    (
      id: string,
      position: [number, number, number],
      rotation?: [number, number, number],
    ) => {
      dispatch({ op: "move", id, position, rotation });
    },
    [],
  );

  const handlePanelEvent = useCallback((panelId: string, data: unknown) => {
    sendViewEvent(panelId, data);
  }, []);

  const activePanel =
    activeTab?.type === "panel" ? htmlPanels.get(activeTab.panelId) : undefined;

  return (
    <div className="view-root">
      <header className="view-header">
        <div className="view-brand">
          <div className="view-eyebrow">{activePanel?.id.startsWith('document-') ? '閱讀與思考' : 'Big Head runtime'}</div>
          <h1>{activePanel?.id.startsWith('document-') ? 'Big Head' : 'ECS View Inspector'}</h1>
        </div>
        <StatusBar
          ws={wsSnapshot}
          panelCount={panels.size}
          htmlPanelCount={htmlPanels.size}
          spatialPanelCount={spatialPanels.size}
          cameraMode={cameraMode}
        />
      </header>

      <nav className="workspace-tabs" aria-label="Runtime views">
        <div
          className="workspace-tab-list"
          role="tablist"
          aria-orientation="horizontal"
          onKeyDown={handleTabKeyDown}
        >
          {tabs.map((tab) => {
            const selected = tab.key === activeTab?.key;
            return (
              <button
                id={tabDomId(tab.key)}
                key={tab.key}
                className="workspace-tab"
                type="button"
                role="tab"
                aria-selected={selected}
                aria-controls="workspace-active-page"
                tabIndex={selected ? 0 : -1}
                title={`${tab.label} · ${tab.detail}`}
                onClick={() => selectTab(tab.key)}
              >
                <span className="workspace-tab-label">{tab.label}</span>
                <span className="workspace-tab-detail">{tab.detail}</span>
              </button>
            );
          })}
        </div>
      </nav>

      <main
        id="workspace-active-page"
        className="workspace-page"
        role="tabpanel"
        aria-labelledby={activeTab ? tabDomId(activeTab.key) : undefined}
      >
        {activePanel?.id === CHARACTER_PANEL ? (
          <CharacterDemo panel={activePanel} connected={wsSnapshot.status === 'open'} />
        ) : activeTab?.type === "spatial" ? (
          <SpatialPage
            panels={spatialPanels}
            cameraMode={cameraMode}
            onCameraModeChange={setCameraMode}
            onPanelMove={handlePanelMove}
            onPanelEvent={handlePanelEvent}
          />
        ) : activePanel && isMindPanel(activePanel) ? (
          <MindView
            panel={activePanel}
            onEvent={(data) => handlePanelEvent(activePanel.id, data)}
          />
        ) : activePanel ? (
          <PanelPage
            panel={activePanel}
            onEvent={(data) => handlePanelEvent(activePanel.id, data)}
          />
        ) : (
          <EmptyWorkspace ws={wsSnapshot} />
        )}
      </main>
    </div>
  );
}

function SpatialPage({
  panels,
  cameraMode,
  onCameraModeChange,
  onPanelMove,
  onPanelEvent,
}: {
  panels: Map<string, PanelState>;
  cameraMode: "idle" | "fly" | "orbit";
  onCameraModeChange: (mode: "idle" | "fly" | "orbit") => void;
  onPanelMove: (
    id: string,
    position: [number, number, number],
    rotation?: [number, number, number],
  ) => void;
  onPanelEvent: (panelId: string, data: unknown) => void;
}) {
  return (
    <section className="spatial-page">
      <div className="scene-header">
        <div>
          <div className="scene-eyebrow">spatial3d</div>
          <h2>World scene</h2>
        </div>
        <div className="scene-meta">
          <span>{panels.size} panels</span>
          <CameraModeBadge mode={cameraMode} />
        </div>
      </div>
      <div className="scene-canvas">
        <CanvasErrorBoundary>
          <Canvas camera={{ position: [5, 3, 7], fov: 50 }}>
            <Scene3D
              panels={panels}
              onPanelMove={onPanelMove}
              onPanelEvent={onPanelEvent}
              onCameraModeChange={onCameraModeChange}
            />
          </Canvas>
        </CanvasErrorBoundary>
      </div>
    </section>
  );
}

class CanvasErrorBoundary extends Component<
  { children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: unknown) {
    console.error("[spatial3d] canvas unavailable", error);
  }

  render() {
    if (this.state.failed) {
      return (
        <div className="canvas-error" role="alert">
          <strong>Spatial renderer unavailable</strong>
          <span>WebGL could not be initialized in this browser session.</span>
        </div>
      );
    }
    return this.props.children;
  }
}

function EmptyWorkspace({ ws }: { ws: WsSnapshot }) {
  return (
    <section className="empty-workspace">
      <div className="empty-workspace-mark" aria-hidden="true" />
      <h2>No view selected</h2>
      <p>
        {ws.status === "open"
          ? "Waiting for the runtime to publish a panel."
          : "Waiting for the view server connection."}
      </p>
    </section>
  );
}

function buildWorkspaceTabs(
  htmlPanels: Map<string, PanelState>,
  spatialPanelCount: number,
): WorkspaceTab[] {
  const panelTabs = Array.from(htmlPanels.values())
    .sort((left, right) => panelPriority(left) - panelPriority(right))
    .map<WorkspaceTab>((panel) => ({
      key: panelTabKey(panel.id),
      type: "panel",
      panelId: panel.id,
      label: panelTabLabel(panel),
      detail: panel.id.startsWith('document-') ? '閱讀' : panel.kind,
    }));

  if (spatialPanelCount > 0) {
    panelTabs.push({
      key: "spatial",
      type: "spatial",
      label: "Spatial World",
      detail: String(spatialPanelCount),
    });
  }
  return panelTabs;
}

function resolveActiveTab(
  tabs: WorkspaceTab[],
  selectedKey: string | null,
): WorkspaceTab | undefined {
  if (selectedKey) {
    return tabs.find((tab) => tab.key === selectedKey);
  }
  return (
    tabs.find((tab) => tab.type === "panel" && tab.panelId === "mind-view") ??
    tabs[0]
  );
}

function panelPriority(panel: PanelState): number {
  const priorities: Record<string, number> = {
    "mind-view": 0,
    "inspector-tickactions": 10,
    "inspector-sparks": 20,
    "inspector-context": 30,
    "inspector-world": 40,
    "inspector-tools": 50,
    "inspector-view": 60,
    "decision-board": 70,
    "work-board": 80,
  };
  return priorities[panel.id] ?? 100;
}

function panelTabLabel(panel: PanelState): string {
  const labels: Record<string, string> = {
    "mind-view": "Mind View",
    "inspector-tickactions": "TickActions",
    "inspector-sparks": "Sparks",
    "inspector-context": "Context",
    "inspector-world": "ECS World",
    "inspector-tools": "Tool Activity",
    "inspector-view": "View Runtime",
    "decision-board": "Decisions",
    "work-board": "Work",
  };
  return labels[panel.id] ?? panel.title;
}

function panelTabKey(panelId: string): string {
  return `panel:${panelId}`;
}

function hashForTabKey(key: string): string {
  return key === "spatial"
    ? "#spatial"
    : `#panel=${encodeURIComponent(key.slice("panel:".length))}`;
}

function tabKeyFromHash(hash: string): string | null {
  if (hash === "#spatial") return "spatial";
  if (!hash.startsWith("#panel=")) return null;
  const encodedId = hash.slice("#panel=".length);
  if (!encodedId) return null;
  try {
    return panelTabKey(decodeURIComponent(encodedId));
  } catch {
    return null;
  }
}

function tabDomId(key: string): string {
  return `workspace-tab-${key.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
}

function isMindPanel(panel: PanelState): boolean {
  return (
    panel.surface === "html2d" &&
    panel.kind === "html" &&
    (MIND_PANEL_IDS.has(panel.id) ||
      panel.title.toLowerCase().includes("mind-view"))
  );
}

function filterPanels(
  panels: Map<string, PanelState>,
  surface: PanelState["surface"],
): Map<string, PanelState> {
  const next = new Map<string, PanelState>();
  for (const [id, panel] of panels) {
    if (panel.surface === surface) {
      next.set(id, panel);
    }
  }
  return next;
}

function StatusBar({
  ws,
  panelCount,
  htmlPanelCount,
  spatialPanelCount,
  cameraMode,
}: {
  ws: WsSnapshot;
  panelCount: number;
  htmlPanelCount: number;
  spatialPanelCount: number;
  cameraMode: "idle" | "fly" | "orbit";
}) {
  const color =
    ws.status === "open"
      ? "bg-emerald-500"
      : ws.status === "connecting" || ws.status === "reconnecting"
        ? "bg-amber-500 animate-pulse"
        : "bg-rose-500";
  const wsLabel =
    ws.status === "reconnecting"
      ? `Reconnecting · attempt ${ws.retry}`
      : ws.status === "closed" && ws.retry >= ws.maxRetries
        ? "Closed"
        : ws.status;
  return (
    <div className="status-bar" aria-label="Runtime connection status">
      <span className={`h-2 w-2 shrink-0 rounded-full ${color}`} />
      <span className="status-connection">{wsLabel}</span>
      <span className="status-separator">|</span>
      <span>{panelCount} panels</span>
      <span className="status-separator">|</span>
      <span>{htmlPanelCount} 2D</span>
      <span className="status-separator">|</span>
      <span>{spatialPanelCount} 3D</span>
      {cameraMode !== "idle" && (
        <>
          <span className="status-separator">|</span>
          <span>cam {cameraMode}</span>
        </>
      )}
    </div>
  );
}

function CameraModeBadge({ mode }: { mode: "idle" | "fly" | "orbit" }) {
  if (mode === "fly") {
    return <span className="text-cyan-300 font-semibold">FLY</span>;
  }
  if (mode === "orbit") {
    return <span className="text-amber-300 font-semibold">ORBIT</span>;
  }
  return <span className="text-slate-400">idle</span>;
}
