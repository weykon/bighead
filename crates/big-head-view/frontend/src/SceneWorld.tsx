import { useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { ContactShadows, RoundedBox } from '@react-three/drei';
import { Group, Vector3 } from 'three';
import type { WorkProjection } from './WorkCity';

export type WorldObject = {
  entity_id: number; name?: { id: string; name: string };
  transform: { x: number; y: number; z: number; rx: number; ry: number; rz: number; sx: number; sy: number; sz: number };
  renderable: { kind: string; color: string; visible: boolean; intensity: number };
};
export type WorldProjection = { objects: WorldObject[]; systems: { entity_id: number; state: { system_id: string; enabled: boolean; runs: number; error: string } }[]; warnings: string[]; work?: WorkProjection };
export const sceneId = (o: WorldObject) => o.name?.id || `entity:${o.entity_id}`;
export const sceneTitle = (o: WorldObject) => o.name?.name || `物件 ${o.entity_id}`;

function Visual({ object, select }: { object: WorldObject; select: (id: string) => void }) {
  const group = useRef<Group>(null);
  const t = object.transform;
  const r = object.renderable;
  const initial = useRef<[number, number, number]>([t.x, t.y, t.z]);
  const target = new Vector3(t.x, t.y, t.z);
  useFrame((_, dt) => { group.current?.position.lerp(target, 1 - Math.exp(-dt * 20)); });
  const material = <meshStandardMaterial color={r.color} roughness={0.55} side={2} />;
  return <group ref={group} position={initial.current} rotation={[t.rx, t.ry, t.rz]} scale={[t.sx, t.sy, t.sz]} visible={r.visible} onClick={e => { e.stopPropagation(); select(sceneId(object)); }}>
    {r.kind === 'lamp' ? <>
      <mesh position={[0, 0.04, 0]} castShadow><cylinderGeometry args={[0.17, 0.19, 0.08, 24]} />{material}</mesh>
      <mesh position={[0, 0.36, 0]} castShadow><cylinderGeometry args={[0.028, 0.028, 0.62, 12]} />{material}</mesh>
      <mesh position={[0, 0.70, 0]} castShadow><coneGeometry args={[0.29, 0.33, 32, 1, true]} />{material}</mesh>
      <pointLight position={[0, 0.48, 0]} intensity={r.intensity} color={r.color} distance={4} />
    </> : r.kind === 'desk' ? <>
      <RoundedBox args={[2.7, 0.14, 1.3]} radius={0.055} position={[0, 0.93, 0]} castShadow receiveShadow>{material}</RoundedBox>
      {[-1.1, 1.1].flatMap(x => [-0.47, 0.45].map(z => <mesh key={`${x}:${z}`} position={[x, 0.4, z]} castShadow><boxGeometry args={[0.09, 0.95, 0.09]} />{material}</mesh>))}
    </> : r.kind === 'shelf' ? <>
      {[0.05, 0.95].map(y => <RoundedBox key={y} args={[2.65, 0.09, 0.48]} radius={0.025} position={[0, y, 0]} castShadow>{material}</RoundedBox>)}
    </> : r.kind === 'floor' ? <RoundedBox args={[5.9, 0.12, 4.3]} radius={0.04} position={[0, 0.04, 0]} receiveShadow>{material}</RoundedBox> : <mesh castShadow receiveShadow>
      {r.kind === 'box' && <boxGeometry />}
      {r.kind === 'sphere' && <sphereGeometry args={[0.5, 24, 16]} />}
      {r.kind === 'cylinder' && <cylinderGeometry args={[0.5, 0.5, 1, 24]} />}
      {r.kind === 'cone' && <coneGeometry args={[0.5, 1, 24]} />}
      {r.kind === 'torus' && <torusGeometry args={[0.35, 0.15, 12, 32]} />}
      {r.kind === 'plane' && <planeGeometry />}
      {material}
    </mesh>}
  </group>;
}

export function SceneWorld({ projection, select }: { projection: WorldProjection; select: (id: string) => void }) {
  return <group>
    <mesh position={[0, -0.04, 0]} rotation={[-Math.PI / 2, 0, 0]} receiveShadow><planeGeometry args={[200, 200]} /><meshStandardMaterial color="#edf0ef" /></mesh>
    <mesh position={[0, 1.7, -2]} receiveShadow><boxGeometry args={[5.9, 3.4, 0.12]} /><meshStandardMaterial color="#e1e8e8" /></mesh>
    <mesh position={[-1.6, 2.03, -1.92]}><boxGeometry args={[1.45, 1.65, 0.12]} /><meshStandardMaterial color="#ffffff" /></mesh>
    <mesh position={[-1.6, 2.03, -1.84]}><planeGeometry args={[1.26, 1.46]} /><meshStandardMaterial color="#abd4e5" /></mesh>
    {projection.objects.map(o => <Visual key={sceneId(o)} object={o} select={select} />)}
    <ContactShadows position={[0, 0.105, 0]} opacity={0.3} scale={10} blur={2.5} far={4} />
  </group>;
}
