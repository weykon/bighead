import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { dragRatio, usePersistentNumber } from "./resize";
import type { PanelState } from "./types";

const DEFAULT_MIND_HTML = `<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <style>
    html, body { margin: 0; height: 100%; overflow: hidden; background: #05070d; color: #dbeafe; font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif; }
    canvas { display: block; width: 100vw; height: 100vh; }
    .label { position: fixed; left: 18px; top: 16px; pointer-events: none; }
    .kicker { font: 700 11px ui-monospace, "SF Mono", Menlo, monospace; letter-spacing: .08em; text-transform: uppercase; color: #38bdf8; opacity: .9; }
    .title { margin-top: 4px; font-size: 18px; font-weight: 800; color: #f8fafc; }
    .hint { margin-top: 6px; max-width: 340px; font-size: 12px; line-height: 1.45; color: #94a3b8; }
  </style>
</head>
<body>
  <canvas id="mind"></canvas>
  <div class="label">
    <div class="kicker">programmable mind-view</div>
    <div class="title">WebGL surface ready</div>
    <div class="hint">Agent can replace this iframe with full HTML, shader WebGL, or Three.js by rendering id="mind-view".</div>
  </div>
  <script type="module">
    const canvas = document.getElementById('mind');
    const gl = canvas.getContext('webgl');
    const vertex = \`
      attribute vec2 p;
      varying vec2 v;
      void main() {
        v = p * 0.5 + 0.5;
        gl_Position = vec4(p, 0.0, 1.0);
      }
    \`;
    const fragment = \`
      precision mediump float;
      varying vec2 v;
      uniform vec2 resolution;
      uniform float time;
      void main() {
        vec2 uv = (gl_FragCoord.xy * 2.0 - resolution.xy) / min(resolution.x, resolution.y);
        float field = 0.0;
        for (int i = 0; i < 5; i++) {
          float fi = float(i);
          vec2 c = vec2(sin(time * .31 + fi * 1.7), cos(time * .23 + fi * 1.3)) * (0.25 + fi * .09);
          field += 0.018 / abs(length(uv - c) - (0.18 + sin(time + fi) * .035));
        }
        vec3 base = vec3(0.015, 0.025, 0.055);
        vec3 glow = vec3(0.18, 0.72, 0.95) * field;
        vec3 warm = vec3(0.95, 0.55, 0.18) * smoothstep(0.35, 0.9, field);
        gl_FragColor = vec4(base + glow + warm * .28, 1.0);
      }
    \`;
    function shader(type, source) {
      const s = gl.createShader(type);
      gl.shaderSource(s, source);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
      return s;
    }
    const program = gl.createProgram();
    gl.attachShader(program, shader(gl.VERTEX_SHADER, vertex));
    gl.attachShader(program, shader(gl.FRAGMENT_SHADER, fragment));
    gl.linkProgram(program);
    gl.useProgram(program);
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1, 1,-1, -1,1, -1,1, 1,-1, 1,1]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(program, 'p');
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    const resolutionLoc = gl.getUniformLocation(program, 'resolution');
    const timeLoc = gl.getUniformLocation(program, 'time');
    const resize = () => {
      const dpr = Math.min(devicePixelRatio || 1, 2);
      canvas.width = Math.floor(innerWidth * dpr);
      canvas.height = Math.floor(innerHeight * dpr);
      gl.viewport(0, 0, canvas.width, canvas.height);
    };
    addEventListener('resize', resize);
    resize();
    let t = 0;
    const frame = () => {
      t += 0.016;
      gl.uniform2f(resolutionLoc, canvas.width, canvas.height);
      gl.uniform1f(timeLoc, t);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
      requestAnimationFrame(frame);
    };
    frame();
  </script>
</body>
</html>`;

type MindRuntimeError = {
  message: string;
  filename?: string;
  line?: number;
  column?: number;
};

const MIND_RUNTIME_MESSAGE_SOURCE = "big-head-mind-runtime";

export function MindView({
  panel,
  onEvent,
}: {
  panel?: PanelState;
  onEvent?: (data: unknown) => void;
}) {
  const [visualPanePct, setVisualPanePct] = usePersistentNumber(
    "big-head:view:mind-visual-pct",
    48,
  );
  const [runtimeError, setRuntimeError] = useState<MindRuntimeError | null>(
    null,
  );
  const bodyRef = useRef<HTMLDivElement>(null);
  const frameRef = useRef<HTMLIFrameElement>(null);
  const reportedErrorRef = useRef("");
  const onEventRef = useRef(onEvent);
  const htmlContent =
    panel?.content.kind === "html" ? panel.content : undefined;
  const rawRuntime = htmlContent
    ? allowsRawMindRuntime(htmlContent.markup)
    : false;
  const visualMarkup =
    rawRuntime && htmlContent
      ? hardenRawRuntime(htmlContent.markup)
      : DEFAULT_MIND_HTML;
  const readoutMarkup = mindReadoutMarkup(panel, rawRuntime);
  const handleReadoutResize = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const container = bodyRef.current;
      if (!container) return;
      event.preventDefault();
      dragRatio(event, container, "y", 30, 75, setVisualPanePct);
    },
    [setVisualPanePct],
  );

  useEffect(() => {
    setRuntimeError(null);
    reportedErrorRef.current = "";
  }, [visualMarkup]);

  useEffect(() => {
    onEventRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    const receiveRuntimeMessage = (event: MessageEvent) => {
      if (event.source !== frameRef.current?.contentWindow) return;
      const data = event.data;
      if (
        !data ||
        data.source !== MIND_RUNTIME_MESSAGE_SOURCE ||
        data.type !== "error"
      )
        return;

      const error: MindRuntimeError = {
        message: String(data.message || "Unknown runtime error"),
        filename: typeof data.filename === "string" ? data.filename : undefined,
        line: typeof data.line === "number" ? data.line : undefined,
        column: typeof data.column === "number" ? data.column : undefined,
      };
      setRuntimeError(error);

      const signature = `${error.message}:${error.line ?? 0}:${error.column ?? 0}`;
      if (reportedErrorRef.current === signature) return;
      reportedErrorRef.current = signature;
      onEventRef.current?.({
        type: "mind_runtime_error",
        ...error,
      });
    };

    window.addEventListener("message", receiveRuntimeMessage);
    return () => window.removeEventListener("message", receiveRuntimeMessage);
  }, []);

  return (
    <section className="mind-shell">
      <div className="mind-header">
        <div>
          <div className="mind-eyebrow">mind-view</div>
          <h2>{panel?.title ?? "Agent programmable WebGL surface"}</h2>
        </div>
        <div className="mind-status">
          {rawRuntime ? "runtime" : panel ? "safe html" : "waiting"}
        </div>
      </div>
      <div
        ref={bodyRef}
        className={rawRuntime ? "mind-body mind-body-raw" : "mind-body"}
        style={
          {
            "--mind-visual": `${rawRuntime ? 100 : visualPanePct}%`,
          } as CSSProperties
        }
      >
        <iframe
          ref={frameRef}
          key={`${panel?.id ?? "mind-default"}-${rawRuntime ? "raw" : "default"}`}
          className="mind-runtime-frame"
          title="mind-view runtime"
          srcDoc={visualMarkup}
          sandbox="allow-scripts allow-forms allow-popups allow-modals"
        />
        {rawRuntime && runtimeError && (
          <div className="mind-runtime-error" role="alert">
            <div className="mind-runtime-error-label">Runtime error</div>
            <div className="mind-runtime-error-message">
              {runtimeError.message}
            </div>
            {(runtimeError.line || runtimeError.filename) && (
              <div className="mind-runtime-error-location">
                {runtimeError.filename || "inline script"}
                {runtimeError.line ? `:${runtimeError.line}` : ""}
                {runtimeError.column ? `:${runtimeError.column}` : ""}
              </div>
            )}
          </div>
        )}
        {!rawRuntime && (
          <div
            className="split-resizer split-resizer-horizontal split-resizer-subtle"
            role="separator"
            aria-label="Resize Mind View runtime and readable layer"
            aria-orientation="horizontal"
            onPointerDown={handleReadoutResize}
          />
        )}
        {!rawRuntime && (
          <div className="mind-readout">
            <div className="mind-readout-kicker">readable layer</div>
            <div
              className="mind-readout-content"
              dangerouslySetInnerHTML={{ __html: readoutMarkup }}
            />
          </div>
        )}
      </div>
    </section>
  );
}

function mindReadoutMarkup(panel?: PanelState, rawRuntime = false): string {
  if (
    panel?.content.kind !== "html" ||
    panel.content.markup.trim().length === 0
  ) {
    return "<p>Waiting for agent-rendered WebGL, Three.js, or shader content.</p>";
  }

  const body = normalizeStaticBody(extractBody(panel.content.markup));
  if (body.trim().length === 0) {
    return rawRuntime
      ? "<p>Raw visual runtime is active. Put long prose, tables, and controls into a normal html2d panel so they stay readable.</p>"
      : "<p>Mind View received HTML, but no readable body content was found.</p>";
  }
  return body;
}

function allowsRawMindRuntime(markup: string): boolean {
  const lower = markup.toLowerCase();
  return (
    lower.includes('data-big-head-mind-runtime="raw"') ||
    lower.includes("data-big-head-mind-runtime='raw'") ||
    lower.includes('name="big-head-mind-runtime" content="raw"') ||
    lower.includes("name='big-head-mind-runtime' content='raw'")
  );
}

function hardenRawRuntime(markup: string): string {
  const runtimeDiagnostics = `<script id="big-head-runtime-diagnostics">
    (function () {
      function report(message, filename, line, column) {
        parent.postMessage({
          source: '${MIND_RUNTIME_MESSAGE_SOURCE}',
          type: 'error',
          message: String(message || 'Unknown runtime error'),
          filename: filename || '',
          line: Number(line) || 0,
          column: Number(column) || 0
        }, '*');
      }
      window.addEventListener('error', function (event) {
        report(event.message, event.filename, event.lineno, event.colno);
      });
      window.addEventListener('unhandledrejection', function (event) {
        var reason = event.reason;
        report(reason && reason.message ? reason.message : reason, '', 0, 0);
      });
    })();
  </script>`;
  const runtimeCss = `<style id="big-head-runtime-guard">
    html, body {
      margin: 0 !important;
      width: 100% !important;
      height: 100% !important;
      overflow: hidden !important;
      background: #05070d !important;
    }
    *, *::before, *::after { box-sizing: border-box; }
    canvas, video, svg {
      max-width: 100% !important;
      max-height: 100% !important;
    }
  </style>`;
  const runtimeHead = `${runtimeCss}${runtimeDiagnostics}`;

  if (/<head[\s\S]*?>/i.test(markup)) {
    return markup.replace(/<head([\s\S]*?)>/i, `<head$1>${runtimeHead}`);
  }
  if (/<html[\s\S]*?>/i.test(markup)) {
    return markup.replace(
      /<html([\s\S]*?)>/i,
      `<html$1><head>${runtimeHead}</head>`,
    );
  }
  return `<!doctype html><html><head><meta charset="utf-8" />${runtimeHead}</head><body>${markup}</body></html>`;
}

function extractBody(markup: string): string {
  const match = markup.match(/<body[^>]*>([\s\S]*?)<\/body>/i);
  if (match) {
    return match[1];
  }
  return markup
    .replace(/<!doctype[^>]*>/gi, "")
    .replace(/<\/?html[^>]*>/gi, "")
    .replace(/<head[\s\S]*?<\/head>/gi, "");
}

function normalizeStaticBody(markup: string): string {
  return markup
    .replace(/<script[\s\S]*?<\/script>/gi, "")
    .replace(/<style[\s\S]*?<\/style>/gi, "")
    .replace(/<link[\s\S]*?>/gi, "")
    .replace(/<iframe[\s\S]*?<\/iframe>/gi, "")
    .replace(/<object[\s\S]*?<\/object>/gi, "")
    .replace(/<embed[\s\S]*?>/gi, "")
    .replace(/<canvas[\s\S]*?<\/canvas>/gi, "")
    .replace(/<canvas[^>]*>/gi, "")
    .replace(/<svg[\s\S]*?<\/svg>/gi, "")
    .replace(/\sstyle=(?:"[^"]*"|'[^']*')/gi, "")
    .replace(/\son[a-z]+=(?:"[^"]*"|'[^']*')/gi, "")
    .replace(/\s(?:href|src)=(["'])\s*javascript:[\s\S]*?\1/gi, "");
}
