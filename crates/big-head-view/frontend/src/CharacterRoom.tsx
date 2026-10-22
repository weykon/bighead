import { useEffect, useMemo, useRef } from 'react';
import { Canvas, useFrame, useThree } from '@react-three/fiber';
import { ContactShadows, RoundedBox, OrbitControls, Html } from '@react-three/drei';
import { Vector3, type Group } from 'three';
import { LampDesk } from 'lucide-react';
import { SceneWorld, type WorldProjection } from './SceneWorld';

export type SceneObject = {
  entity_id: number;
  item: { id: string; name: string; kind: string; size: [number, number, number]; allowed_surfaces: string[] };
  placement: { surface: string; position: [number, number, number]; revision: number };
};
export const locationLabel = (id: string) => ({ desk: '桌上', shelf: '书架', floor: '地上', wall: '墙边', room: '房间里' }[id] ?? id);

export type RoomObject = {
  id: string; title: string; body: string; kind: string; source_turn_id: string;
  source_quote: string; reason: string; location: string; revision: number; user_confirmed: boolean;
};
export const objectLabel = (o: RoomObject) => o.user_confirmed ? '已确认' : o.kind === 'idea' ? '猜想' : '原话';

function Companion({ attentive, greet }: { attentive: boolean; greet: () => void }) {
  const body = useRef<Group>(null);
  const hand = useRef<Group>(null);
  useFrame(({ clock }) => {
    const t = clock.getElapsedTime();
    if (body.current) {
      body.current.position.y = Math.sin(t * 1.7) * 0.035;
      body.current.rotation.y = attentive ? Math.sin(t * 0.8) * 0.08 : Math.sin(t * 0.35) * 0.24;
    }
    if (hand.current) hand.current.rotation.z = attentive ? -1.9 + Math.sin(t * 5) * 0.25 : -0.15;
  });
  return <group ref={body} onClick={e => { e.stopPropagation(); greet(); }}>
    <RoundedBox args={[1.12, 0.9, 0.8]} radius={0.22} position={[0, 1.75, 0]}><meshStandardMaterial color="#c4e9df" roughness={0.48} /></RoundedBox>
    <RoundedBox args={[0.88, 0.49, 0.10]} radius={0.045} position={[0, 1.76, 0.45]}><meshStandardMaterial color="#263d3b" /></RoundedBox>
    {[-0.22, 0.22].map(x => <mesh key={x} position={[x, 1.79, 0.52]}><capsuleGeometry args={[0.035, 0.08, 4, 12]} /><meshStandardMaterial color="#fff9b9" emissive="#fff9b9" emissiveIntensity={0.6} /></mesh>)}
    <mesh position={[0, 2.31, 0]}><sphereGeometry args={[0.075, 16, 16]} /><meshStandardMaterial color="#ee786b" /></mesh>
    <mesh position={[0, 2.24, 0]}><cylinderGeometry args={[0.02, 0.02, 0.14]} /><meshStandardMaterial color="#375752" /></mesh>
    <RoundedBox args={[0.78, 0.77, 0.57]} radius={0.18} position={[0, 0.89, 0]}><meshStandardMaterial color="#5daca0" /></RoundedBox>
    <mesh position={[0, 0.98, 0.3]} rotation={[Math.PI / 2, 0, 0]}><cylinderGeometry args={[0.1, 0.1, 0.025, 24]} /><meshStandardMaterial color="#f4cb62" /></mesh>
    {[-1, 1].map(side => <group key={side}>
      <RoundedBox args={[0.29, 0.31, 0.49]} radius={0.09} position={[side * 0.25, 0.27, 0.1]}><meshStandardMaterial color="#294a46" /></RoundedBox>
      <mesh position={[side * 0.25, 0.47, 0]}><sphereGeometry args={[0.11, 16, 16]} /><meshStandardMaterial color="#c4e9df" /></mesh>
    </group>)}
    <group position={[0.48, 1.12, 0]} ref={hand}><RoundedBox args={[0.22, 0.57, 0.24]} radius={0.1} position={[0.06, -0.2, 0]}><meshStandardMaterial color="#c4e9df" /></RoundedBox></group>
    <RoundedBox args={[0.22, 0.57, 0.24]} radius={0.1} position={[-0.51, 0.93, 0]} rotation={[0, 0, -0.1]}><meshStandardMaterial color="#c4e9df" /></RoundedBox>
  </group>;
}


function SceneLamp({ object, select }: { object: SceneObject; select: (id: string) => void }) {
  const group = useRef<Group>(null);
  const initial = useRef(object.placement.position);
  const target = useMemo(() => new Vector3(...object.placement.position), [object.placement.position]);
  useFrame((_, delta) => { group.current?.position.lerp(target, 1 - Math.exp(-delta * 9)); });
  return <group ref={group} position={initial.current} name="scene-lamp" onClick={e => { e.stopPropagation(); select(object.item.id); }}>
    <mesh position={[0, 0.04, 0]} castShadow><cylinderGeometry args={[0.17, 0.19, 0.08, 24]} /><meshStandardMaterial color="#d58c4e" /></mesh>
    <mesh position={[0, 0.36, 0]} castShadow><cylinderGeometry args={[0.028, 0.028, 0.62, 12]} /><meshStandardMaterial color="#b07847" /></mesh>
    <mesh position={[0, 0.70, 0]} castShadow><coneGeometry args={[0.29, 0.33, 32, 1, true]} /><meshStandardMaterial color="#f0cc69" side={2} /></mesh>
    <pointLight position={[0, 0.48, 0]} intensity={0.5} color="#ffe8b0" distance={3} />
    <Html center position={[0, 1.0, 0]} zIndexRange={[3, 0]}><button className="room-token-label" title="台灯" aria-label="查看场景物件：台灯" onClick={() => select(object.item.id)}><LampDesk size={15} /></button></Html>
  </group>;
}

function Furnishings({ scene, select }: { scene: SceneObject[]; select: (id: string) => void }) {
  const floor = scene.find(o => o.item.id === 'floor')?.placement.position ?? [0, 0, 0];
  const desk = scene.find(o => o.item.id === 'desk')?.placement.position ?? [0.45, 0.1, 0.45];
  const shelf = scene.find(o => o.item.id === 'shelf')?.placement.position ?? [1.1, 0.75, -1.68];
  return <group>
    <mesh rotation={[-Math.PI / 2, 0, 0]} receiveShadow><planeGeometry args={[200, 200]} /><meshStandardMaterial color="#edf0ef" /></mesh>
    <RoundedBox args={[5.9, 0.12, 4.3]} radius={0.04} position={[floor[0], floor[1] + 0.04, floor[2]]} receiveShadow><meshStandardMaterial color="#d8e4df" /></RoundedBox>
    <mesh position={[0, 1.7, -2]} receiveShadow><boxGeometry args={[5.9, 3.4, 0.12]} /><meshStandardMaterial color="#e1e8e8" /></mesh>
    <RoundedBox args={[1.45, 1.65, 0.09]} radius={0.04} position={[-1.6, 2.03, -1.89]}><meshStandardMaterial color="#f9fbfc" /></RoundedBox>
    <mesh position={[-1.6, 2.03, -1.83]}><planeGeometry args={[1.26, 1.46]} /><meshStandardMaterial color="#abd4e5" emissive="#abd4e5" emissiveIntensity={0.3} /></mesh>
    <mesh position={[-1.6, 2.03, -1.78]}><boxGeometry args={[0.05, 1.48, 0.05]} /><meshStandardMaterial color="#fffafa" /></mesh>
    <mesh position={[-1.6, 2.03, -1.78]}><boxGeometry args={[1.28, 0.05, 0.05]} /><meshStandardMaterial color="#fffafa" /></mesh>
    <group position={desk as [number, number, number]} onClick={e => { e.stopPropagation(); select('desk'); }}>
      <RoundedBox args={[2.7, 0.14, 1.3]} radius={0.055} position={[0, 0.93, 0]} castShadow receiveShadow><meshStandardMaterial color="#faf4e8" /></RoundedBox>
      {[-1.1, 1.1].flatMap(x => [-0.47, 0.45].map(z => <mesh key={x + '-' + z} position={[x, 0.4, z]} castShadow><boxGeometry args={[0.09, 0.95, 0.09]} /><meshStandardMaterial color="#6e817b" /></mesh>))}
    </group>
    <group position={shelf as [number, number, number]} onClick={e => { e.stopPropagation(); select('shelf'); }}>
      {[0.05, 0.95].map(y => <RoundedBox key={y} args={[2.65, 0.09, 0.48]} radius={0.025} position={[0, y, 0]} castShadow><meshStandardMaterial color="#b47c89" /></RoundedBox>)}
    </group>
    {scene.filter(o => o.item.kind === 'lamp').map(object => <SceneLamp key={object.item.id} object={object} select={select} />)}
    <ContactShadows position={[0, 0.105, 0]} opacity={0.3} scale={10} blur={2.5} far={4} />
  </group>;
}

function RoomToken({ object, index, selected, select }: { object: RoomObject; index: number; selected: boolean; select: () => void }) {
  const shelf = object.location === 'shelf';
  const color = object.user_confirmed ? '#9cd1b9' : object.kind === 'idea' ? '#e7a2b1' : '#f0d781';
  const position: [number, number, number] = shelf
    ? [0.12 + (index % 4) * 0.6, (index < 4 ? 1.7 : 0.8) + 0.19, -1.65]
    : object.location === 'floor' ? [-0.8 + (index % 4) * 0.49, 0.14, 1.45 + Math.floor(index / 4) * 0.4]
    : [-0.53 + (index % 4) * 0.49, 1.14, 0.22 + Math.floor(index / 4) * 0.5];
  return <group position={position} onClick={e => { e.stopPropagation(); select(); }}>
    <RoundedBox args={shelf ? [0.4, 0.29, 0.14] : [0.41, 0.08, 0.34]} radius={0.025} castShadow>
      <meshStandardMaterial color={color} emissive={selected ? '#f6d370' : '#000000'} emissiveIntensity={selected ? 0.35 : 0} />
    </RoundedBox>
    <mesh position={shelf ? [0, 0, 0.077] : [0, 0.045, 0]} rotation={shelf ? [0, 0, 0] : [-Math.PI / 2, 0, 0]}><planeGeometry args={[0.27, 0.025]} /><meshStandardMaterial color="#fffcf4" /></mesh>
    <Html center position={[0, shelf ? 0.25 : 0.17, 0]} zIndexRange={[3, 0]}>
      <button className={'room-token-label ' + (selected ? 'selected' : '')} aria-label={'查看物件：' + object.title} onClick={select}>{index + 1}</button>
    </Html>
  </group>;
}

function RoomControls() {
  const { camera, size } = useThree();
  const distanceScale = Math.max(size.height < 220 ? 0.72 : 1, 1.35 / (size.width / size.height));
  useEffect(() => {
    camera.position.set(5 * distanceScale, 1.2 + 3 * distanceScale, 7.5 * distanceScale);
    camera.lookAt(0, 1.2, 0);
  }, [camera, distanceScale]);
  return <OrbitControls target={[0, 1.2, 0]} enablePan={false} minDistance={6 * distanceScale} maxDistance={12 * distanceScale} minPolarAngle={0.65} maxPolarAngle={1.4} />;
}

export function CharacterRoom(props: { objects: RoomObject[]; scene: SceneObject[]; projection?: WorldProjection; selected: string; select: (id: string) => void; selectScene: (id: string) => void; attentive: boolean; greet: () => void }) {
  return <Canvas shadows camera={{ position: [5, 4.2, 7.5], fov: 40 }} gl={{ preserveDrawingBuffer: true }}>
    <color attach="background" args={['#edf0ef']} />
    <ambientLight intensity={1.6} /><directionalLight position={[-3, 7, 5]} intensity={2.5} castShadow shadow-mapSize={[1024, 1024]} />
    {props.projection ? <SceneWorld projection={props.projection} select={props.selectScene} /> : <Furnishings scene={props.scene} select={props.selectScene} />}
    <group position={[-1.8, 0.1, 0.5]} rotation={[0, 0.3, 0]} scale={0.83}><Companion attentive={props.attentive} greet={props.greet} /></group>
    {props.objects.map(object => <RoomToken key={object.id} object={object} index={props.objects.filter(o => o.location === object.location).findIndex(o => o.id === object.id)} selected={object.id === props.selected} select={() => props.select(object.id)} />)}
    <RoomControls />
  </Canvas>;
}
