# Scene World 操作指南

这是 Big Head 页面中可编程 ECS 房间与工作街区的按需操作手册。执行场景或工作对象变更前先读取实际状态，再按本指南组合通用 ECS CRUD、动态组件和 Rhai system；不要把每种行为误认为需要新增专用工具。

## 可视化组件

给一个实体组合这些已注册组件即可出现在页面：
- SceneName: {name:string, id:string}。id 可省略，自动生成并持久化。用名字/稳定 id 找实体，不要把 numeric entity_id 写死进长期脚本，恢复后数字会变。
- SceneTransform: {x:number,y:number,z:number,rx:number,ry:number,rz:number,sx:number,sy:number,sz:number}。坐标默认0，旋转为弧度默认0，缩放默认1。都是世界坐标。
- SceneRenderable: {kind:string,color:string,visible:boolean,intensity:number}。kind 为 box/sphere/cylinder/cone/torus/plane，或已有的 lamp/desk/shelf/floor 模型。颜色用 #RRGGBB，默认可见。基础图形居中，单位尺寸约1，sphere 半径0.5；地板顶部 y=0.1。缩放0<值<=20，坐标绝对值<=100。台灯以底座为坐标原点，强度0..20。

可以自由改变这些组件或增删实体。位置、颜色、大小、光强和可见性会自动投影到 Three.js，无需调用 render_view。原有家具也已挂载这些组件；移动家具修改 SceneTransform，不再使用旧 RoomPlacement。任何新业务组件（例如轨道参数、漂浮速度、跟随目标等）需先 define_component，字段类型 number/integer/string/boolean。新组件本身不自动产生画面，写 Rhai 将其状态反映到上述可视化组件。

## 脚本与持续执行

define_system({id,description,source}) 注册 Rhai 代码；run_system({id,args:{dt:0.05,elapsed:0.05}}) 实际试跑。
持续运行：另建一个带 SceneSystem 的实体，值 {system_id:"你的system id",enabled:true}。调度器约每50ms运行一次，不用再创建 TickAction，不用反复 spawn 任务。运行时向脚本传 args.dt（本步秒数）、args.elapsed（该系统累计秒数）、args.entity_id（调度实体）。SceneSystem.runs/error 是运行证据；出错会自动 enabled=false，修复代码后再启用。重复请求要复用已有系统和调度实体，停止行为用 enabled=false 或移除对应行为组件。

Rhai API:
- query("ComponentName") 返回数组，每行有 _entity_id 和 ComponentName 属性。多组件不要凭空假设一定都有；可 query 一个行为组件，然后 has_component(id,"SceneTransform") 检查。
- get_component(eid,"SceneTransform") 返回 map；set_component(eid,"SceneTransform",#{x:1.0}) 支持部分更新。
- spawn_entity(#{"SceneName":#{name:"物件"},"SceneTransform":#{x:0.0,y:1.0,z:0.0},"SceneRenderable":#{kind:"box",color:"#ee6677"}}) 返回实体id。
- define_component(name, #{field:"number"})、remove_component、despawn_entity、log；数学 sin(angle)/cos(angle)，角度为浮点数弧度。
- args.dt 只在调度器或你明确传参的 run_system 调用中存在。每次运行的局部变量会重置，累计状态要放在组件里，或使用 args.elapsed。
- 不要把整个脚本套在 JSON/map 字面量里。脚本从 let/for/if 等 Rhai 语句开始。

用户只想改一个对象时，用自定义组件上的模式/参数控制各实体，保留其他对象的行为和状态。不要重建整个世界。用户要求“停止并留在当前位置”时停用行为即可，不要复位变换。绕圈半径要考虑当前视野和家具，默认用小方块、较小半径，以免物件跑出房间。不要改写已有确认便签的正文或来源。

## 检查与边界

先读实际实体与已定义系统，需要时 inspect_ecs_capabilities(view:"systems",name:...) 取完整代码和 definition_hash。update_system 必须带 expected_definition_hash 与完整 source，更新后需实际 run_system 并回读组件验证，不能把编译成功当作行为成功。
用 inspect_scene 查看实际投影和脚本健康情况。先测试脚本，再挂 SceneSystem；若已挂载且失败，修好并重新启用。每轮最多32次推理，脚本每次最多100000操作；最多16个持续系统和128个可视物件。错误前的脚本写入可能已经发生，需要检查当前状态，不要声称全部撤销。无工具执行证据时不要声称完成。
当前模型推理与场景调度共享主循环；等待你的回复期间脚本不会自动推进。不要靠连续 inspect_scene 等待 runs 增长，要用 run_system 带 args 实际试跑并检查；回复结束后持续调度恢复。
不要编辑审计、角色对话或运行时投递实体，也不要制造 Spark 自激循环。你操作的是当前专用世界，不具有 shell 或网络权限。这些 Rhai 工具不是面对恶意脚本的安全隔离边界。

## 工作街区与成果之书

world_projection.work 是工作街区。建筑和书中的章节共享同一个 WorkRecord，绝不复制一份任务状态。active_view 为 city/book 时优先理解工作对象，selected_object_id 可以是 WorkRecord.id。旧房间家具在另一个视图，不要为了布局城市而删除它们。
inspect_work 提供事实、证据快照、建筑布局和联系。初始记录来自项目文档与用户请求，是有来源的历史记录，不是本轮重新执行的测试。别把旧结果当成刚刚验证，不能用不相关的证据证明新成果完成。新想法应为 idea/in_progress。record_work 用当前 expected_revision 更新事实；arrange_work 只安排建筑和道路，不改变事实状态。书自动更新，不需要额外创建书页实体。
进入建筑时页面原生打开 WorkInspector，并直接展示该 WorkRecord 的已关联证据；这不需要 WorkTrigger、额外 panel 组件或 Rhai。record_work 也已经原生拒绝没有匹配完成证据的 completed 状态。先通过 inspect_work 和工具描述确认这些契约，不要用动态组件重复实现已有校验。
任意动态组件不会自动出现在页面。只有 world_projection 已投影的字段或前端明确支持的组件才会渲染；Rhai 写入一个 WorkPanel/BuildingPanel 类组件不等于用户能看到面板。WorkTrigger 用于进入事件的附加副作用，不用于打开原生详情或显示已有证据。如果请求需要新的 UI 投影而当前没有对应工具，应明确指出引擎缺口，不要用不可见组件假装完成。
WorkRecord {id,title,summary,status,evidence_ids,revision}；WorkAppearance {x,z,height,color,style,glow,revision}。样式 archive/workshop/lab/tower/garden，x 在 -5..5，z 在 -4..4，高度 .5..2.4。用 arrange_work 批量布置，中心间距至少1.5，保持稳定 id。每个 placements 项必须带 appearance.expected_revision。未提到的建筑/道路保持不动。道路是 WorkConnection {from,to,label}，表达实际联系而不是虚构完成量。
用户希望你自行安排时，你可以按相关性调整建筑位置、外观和连接，不必逐项询问；安排完 inspect_work 回读。移动建筑不能标记工作完成，不能把已知 blocker 改成已解决。需要记录新结果但缺少可引用证据时，先保留 in_progress 并说明缺口。
进入建筑会触发 enter 事件并在 work.events 留下有限记录。要响应事件：先 define_system，试跑 run_system({id,args:{event:{target_id:"checkpoint",kind:"enter",sequence:1}}})，然后创建 WorkTrigger {target_id:"checkpoint" 或 "*",event:"enter",system_id:"你定义的系统",enabled:true}。真实点击由 runtime 运行，无需 LLM。脚本用 args.event.target_id 与 query("WorkRecord") 的稳定id匹配实体。可以修改 WorkAppearance.glow (0..2)、颜色或自定义状态；不要每次点击 spawn 新任务。每个触发器 runs/error 是执行证据。重复 event id 不重放，最近32次有去重；错误前写入可能保留。要渐变或其他持续行为，沿用 SceneSystem + args.dt，查询 WorkAppearance 更新，页面会同步。
目前 enter 是打开建筑详情，不是真实寻路/碰撞事件。没有自动导入 Git/Codex 更新或自主后台规划；只在当前明确请求或已安装脚本触发时执行。书与城市布局可由你安排，但工作完成必须有实际证据。
交互计时：WorkTrigger 仅提供 args.event，绝无 args.elapsed。不要把事件的开始时间写成0再减去 SceneSystem 累计时间。延迟/渐变应把 remaining_secs 存进自定义组件，每次事件重置，持续系统按 args.dt 扣减，到0移除临时组件。连续两次事件都应验证。原生 WorkDistrict 中的证据目录由初始化导入，不要自行伪造或改写来源；当前只能为已导入的工作引用其对应完成证据。
