import { useEffect, useRef } from 'react';
import { Canvas, useFrame, useThree } from '@react-three/fiber';
import { Html, OrbitControls, RoundedBox, ContactShadows } from '@react-three/drei';
import { Group, OrthographicCamera } from 'three';
import { ArrowRight, BookOpen, MessageCircle, X } from 'lucide-react';

export type WorkEvidence = { id: string; source: string; quote: string; digest: string; supports_completion: boolean };
export type WorkItem = {
  entity_id: number; record: { id: string; title: string; summary: string; status: string; revision: number; evidence_ids: string[] };
  status: string; appearance?: { x: number; z: number; height: number; style: string; color: string; glow: number; revision: number };
  evidence: WorkEvidence[];
};
export type WorkProjection = {
  initialized: boolean; records: WorkItem[]; connections: { from: string; to: string; label: string }[];
  events: { id: string; target_id: string; kind: string; sequence: number; status?: string }[];
  triggers: { entity_id: number; trigger: { target_id: string; runs: number; enabled: boolean; error: string } }[];
  warnings: string[];
};
export const workStatus = (s: string) => ({ completed: '已完成', in_progress: '进行中', blocked: '待解决', idea: '想法', archived: '已归档', unverified: '待核实' }[s] ?? '待核实');

function Building({ item, index, selected, enter }: { item: WorkItem; index: number; selected: boolean; enter: () => void }) {
  const flag = useRef<Group>(null);
  useFrame(({ clock }) => { if (flag.current) flag.current.rotation.y = Math.sin(clock.elapsedTime * 1.6) * 0.18; });
  const a = item.appearance!;
  const h = a.height;
  const complete = item.status === 'completed';
  const blocked = item.status === 'blocked';
  const color = a.color;
  const material = <meshStandardMaterial color={color} emissive={color} emissiveIntensity={a.glow * 0.35} roughness={0.7} />;
  return <group position={[a.x, 0.12, a.z]} onClick={e => { e.stopPropagation(); enter(); }}>
    <RoundedBox args={[1.5, 0.16, 1.45]} radius={0.05} receiveShadow><meshStandardMaterial color={selected ? '#efd585' : '#e2e9e6'} /></RoundedBox>
    {a.style === 'garden' ? <>
      <mesh position={[0, 0.25, 0]}><cylinderGeometry args={[0.54, 0.62, 0.35, 16]} />{material}</mesh>
      <mesh position={[0, 0.65, 0]}><icosahedronGeometry args={[0.48, 1]} /><meshStandardMaterial color="#73ae83" /></mesh>
    </> : <>
      <RoundedBox args={[1.06, h, 0.94]} radius={0.035} position={[0, h / 2 + 0.1, 0]} castShadow receiveShadow>{material}</RoundedBox>
      {a.style === 'lab' ? <mesh position={[0, h + 0.11, 0]} castShadow><sphereGeometry args={[0.57, 24, 12, 0, Math.PI * 2, 0, Math.PI / 2]} /><meshStandardMaterial color="#cbdde6" metalness={0.25} roughness={0.3} /></mesh> : a.style === 'workshop' ? <mesh position={[0, h + 0.25, 0]} rotation={[0, Math.PI / 4, 0]} castShadow><coneGeometry args={[0.87, 0.38, 4]} /><meshStandardMaterial color="#546f79" /></mesh> : <RoundedBox args={[1.2, 0.13, 1.08]} radius={0.03} position={[0, h + 0.14, 0]} castShadow><meshStandardMaterial color="#f6f7ec" /></RoundedBox>}
      {[0.3, 0.7].filter(y => y < h).flatMap(y => [-0.3, 0.3].map(x => <mesh key={`${x}:${y}`} position={[x, y + 0.14, 0.48]}><boxGeometry args={[0.19, 0.18, 0.035]} /><meshStandardMaterial color={complete ? '#ffe9a5' : '#617f88'} emissive={complete ? '#edca72' : color} emissiveIntensity={complete ? 0.22 + a.glow : a.glow * 0.4} /></mesh>))}
      <mesh position={[0, 0.3, 0.49]}><boxGeometry args={[0.22, 0.42, 0.04]} /><meshStandardMaterial color="#3d6264" /></mesh>
      {a.style === 'archive' && [-0.48, 0.48].map(x => <mesh key={x} position={[x, h / 2, 0.54]}><cylinderGeometry args={[0.045, 0.045, h, 8]} /><meshStandardMaterial color="#f3f5e8" /></mesh>)}
      {a.style === 'tower' && <mesh position={[0.27, h + 0.46, 0]}><boxGeometry args={[0.3, 0.6, 0.34]} />{material}</mesh>}
    </>}
    {!complete && <group position={[0, 0.28, 0.75]}>
      <mesh><boxGeometry args={[1.12, 0.1, 0.06]} /><meshStandardMaterial color={blocked ? '#d97982' : '#e2b752'} /></mesh>
      {[-0.48, 0.48].map(x => <mesh key={x} position={[x, -0.05, 0]}><boxGeometry args={[0.04, 0.3, 0.06]} /><meshStandardMaterial color="#829b99" /></mesh>)}
    </group>}
    <group ref={flag} position={[-0.46, h + 0.28, 0]}>
      <mesh position={[0, 0.2, 0]}><cylinderGeometry args={[0.012, 0.012, 0.6, 6]} /><meshStandardMaterial color="#61777d" /></mesh>
      <mesh position={[0.13, 0.37, 0]}><boxGeometry args={[0.26, 0.14, 0.015]} /><meshStandardMaterial color={blocked ? '#d97982' : complete ? '#6fae94' : '#e9c963'} /></mesh>
    </group>
    <Html position={[0, h + 0.95, 0]} center zIndexRange={[3, 0]}>
      <button className={'city-label ' + (selected ? 'selected' : '')} aria-label={'进入建筑：' + item.record.title} onClick={enter}><span>{String(index + 1).padStart(2, '0')}</span><strong>{item.record.title}</strong></button>
    </Html>
  </group>;
}

function CityCamera() {
  const { camera, size } = useThree();
  useEffect(() => { const c = camera as OrthographicCamera; c.zoom = Math.min(size.width / 14, size.height / 10); c.updateProjectionMatrix(); }, [camera, size.width, size.height]);
  return <OrbitControls target={[0, 0, 0]} enablePan={false} minZoom={12} maxZoom={130} minPolarAngle={0.45} maxPolarAngle={1.2} />;
}

export function WorkCity({ work, selected, enter }: { work: WorkProjection; selected: string; enter: (id: string) => void }) {
  return <Canvas orthographic shadows camera={{ position: [9, 10, 12], zoom: 48 }} gl={{ preserveDrawingBuffer: true }}>
    <color attach="background" args={['#e8f1ef']} /><ambientLight intensity={1.6} /><directionalLight position={[-4, 10, 6]} intensity={2.3} castShadow shadow-mapSize={[2048, 2048]} />
    <mesh rotation={[-Math.PI / 2, 0, 0]} position={[0, -0.17, 0]} receiveShadow><planeGeometry args={[200, 200]} /><meshStandardMaterial color="#e8f1ef" /></mesh>
    <RoundedBox args={[12.4, 0.25, 9.5]} radius={0.1} receiveShadow><meshStandardMaterial color="#cbdcd5" /></RoundedBox>
    <mesh position={[0, 0.14, 0]} rotation={[-Math.PI / 2, 0, 0]}><planeGeometry args={[11.9, 9]} /><meshStandardMaterial color="#e2e9e2" /></mesh>
    {work.connections.map(c => {
      const a = work.records.find(r => r.record.id === c.from)?.appearance, b = work.records.find(r => r.record.id === c.to)?.appearance;
      if (!a || !b) return null;
      const dx = b.x - a.x, dz = b.z - a.z;
      return <group key={`${c.from}:${c.to}`} position={[(a.x + b.x) / 2, 0.17, (a.z + b.z) / 2]} rotation={[0, Math.atan2(dx, dz), 0]} onClick={e => { e.stopPropagation(); enter(c.to); }}>
        <mesh receiveShadow><boxGeometry args={[0.38, 0.045, Math.hypot(dx, dz)]} /><meshStandardMaterial color="#abbdbb" /></mesh>
        <mesh position={[0, 0.025, 0]}><boxGeometry args={[0.025, 0.01, Math.hypot(dx, dz)]} /><meshStandardMaterial color="#f8f6d9" /></mesh>
      </group>;
    })}
    {[[-5.6, -3.4], [-5.5, 2.7], [5.4, -3.2], [5.4, 3.6], [-3.8, 3.7]].map(([x, z], i) => <group key={i} position={[x, 0.2, z]}>
      <mesh position={[0, 0.25, 0]}><cylinderGeometry args={[0.06, 0.09, 0.5, 7]} /><meshStandardMaterial color="#86988e" /></mesh>
      <mesh position={[0, 0.65, 0]} castShadow><icosahedronGeometry args={[0.36, 1]} /><meshStandardMaterial color={i % 2 ? '#8caf92' : '#83b5ad'} /></mesh>
    </group>)}
    {work.records.filter(r => r.appearance).map((r, index) => <Building key={r.record.id} item={r} index={index} selected={selected === r.record.id} enter={() => enter(r.record.id)} />)}
    <ContactShadows opacity={0.25} scale={20} blur={2.5} far={5} position={[0, 0.16, 0]} />
    <CityCamera />
  </Canvas>;
}

export function WorkInspector({ item, work, close, discuss, enter, openBook }: { item: WorkItem; work: WorkProjection; close: () => void; discuss: () => void; enter: (id: string) => void; openBook: () => void }) {
  const links = work.connections.filter(c => c.from === item.record.id || c.to === item.record.id);
  return <section className="work-inspector" aria-label="工作详情">
    <header><div><small className={'work-status ' + item.status}>{workStatus(item.status)}</small><h3>{item.record.title}</h3></div><button title="关闭详情" aria-label="关闭工作详情" onClick={close}><X size={17} /></button></header>
    <p>{item.record.summary}</p>
    <div className="work-inspector-actions"><button onClick={discuss}><MessageCircle size={16} />接着讨论</button><button onClick={openBook}><BookOpen size={16} />翻开书页</button></div>
    <h4>记录依据</h4>
    {item.evidence.length ? item.evidence.map(e => <details key={e.id} open><summary>{e.source.startsWith('conversation:') ? '你的构想' : '项目记录'}</summary><blockquote>{e.quote}</blockquote><small className="work-source">{e.source}<br />快照 {e.digest.slice(0, 12)}</small></details>) : <p className="work-muted">尚未附上证据。</p>}
    {!!links.length && <><h4>相连的工作</h4>{links.map(c => { const id = c.from === item.record.id ? c.to : c.from; return <button className="work-link" key={`${c.from}:${c.to}`} onClick={() => enter(id)}><span>{work.records.find(r => r.record.id === id)?.record.title}<small>{c.label}</small></span><ArrowRight size={15} /></button>; })}</>}
  </section>;
}

export function WorkBook({ work, selected, enter, discuss }: { work: WorkProjection; selected: string; enter: (id: string) => void; discuss: (id: string) => void }) {
  const item = work.records.find(r => r.record.id === selected) ?? work.records[0];
  const index = work.records.findIndex(r => r.record.id === item?.record.id);
  if (!item) return null;
  return <section className="work-book" aria-label="成果之书">
    <nav aria-label="章节目录">{work.records.map((r, i) => <button key={r.record.id} aria-current={r.record.id === item.record.id ? 'page' : undefined} onClick={() => enter(r.record.id)}><span>{String(i + 1).padStart(2, '0')}</span>{r.record.title}</button>)}</nav>
    <article>
      <small className="book-chapter">BIG HEAD / WORK JOURNAL · {String(index + 1).padStart(2, '0')}</small>
      <h2>{item.record.title}</h2><span className={'work-status ' + item.status}>{workStatus(item.status)}</span>
      <p className="book-summary">{item.record.summary}</p>
      <h3>留下的依据</h3>
      {item.evidence.map(e => <div className="book-evidence" key={e.id}><blockquote>{e.quote}</blockquote><p>{e.source}</p></div>)}
      {!item.evidence.length && <p className="work-muted">这一页还在等待证据。</p>}
      <button className="book-discuss" onClick={() => discuss(item.record.id)}><MessageCircle size={16} />聊聊这一页</button>
      <footer><button disabled={index === 0} onClick={() => enter(work.records[index - 1].record.id)}>上一章</button><span>{index + 1} / {work.records.length}</span><button disabled={index === work.records.length - 1} onClick={() => enter(work.records[index + 1].record.id)}>下一章</button></footer>
    </article>
  </section>;
}
