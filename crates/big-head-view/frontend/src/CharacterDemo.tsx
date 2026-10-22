import { useEffect, useRef, useState } from 'react';
import { Send, Hand, RotateCcw, Check, Pencil, ArrowUpFromLine, ArrowDownToLine, Trash2, X, MessageCircle, Bookmark, Save, LampDesk, Building2, BookOpen, Home, Plus, Route } from 'lucide-react';
import type { PanelState } from './types';
import { sendViewEvent } from './ws';
import { CharacterRoom, objectLabel, locationLabel, type RoomObject, type SceneObject } from './CharacterRoom';
import './character.css';
import { sceneId, sceneTitle, type WorldProjection } from './SceneWorld';
import { WorkCity, WorkBook, WorkInspector, workStatus } from './WorkCity';
import './work-world.css';

export const CHARACTER_PANEL = 'character-demo';
type Turn = { id: string; text: string; reply: string; status: string };
type State = { visits: number; turns: Turn[]; activity?: string; objects?: RoomObject[]; scene?: SceneObject[]; world_projection?: WorldProjection; last_event_id?: string; notice?: string; room_changes?: { id: string; message: string }[] };
type Request = { action: string; interaction_id: string; text?: string; object_id?: string; expected_revision?: number; body?: string; view?: string };

function Inspector({ object, disabled, act, close, discuss }: { object: RoomObject; disabled: boolean; act: (action: string, body?: string) => void; close: () => void; discuss: () => void }) {
  const [editing, setEditing] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [body, setBody] = useState(object.body);
  useEffect(() => { setEditing(false); setRemoving(false); setBody(object.body); }, [object.id, object.revision]);
  return <section className="room-inspector" aria-label="物件详情">
    <header><div><small>{objectLabel(object)} · {locationLabel(object.location)}</small><h3>{object.title}</h3></div><button title="关闭详情" aria-label="关闭详情" onClick={close}><X size={18} /></button></header>
    {editing ? <div className="room-edit"><textarea autoFocus aria-label="修正内容" maxLength={400} value={body} onChange={e => setBody(e.target.value)} /><div><button onClick={() => setEditing(false)} title="取消修正" aria-label="取消修正"><X size={17} /></button><button disabled={disabled || !body.trim()} onClick={() => act('room_correct', body.trim())}><Save size={16} />保存修正</button></div></div> : <p className="room-body">{object.body}</p>}
    <details open><summary>来自这句话</summary><blockquote>{object.source_quote}</blockquote><p>{object.reason}</p></details>
    {removing ? <div className="room-remove"><p>撤回「{object.title}」？</p><button disabled={disabled} onClick={() => act('room_remove')}><Trash2 size={16} />确认撤回</button><button onClick={() => setRemoving(false)}>取消</button></div> : <div className="room-actions">
      {!object.user_confirmed && <button disabled={disabled} onClick={() => act('room_confirm')}><Check size={16} />确认</button>}
      <button disabled={disabled} onClick={() => setEditing(true)}><Pencil size={16} />修正</button>
      <button disabled={disabled} onClick={() => act('room_move')}>{object.location === 'desk' ? <ArrowUpFromLine size={16} /> : <ArrowDownToLine size={16} />}{object.location === 'desk' ? '移到书架' : '放回桌上'}</button>
      <button disabled={disabled} title="撤回物件" aria-label="撤回物件" onClick={() => setRemoving(true)}><Trash2 size={16} /></button>
      <button disabled={disabled} onClick={discuss}><MessageCircle size={16} />接着聊</button>
    </div>}
  </section>;
}

export function CharacterDemo({ panel, connected }: { panel?: PanelState; connected: boolean }) {
  const state = (panel?.content.kind === 'a2ui' ? panel.content.components.find(c => c.id === 'character-state')?.state : undefined) as State | undefined;
  const [draft, setDraft] = useState('');
  const [pending, setPending] = useState<{ request: Request; at: number } | null>(null);
  const [now, setNow] = useState(Date.now());
  const [wave, setWave] = useState(false);
  const [selected, setSelected] = useState('');
  const [topic, setTopic] = useState('');
  const [tab, setTab] = useState<'chat' | 'objects' | 'scene' | 'work'>('work');
  const [view, setView] = useState<'city' | 'book' | 'room'>('city');
  const transcript = useRef<HTMLDivElement>(null);
  const composer = useRef<HTMLInputElement>(null);
  const objects = state?.objects ?? [];
  const scene = state?.scene ?? [];
  const projection = state?.world_projection;
  const work = projection?.work;
  const selectedWork = work?.records.find(r => r.record.id === selected);
  const worldObjects = projection?.objects ?? [];
  const selectedWorld = worldObjects.find(o => sceneId(o) === selected);
  const selectedScene = scene.find(o => o.item.id === selected);
  const object = objects.find(o => o.id === selected);
  const topicObject = objects.find(o => o.id === topic);
  const topicScene = scene.find(o => o.item.id === topic);
  const topicWorld = worldObjects.find(o => sceneId(o) === topic);
  const topicTitle = work?.records.find(r => r.record.id === topic)?.record.title ?? topicObject?.title ?? (topicWorld ? sceneTitle(topicWorld) : topicScene?.item.name);
  const latest = state?.turns.at(-1);
  const waiting = !!pending || latest?.status === 'thinking';
  const ready = connected && !!state;
  useEffect(() => { if (connected) sendViewEvent(CHARACTER_PANEL, { action: 'sync' }); }, [connected]);
  useEffect(() => {
    if (pending && (state?.last_event_id === pending.request.interaction_id || state?.turns.some(t => t.id === pending.request.interaction_id))) setPending(null);
  }, [state, pending, tab]);
  useEffect(() => { transcript.current?.scrollTo({ top: transcript.current.scrollHeight, behavior: 'smooth' }); }, [latest?.id, latest?.status, pending, tab]);
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, []);
  useEffect(() => { if (!wave) return; const timer = setTimeout(() => setWave(false), 2500); return () => clearTimeout(timer); }, [wave]);
  function request(payload: Omit<Request, 'interaction_id'>) {
    if (!ready || waiting) return false;
    const request = { ...payload, interaction_id: crypto.randomUUID() };
    if (!sendViewEvent(CHARACTER_PANEL, request)) return false;
    setPending({ request, at: Date.now() });
    return true;
  }
  function send(text: string, action = 'say') {
    if (!text.trim()) return;
    if (request({ action, text: text.trim(), view, ...(topicTitle ? { object_id: topic } : {}) })) { setDraft(''); setWave(true); setTab('chat'); }
  }
  function enterWork(id: string) {
    setSelected(id); setTopic(id); setTab(view === 'book' ? 'chat' : 'work');
    if (ready) sendViewEvent(CHARACTER_PANEL, { action: 'work_enter', interaction_id: crypto.randomUUID(), object_id: id });
  }
  function discussWork(id: string) { setTopic(id); setTab('chat'); setTimeout(() => composer.current?.focus(), 0); }
  function act(action: string, body?: string) {
    if (object) request({ action, object_id: object.id, expected_revision: object.revision, ...(body === undefined ? {} : { body }) });
  }
  const status = !connected ? '连接已断开' : !state ? '等待角色上线' : pending ? '正在送达' : latest?.status === 'thinking' ? '正在回应' : latest?.status === 'partial' ? '部分完成，可继续' : latest?.status === 'error' || latest?.status === 'interrupted' ? '回应未完成' : state.activity === 'resting' ? '歇一会儿' : '在这里，听你说';
  return <section className={'character-world world-view-' + view}>
    <div className="character-stage">
      <div className="character-heading"><span>BIG HEAD / {view === 'room' ? 'OUR ROOM' : 'LIVING WORK'}</span><h2>{view === 'room' ? '小小世界' : view === 'city' ? '我们的工作街区' : '成果之书'}</h2><p><i className={ready ? 'online' : ''} />{status}</p></div>
      <nav className="world-views" aria-label="世界视图">{([['city', Building2, '街区'], ['book', BookOpen, '书'], ['room', Home, '房间']] as const).map(([id, Icon, label]) => <button key={id} title={label} aria-label={label + '视图'} aria-pressed={view === id} onClick={() => { setView(id); setTab(id === 'room' ? 'scene' : id === 'book' ? 'chat' : 'work'); }}><Icon size={18} /></button>)}</nav>
      {view === 'room' ? <div className="character-canvas"><CharacterRoom objects={objects} scene={scene} projection={projection} selected={selected} select={id => { setSelected(id); setTab('objects'); }} selectScene={id => { setSelected(id); setTab('scene'); }} attentive={wave || waiting} greet={() => send('早上好', 'greet')} /></div> : work?.initialized ? view === 'city' ? <div className="character-canvas city-canvas"><WorkCity work={work} selected={selected} enter={enterWork} /></div> : <WorkBook work={work} selected={selected} enter={enterWork} discuss={discussWork} /> : <div className="work-empty"><Building2 size={40} /><button disabled={!ready || waiting} onClick={() => request({ action: 'work_initialize' })}><Plus size={17} />建立项目街区</button></div>}
      {view !== 'book' && <div className="character-stage-footer"><span>{view === 'city' ? `${work?.records.length ?? 0} 项工作 · ${work?.connections.length ?? 0} 条联系` : `相遇 ${state?.visits ?? 0} 次 · ${objects.length} 件小东西`}</span>{view === 'city' ? <button disabled={!ready || waiting || !work?.records.length} onClick={() => { const i = work!.records.findIndex(r => r.record.id === selected); enterWork(work!.records[(i + 1) % work!.records.length].record.id); }}><Route size={17} />逛逛街区</button> : <button onClick={() => send('早上好', 'greet')} disabled={!ready || waiting}><Hand size={17} />打个招呼</button>}</div>}
    </div>
    <aside className="character-conversation">
      <nav className="character-tabs" aria-label="房间内容">
        <button aria-pressed={tab === 'chat'} onClick={() => setTab('chat')}><MessageCircle size={16} />说说话</button>
        {view !== 'room' ? <button aria-pressed={tab === 'work'} onClick={() => setTab('work')}><Building2 size={16} />工作 <span>{work?.records.length ?? 0}</span></button> : <>
        <button aria-pressed={tab === 'objects'} onClick={() => setTab('objects')}><Bookmark size={16} />留下的东西 <span>{objects.length}/8</span></button>
        <button aria-pressed={tab === 'scene'} onClick={() => setTab('scene')}><LampDesk size={16} />房间</button>
        </>}
      </nav>
      {tab === 'chat' ? <div className="character-transcript" ref={transcript} role="log" aria-live="polite">
        {!state?.turns.length && !pending && <div className="character-empty">今天想聊些什么？</div>}
        {state?.turns.map(turn => <div className="character-turn" key={turn.id}><p className="character-user"><small>你</small>{turn.text}</p><p className={turn.status === 'error' || turn.status === 'interrupted' ? 'character-reply failed' : turn.status === 'partial' ? 'character-reply partial' : 'character-reply'}><small>Big Head</small>{turn.status === 'thinking' ? '正在想怎么回应你…' : turn.reply}</p></div>)}
        {pending?.request.text && <p className="character-user"><small>你 · 待接收</small>{pending.request.text}</p>}
      </div> : <div className="room-collection">
        {tab === 'work' && work ? <>
          {selectedWork ? <WorkInspector item={selectedWork} work={work} close={() => setSelected('')} discuss={() => discussWork(selectedWork.record.id)} enter={enterWork} openBook={() => { setView('book'); setTab('chat'); }} /> : <div className="work-list">{work.records.map((r, i) => <button key={r.record.id} onClick={() => enterWork(r.record.id)}><span className="work-number" style={{ borderColor: r.appearance?.color }}>{String(i + 1).padStart(2, '0')}</span><span>{r.record.title}<small className={'work-status ' + r.status}>{workStatus(r.status)}</small></span></button>)}</div>}
          {!!work.events.length && <div className="room-history"><h4>最近来过</h4>{work.events.slice(-4).reverse().map(e => <p key={e.id}>#{e.sequence} · {work.records.find(r => r.record.id === e.target_id)?.record.title ?? e.target_id}{e.status === 'failed' ? ' · 响应失败' : e.status === 'accepted' ? ' · 响应尚未完成' : e.status === 'executed' ? ' · 已响应' : ''}</p>)}</div>}
          {work.triggers.map(t => <div className="room-history" key={t.entity_id}><p>{t.trigger.error ? `互动已暂停：${t.trigger.error}` : `进入时响应 · ${t.trigger.enabled ? '已启用' : '已暂停'} · ${t.trigger.runs} 次`}</p></div>)}
          {work.warnings.map(w => <p className="character-notice" key={w}>{w}</p>)}
        </> : tab === 'scene' ? <>
          <div className="room-object-list">{projection ? worldObjects.map(o => <button key={sceneId(o)} aria-pressed={selected === sceneId(o)} onClick={() => setSelected(sceneId(o))}><span className="room-swatch" style={{ background: o.renderable.color }} /><span className="room-object-name">{sceneTitle(o)}<small>{o.renderable.visible ? '可见' : '已隐藏'}</small></span></button>) : scene.map(o => <button key={o.item.id} aria-pressed={selected === o.item.id} onClick={() => setSelected(o.item.id)}><LampDesk size={18} /><span className="room-object-name">{o.item.name}<small>{locationLabel(o.placement.surface)}</small></span></button>)}</div>
          {selectedWorld ? <section className="room-inspector" aria-label="场景物件详情"><header><h3>{sceneTitle(selectedWorld)}</h3></header><p className="room-body">位置 {selectedWorld.transform.x.toFixed(2)}, {selectedWorld.transform.y.toFixed(2)}, {selectedWorld.transform.z.toFixed(2)}</p><div className="room-actions"><button disabled={!ready || waiting} onClick={() => { setTopic(sceneId(selectedWorld)); setTab('chat'); setTimeout(() => composer.current?.focus(), 0); }}><MessageCircle size={16} />接着聊</button></div></section> : !projection && selectedScene && <section className="room-inspector"><h3>{selectedScene.item.name}</h3></section>}
          {!!projection?.systems.length && <div className="room-history"><h4>持续动作</h4>{projection.systems.map(s => <p key={s.entity_id}>{s.state.system_id} · {s.state.error ? '出错已暂停' : s.state.enabled ? '运行中' : '已暂停'} · {s.state.runs.toLocaleString()}<br />{s.state.error}</p>)}</div>}
          {projection?.warnings.map(w => <p className="character-notice" key={w}>{w}</p>)}
        </> : <>
        <div className="room-object-list">{objects.length === 0 && <p className="character-empty">桌上暂时空着。</p>}{objects.map(o => <button key={o.id} aria-pressed={selected === o.id} onClick={() => setSelected(o.id)}><span className={'room-swatch ' + (o.user_confirmed ? 'confirmed' : o.kind)} /><span className="room-object-name">{o.title}<small>{objectLabel(o)} · {locationLabel(o.location)}</small></span></button>)}</div>
        {object && <Inspector object={object} disabled={!ready || waiting} act={act} close={() => setSelected('')} discuss={() => { setTopic(object.id); setTab('chat'); setTimeout(() => composer.current?.focus(), 0); }} />}
        </>}
        {view === 'room' && !!state?.room_changes?.length && <div className="room-history"><h4>房间的变化</h4>{state.room_changes.slice(-4).reverse().map(r => <p key={r.id}>{r.message}</p>)}</div>}
      </div>}
      {state?.notice && <div className="character-notice" role="status">{state.notice}</div>}
      {pending && now - pending.at > 30000 && <div className="character-notice">暂未收到确认。<button title="重试同一请求" aria-label="重试同一请求" onClick={() => sendViewEvent(CHARACTER_PANEL, pending.request)} disabled={!connected}><RotateCcw size={16} /></button></div>}
      {tab === 'chat' && <>
        {topicTitle && <div className="character-topic"><span>聊聊：{topicTitle}</span><button title="结束这个话题" aria-label="结束这个话题" onClick={() => setTopic('')}><X size={14} /></button></div>}
        <form onSubmit={e => { e.preventDefault(); send(draft); }}><input ref={composer} aria-label="对角色说的话" maxLength={1000} placeholder="说点什么…" value={draft} onChange={e => setDraft(e.target.value)} disabled={!ready || waiting} /><button title="发送" aria-label="发送" disabled={!ready || waiting || !draft.trim()}><Send size={19} /></button></form>
      </>}
    </aside>
  </section>;
}
