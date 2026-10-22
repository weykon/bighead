# big-head-view Frontend

React + R3F + drei 3D floating panel renderer.

## Build

```bash
cd crates/big-head-view/frontend
npm install
npm run build       # outputs to dist/
```

The `big-head-view` axum server serves `dist/` at the root URL.

## Dev (with HMR)

```bash
npm run dev
# Vite serves on http://localhost:5173 with /ws proxy → ws://localhost:3030/ws
```

Then in a separate terminal:
```bash
cargo run -p big-head
# which will start the view server on :3030
```

## Architecture

- `src/types.ts` — protocol mirror (must stay in sync with `big-head-view/src/protocol.rs`)
- `src/ws.ts` — WebSocket client + `applyCommand` reducer
- `src/panels.tsx` — per-PanelKind content renderers
- `src/Scene3D.tsx` — R3F scene with floating `<Html>` panels
- `src/App.tsx` — top-level: wires WebSocket → reducer → scene
