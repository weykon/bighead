// 3D scene with floating panels.
// Uses @react-three/drei's <Html> to embed DOM panels at 3D positions.
// Supports drag to reposition panels (sends drag_end event via WebSocket).
// Camera control: hold right-mouse to enter FPS fly mode (WASD + mouse look).
// Hold Alt/Shift/Ctrl for legacy orbit-camera mode.

import { Html, OrbitControls } from '@react-three/drei';
import { useEffect, useRef, useState, useCallback } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import * as THREE from 'three';
import type { PanelState } from './types';
import { PanelBody } from './panels';
import { sendMove } from './ws';

export function Scene3D({
  panels,
  onPanelMove,
  onPanelEvent,
  onCameraModeChange,
}: {
  panels: Map<string, PanelState>;
  onPanelMove?: (
    id: string,
    position: [number, number, number],
    rotation?: [number, number, number],
  ) => void;
  onPanelEvent?: (panel_id: string, data: unknown) => void;
  onCameraModeChange?: (mode: 'idle' | 'fly' | 'orbit') => void;
}) {
  // Camera mode:
  //   'idle'  — passive; OrbitControls is on and idle, panel interactions normal
  //   'fly'   — right-mouse held; pointer locked, WASD + mouse look; OrbitControls off
  //   'orbit' — Alt/Shift/Ctrl held; OrbitControls on (legacy behavior)
  const [cameraMode, setCameraModeRaw] = useState<'idle' | 'fly' | 'orbit'>('idle');
  const setCameraMode = useCallback(
    (updater: 'idle' | 'fly' | 'orbit' | ((m: 'idle' | 'fly' | 'orbit') => 'idle' | 'fly' | 'orbit')) => {
      setCameraModeRaw((prev) => {
        const next = typeof updater === 'function' ? updater(prev) : updater;
        if (next !== prev) onCameraModeChange?.(next);
        return next;
      });
    },
    [onCameraModeChange],
  );
  const flyState = useRef({
    yaw: 0,
    pitch: 0,
    keys: new Set<string>(),
  });

  const onPointerLockChange = useCallback(() => {
    if (document.pointerLockElement === null) {
      // Lock was released (Esc, window blur, browser denied the request, etc.).
      // Drop fly mode and clear any keys that were held so we don't keep
      // drifting after exit.
      flyState.current.keys.clear();
      setCameraMode((m) => (m === 'fly' ? 'idle' : m));
    } else {
      // Lock acquired: enter fly mode. (We don't enter on mousedown itself
      // because requestPointerLock is async and the browser may reject it
      // — only this callback confirms we actually have the lock.)
      setCameraMode('fly');
    }
  }, []);

  // Right-mouse-button → request pointer lock → enter fly mode.
  // Left mouse is reserved for panel drag and OrbitControls drag, so it
  // must not also trigger fly mode.
  useEffect(() => {
    const canvas = document.querySelector('canvas');
    if (!canvas) return;

    const handleContextMenu = (e: MouseEvent) => {
      e.preventDefault();
    };
    const handleMouseDown = (e: MouseEvent) => {
      if (e.button !== 2) return; // right click only
      // If a panel is being interacted with, let that take precedence.
      const target = e.target as HTMLElement | null;
      if (target?.closest('.panel-drag-handle') || target?.closest('button, input, select, textarea, a')) {
        return;
      }
      e.preventDefault();
      canvas.requestPointerLock();
    };

    canvas.addEventListener('contextmenu', handleContextMenu);
    canvas.addEventListener('mousedown', handleMouseDown);
    document.addEventListener('pointerlockchange', onPointerLockChange);
    return () => {
      canvas.removeEventListener('contextmenu', handleContextMenu);
      canvas.removeEventListener('mousedown', handleMouseDown);
      document.removeEventListener('pointerlockchange', onPointerLockChange);
    };
  }, [onPointerLockChange]);

  // Mouse-look: accumulate yaw/pitch from pointer-lock movement events.
  // Sensitivities tuned for ~1080p / standard mouse DPI; tweak via constants
  // if it feels off.
  useEffect(() => {
    const LOOK_SENSITIVITY = 0.0025;
    const handleMouseMove = (e: MouseEvent) => {
      if (document.pointerLockElement === null) return;
      flyState.current.yaw -= e.movementX * LOOK_SENSITIVITY;
      flyState.current.pitch -= e.movementY * LOOK_SENSITIVITY;
      // Clamp pitch so the camera can't roll over backwards.
      const maxPitch = Math.PI / 2 - 0.01;
      flyState.current.pitch = Math.max(-maxPitch, Math.min(maxPitch, flyState.current.pitch));
    };
    document.addEventListener('mousemove', handleMouseMove);
    return () => document.removeEventListener('mousemove', handleMouseMove);
  }, []);

  // WASD + Space/Shift: track held keys. Don't preventDefault on Tab so the
  // browser can still switch focus if needed; only the movement keys are
  // actively consumed.
  useEffect(() => {
    const MOVEMENT_KEYS = new Set(['w', 'a', 's', 'd', 'q', 'e', ' ', 'shift', 'arrowup', 'arrowdown', 'arrowleft', 'arrowright']);
    const isInputFocused = () => {
      const el = document.activeElement as HTMLElement | null;
      if (!el) return false;
      const tag = el.tagName;
      return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (isInputFocused()) return;
      const k = e.key.toLowerCase();
      // F toggles fly mode. This is a keyboard-only path that does not
      // require pointer lock — it lets the user enter fly mode even when
      // right-click can't acquire the lock (e.g. some browser policies),
      // and makes the feature discoverable without reading the hint pill.
      // We still request pointer lock so mouse-look works; if the request
      // fails, fly mode is still active and the user can use W/A/S/D, just
      // without free mouse-look.
      if (k === 'f' && !e.altKey && !e.ctrlKey && !e.metaKey) {
        e.preventDefault();
        // Toggle via the live pointerLockElement + functional updater so
        // we don't read a stale `cameraMode` from the listener's closure.
        const locked = document.pointerLockElement !== null;
        if (locked) {
          document.exitPointerLock();
        } else {
          document.querySelector('canvas')?.requestPointerLock();
        }
        // Flip mode to fly (entry) or queue the exit. Exit will also be
        // confirmed by the pointerlockchange handler when the lock
        // actually releases.
        setCameraMode((m) => (m === 'fly' ? 'idle' : 'fly'));
        return;
      }
      // Esc also exits fly mode. The browser auto-releases pointer lock on
      // Esc, but a second explicit path makes the exit feel immediate and
      // covers any case where pointerlockchange is delayed or suppressed.
      // Use the live pointerLockElement state to decide (not the closed-over
      // `cameraMode`, which is stale until React re-renders the listener).
      if (k === 'escape') {
        const isLocked = document.pointerLockElement !== null;
        if (isLocked) document.exitPointerLock();
        // We don't read cameraMode here — just clear fly state and let
        // pointerlockchange (or the next render) reconcile.
        flyState.current.keys.clear();
        setCameraMode((m) => (m === 'fly' ? 'idle' : m));
        return;
      }
      if (MOVEMENT_KEYS.has(k)) {
        flyState.current.keys.add(k);
        // Prevent the page from scrolling on Space / arrow keys.
        if (k === ' ' || k.startsWith('arrow')) e.preventDefault();
      }
      // Legacy orbit-mode trigger: Alt or Ctrl/Meta held. Shift is excluded
      // because it's used for sprinting in fly mode and would always fire.
      // (Pure Shift-held is a no-op for orbit.)
      if (e.altKey || e.ctrlKey || e.metaKey) {
        setCameraMode((m) => (m === 'fly' ? m : 'orbit'));
      }
    };
    const handleKeyUp = (e: KeyboardEvent) => {
      const k = e.key.toLowerCase();
      flyState.current.keys.delete(k);
      // Exit orbit only when every modifier is released. Re-check all four
      // because a keyup for one modifier may leave others held.
      if (!e.altKey && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
        setCameraMode((m) => (m === 'orbit' ? 'idle' : m));
      }
    };
    const handleBlur = () => {
      flyState.current.keys.clear();
    };
    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('keyup', handleKeyUp);
    window.addEventListener('blur', handleBlur);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('keyup', handleKeyUp);
      window.removeEventListener('blur', handleBlur);
    };
  }, []);

  // Re-render the camera each frame from the fly state.
  function FlyCamera() {
    const { camera } = useThree();
    useEffect(() => {
      // Initialize yaw/pitch from current camera orientation so the first
      // frame of fly mode doesn't snap the user somewhere unexpected.
      const euler = new THREE.Euler().setFromQuaternion(camera.quaternion, 'YXZ');
      flyState.current.yaw = euler.y;
      flyState.current.pitch = euler.x;
    }, [camera]);
    useFrame((_, dtRaw) => {
      if (cameraMode !== 'fly') return;
      const dt = Math.min(dtRaw, 0.1); // clamp huge frame deltas (tab switch etc.)
      const keys = flyState.current.keys;
      const isSprint = keys.has('shift');
      const speed = (isSprint ? 12 : 5) * dt;
      // Forward/strafe vectors derived from yaw only (ignore pitch so
      // up/down look doesn't make W move up/down — that's what Q/E are for).
      const forward = new THREE.Vector3(-Math.sin(flyState.current.yaw), 0, -Math.cos(flyState.current.yaw));
      const right = new THREE.Vector3(Math.cos(flyState.current.yaw), 0, -Math.sin(flyState.current.yaw));
      const move = new THREE.Vector3();
      if (keys.has('w')) move.add(forward);
      if (keys.has('s')) move.sub(forward);
      if (keys.has('d')) move.add(right);
      if (keys.has('a')) move.sub(right);
      if (keys.has('e') || keys.has(' ')) move.y += 1;
      if (keys.has('q')) move.y -= 1;
      if (move.lengthSq() > 0) move.normalize().multiplyScalar(speed);
      camera.position.add(move);
      // Apply yaw/pitch via YXZ Euler so roll stays 0.
      const euler = new THREE.Euler(flyState.current.pitch, flyState.current.yaw, 0, 'YXZ');
      camera.quaternion.setFromEuler(euler);
    });
    return null;
  }

  return (
    <>
      <color attach="background" args={['#0a0e1a']} />
      <ambientLight intensity={0.4} />
      <directionalLight position={[5, 10, 5]} intensity={0.8} />
      {/* Grid pushed far down so it never sits behind panels in screen-space. */}
      <gridHelper args={[40, 40, '#0f172a', '#0f172a']} position={[0, -8, 0]} />
      <axesHelper args={[1.5]} />

      <FlyCamera />

      {cameraMode !== 'fly' && (
        <OrbitControls
          // Unmount in fly mode so its internal damping/target state doesn't
          // fight FlyCamera for control of the camera transform. When we
          // re-mount in orbit mode, the controls pick up the camera's
          // current position as the new orbit target.
          key={cameraMode}
          enableDamping
          dampingFactor={0.08}
          minDistance={2}
          maxDistance={50}
        />
      )}

      {/* Collect all panel refs for occlusion */}
      {Array.from(panels.values()).map((p) => (
        <PanelMesh
          key={p.id}
          panel={p}
          onPanelMove={onPanelMove}
          onPanelEvent={onPanelEvent}
        />
      ))}

      {/* Crosshair + mode hint overlay. Lives inside Canvas so it can use
          drei <Html> with portal disabled (default fullscreen overlay). */}
      {cameraMode === 'fly' && <FlyHUD />}
    </>
  );
}

// Crosshair + WASD hint rendered to the screen. Uses drei <Html> in default
// (non-transform) mode so it's a normal 2D overlay anchored to the camera's
// viewport, not floating in 3D space.
function FlyHUD() {
  return (
    <Html
      // No `transform` → positioned in screen space by calculatePosition.
      style={{ pointerEvents: 'none' }}
      zIndexRange={[1000, 0]}
    >
      <div
        style={{
          position: 'absolute',
          top: 0,
          left: 0,
          width: '100%',
          height: '100%',
        }}
      >
        {/* Crosshair */}
        <div
          style={{
            position: 'absolute',
            top: '50%',
            left: '50%',
            width: 12,
            height: 12,
            transform: 'translate(-50%, -50%)',
            pointerEvents: 'none',
          }}
        >
          <div
            style={{
              position: 'absolute',
              top: '50%',
              left: 0,
              width: '100%',
              height: 1,
              background: 'rgba(34, 211, 238, 0.7)',
              transform: 'translateY(-50%)',
            }}
          />
          <div
            style={{
              position: 'absolute',
              left: '50%',
              top: 0,
              width: 1,
              height: '100%',
              background: 'rgba(34, 211, 238, 0.7)',
              transform: 'translateX(-50%)',
            }}
          />
        </div>
        {/* Hint pill */}
        <div
          style={{
            position: 'absolute',
            top: 12,
            left: '50%',
            transform: 'translateX(-50%)',
            background: 'rgba(15, 23, 42, 0.85)',
            border: '1px solid rgba(34, 211, 238, 0.5)',
            color: '#e2e8f0',
            padding: '6px 12px',
            borderRadius: 6,
            fontSize: 11,
            fontFamily: 'ui-sans-serif, system-ui, sans-serif',
            whiteSpace: 'nowrap',
          }}
        >
          FLY · WASD move · Q/E down/up · Shift sprint · F or Esc to exit
        </div>
      </div>
    </Html>
  );
}

function PanelMesh({
  panel,
  onPanelMove,
  onPanelEvent,
}: {
  panel: PanelState;
  onPanelMove?: (
    id: string,
    position: [number, number, number],
    rotation?: [number, number, number],
  ) => void;
  onPanelEvent?: (panel_id: string, data: unknown) => void;
}) {
  const { camera } = useThree();
  const groupRef = useRef<THREE.Group>(null);
  const [isDragging, setIsDragging] = useState(false);
  const [isHovered, setIsHovered] = useState(false);
  const dragStartRef = useRef<{ mouse: THREE.Vector2; panelPos: THREE.Vector3 } | null>(null);

  // While dragging the title bar, ditch document-wide text selection so the
  // body of the panel (or anything else under the cursor) doesn't get
  // accidentally selected mid-drag. Restore on drag end so users can still
  // copy values when not dragging.
  useEffect(() => {
    if (!isDragging) return;
    const prev = document.body.style.userSelect;
    document.body.style.userSelect = 'none';
    return () => {
      document.body.style.userSelect = prev;
    };
  }, [isDragging]);

  // Highlight pulse animation
  useFrame((_, dt) => {
    if (!groupRef.current) return;
    const now = Date.now();
    if (panel.highlighted && panel.highlightUntil && now < panel.highlightUntil) {
      const pulse = 0.5 + 0.5 * Math.sin(now * 0.01);
      const s = 1 + 0.1 * pulse;
      groupRef.current.scale.set(s, s, s);
    } else {
      const s = panel.scale[0];
      groupRef.current.scale.lerp(new THREE.Vector3(s, s, s), 0.1);
    }
  });

  // Update position when panel.position changes (from external move command)
  useEffect(() => {
    if (groupRef.current && !isDragging) {
      groupRef.current.position.set(...panel.position);
    }
  }, [panel.position, isDragging]);

  const handlePointerDown = (e: React.PointerEvent) => {
    // Only drag on title bar
    if ((e.target as HTMLElement).closest('.panel-drag-handle')) {
      e.stopPropagation();
      // Prevent the browser from starting a native text selection on the
      // initial click — without this the body content under the cursor can
      // get highlighted as the drag travels through it.
      e.preventDefault();
      setIsDragging(true);
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
      if (groupRef.current) {
        dragStartRef.current = {
          mouse: new THREE.Vector2(e.clientX, e.clientY),
          panelPos: groupRef.current.position.clone(),
        };
      }
    }
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    if (isDragging && dragStartRef.current && groupRef.current) {
      const deltaX = (e.clientX - dragStartRef.current.mouse.x) * 0.01;
      const deltaY = (e.clientY - dragStartRef.current.mouse.y) * -0.01;
      groupRef.current.position.x = dragStartRef.current.panelPos.x + deltaX;
      groupRef.current.position.y = dragStartRef.current.panelPos.y + deltaY;
    }
  };

  const handlePointerUp = (e: React.PointerEvent) => {
    if (isDragging && dragStartRef.current) {
      setIsDragging(false);
      (e.target as HTMLElement).releasePointerCapture(e.pointerId);
      if (groupRef.current) {
        const newPos: [number, number, number] = [
          groupRef.current.position.x,
          groupRef.current.position.y,
          groupRef.current.position.z,
        ];
        // If the cursor barely moved, treat this as a click (or the first
        // half of a double-click) and don't sync — both the dispatch and
        // the WebSocket send would just be noise. The threshold is tiny
        // (1 cm-equivalent in world units).
        const dx = newPos[0] - dragStartRef.current.panelPos.x;
        const dy = newPos[1] - dragStartRef.current.panelPos.y;
        const dz = newPos[2] - dragStartRef.current.panelPos.z;
        if (Math.hypot(dx, dy, dz) > 0.01) {
          // Optimistically commit the new position to parent state first, so
          // the useEffect that watches [panel.position, isDragging] sees the
          // dragged value (not the stale one) when isDragging flips to false.
          // Otherwise the panel would snap back until the server's Move
          // command round-trips.
          onPanelMove?.(panel.id, newPos);
          // Send position update via WebSocket so server snapshot + other
          // clients stay in sync.
          sendMove(panel.id, newPos);
          console.log('[PanelMesh] drag ended:', panel.id, newPos);
        }
      }
      dragStartRef.current = null;
    }
  };

  // Double-clicking the title bar drops the panel directly in front of the
  // current camera, sized and oriented as a billboard. Distance is tuned
  // against `distanceFactor={6}` so the panel reads at a comfortable size.
  const handleDoubleClick = useCallback(
    (e: React.MouseEvent) => {
      if (!(e.target as HTMLElement).closest('.panel-drag-handle')) return;
      e.stopPropagation();
      const forward = new THREE.Vector3();
      camera.getWorldDirection(forward);
      const camPos = new THREE.Vector3();
      camera.getWorldPosition(camPos);
      const distance = 5;
      const newPos: [number, number, number] = [
        camPos.x + forward.x * distance,
        camPos.y + forward.y * distance,
        camPos.z + forward.z * distance,
      ];
      // Copy camera world rotation so the panel sits parallel to the screen
      // plane with its visible face toward the camera.
      const camQuat = new THREE.Quaternion();
      camera.getWorldQuaternion(camQuat);
      const euler = new THREE.Euler().setFromQuaternion(camQuat);
      const newRot: [number, number, number] = [euler.x, euler.y, euler.z];
      onPanelMove?.(panel.id, newPos, newRot);
      // Server only persists position over the wire today; rotation stays
      // local for the current session.
      sendMove(panel.id, newPos);
    },
    [camera, onPanelMove, panel.id],
  );

  // Calculate depth-based visual effects
  // Z > 0 = closer (fully opaque), Z < 0 = farther (slightly recede).
  // Sign must be + so closer panels read more clearly, not less.
  const depthZ = panel.position[2];
  const depthOpacity = Math.max(0.7, Math.min(1, 1 + (depthZ * 0.05)));

  return (
    <group
      ref={groupRef}
      position={panel.position}
      rotation={panel.rotation ?? [0, 0, 0]}
    >
      <Html
        transform
        sprite
        distanceFactor={6}
        // zIndexRange defaults to [16777271, 0], which maps camera distance
        // to a per-millimeter zIndex delta of ~17. That value gets re-applied
        // to the outer wrapper every frame (see Html.js:247), so a slowly
        // orbiting camera causes constant zIndex churn — and when two panels
        // round to the same zIndex, the browser is free to flip their paint
        // order each frame → visible flicker. Pin the range so the per-frame
        // zIndex actually changes only when distance changes by a decimeter
        // or more, decoupling paint order from camera micro-motion.
        zIndexRange={[100, 0]}
        style={{
          // Inner-div styling only: drei controls the outer wrapper's
          // zIndex every frame from camera distance, so setting zIndex
          // here would be a no-op.
          transition: 'opacity 0.3s ease',
          opacity: depthOpacity,
        }}
      >
        <div
          className={`pointer-events-auto ${isDragging ? 'cursor-grabbing' : 'cursor-grab'}`}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerUp}
          onDoubleClick={handleDoubleClick}
          onPointerEnter={() => setIsHovered(true)}
          onPointerLeave={() => !isDragging && setIsHovered(false)}
        >
          <PanelCard
            panel={panel}
            isHovered={isHovered}
            isDragging={isDragging}
            onEvent={(data) => onPanelEvent?.(panel.id, data)}
          />
        </div>
      </Html>
    </group>
  );
}

function PanelCard({
  panel,
  isHovered,
  isDragging,
  onEvent,
}: {
  panel: PanelState;
  isHovered: boolean;
  isDragging: boolean;
  onEvent?: (data: unknown) => void;
}) {
  return (
    <div
      className={`panel-card-3d ${
        isDragging ? 'panel-card-3d-dragging' :
        isHovered ? 'panel-card-3d-hovered' : ''
      }`}
      style={
        panel.highlighted && panel.highlightUntil && Date.now() < panel.highlightUntil
          ? { boxShadow: `0 0 24px ${highlightColor(panel.highlighted)}` }
          : undefined
      }
    >
      {/* Drag handle / title bar */}
      <div className="panel-drag-handle panel-card-titlebar">
        {/* Drag indicator */}
        <div className="flex flex-col gap-0.5 opacity-40">
          <div className="flex gap-0.5">
            <div className="w-1 h-1 rounded-full bg-slate-400"></div>
            <div className="w-1 h-1 rounded-full bg-slate-400"></div>
          </div>
          <div className="flex gap-0.5">
            <div className="w-1 h-1 rounded-full bg-slate-400"></div>
            <div className="w-1 h-1 rounded-full bg-slate-400"></div>
          </div>
        </div>
        <div className="panel-card-title">
          {panel.title}
        </div>
        {isDragging && (
          <div className="text-[10px] text-cyan-400 animate-pulse">moving...</div>
        )}
      </div>
      
      <div className="panel-card-body">
        <PanelBody
          kind={panel.kind}
          content={panel.content}
          onEvent={onEvent}
        />
      </div>
    </div>
  );
}

function highlightColor(e: NonNullable<PanelState['highlighted']>): string {
  if (e.type === 'pulse') return e.color;
  if (e.type === 'outline') return e.color;
  return '#ffffff';
}
