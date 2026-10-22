//! Rhai 脚本引擎与 ECS 世界绑定。
//!
//! 脚本可以低层访问 ECS 世界：直接遍历实体、读写组件、创建/销毁实体。
//! `World` 和 `ComponentActionRegistry` 的线程局部指针在 `Engine::eval`
//! 前设置，通过 RAII guard 清理。

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use rhai::{Array, Dynamic, Engine, EvalAltResult, Map};

use big_head_ai::log::{LogEmitter, LogLevel, emit as emit_log};

use big_head_ecs::components::{Spark, SparkStatus};
use big_head_ecs::dynamic_component;
use big_head_ecs::registry::ComponentActionRegistry;
use big_head_ecs::schema::{DynamicComponentStore, DynamicSchema, FieldType};
use big_head_ecs::{
    SystemDefinition, SystemKind, SystemRegistry, query_reader, resolve_entity, snapshot_reader,
};

use big_head_save::read::execute_get_component;

use big_head_query::{execute_dynamic_query_with_reader, parse_query};

// ── 线程局部世界访问 ────────────────────────────────────────────────────

thread_local! {
    static WORLD_PTR: Cell<*mut World> = const { Cell::new(std::ptr::null_mut()) };
    static REGISTRY_PTR: Cell<*mut ComponentActionRegistry> = const { Cell::new(std::ptr::null_mut()) };
    static SYSTEM_REGISTRY_PTR: Cell<*mut SystemRegistry> = const { Cell::new(std::ptr::null_mut()) };
    static SPARK_QUEUE_PTR: Cell<*mut VecDeque<Entity>> = const { Cell::new(std::ptr::null_mut()) };
    static LOG_EMITTER: RefCell<Option<LogEmitter>> = RefCell::new(None);
    static LOG_CAPTURE: RefCell<Vec<String>> = RefCell::new(Vec::new());
    static LOG_CAPTURE_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// RAII guard：eval 前设置线程局部指针，drop 时清理。
struct WorldGuard;

impl WorldGuard {
    fn set(
        world: &mut World,
        registry: &mut ComponentActionRegistry,
        system_registry: &mut SystemRegistry,
        spark_queue: Option<&mut VecDeque<Entity>>,
        emitter: Option<LogEmitter>,
    ) -> Self {
        WORLD_PTR.with(|p| {
            assert!(p.get().is_null(), "nested Rhai eval not allowed");
            p.set(world as *mut World);
        });
        REGISTRY_PTR.with(|p| p.set(registry as *mut ComponentActionRegistry));
        SYSTEM_REGISTRY_PTR.with(|p| p.set(system_registry as *mut SystemRegistry));
        SPARK_QUEUE_PTR.with(|p| {
            p.set(
                spark_queue
                    .map(|q| q as *mut VecDeque<Entity>)
                    .unwrap_or(std::ptr::null_mut()),
            );
        });
        LOG_EMITTER.with(|e| *e.borrow_mut() = emitter);
        WorldGuard
    }
}

impl Drop for WorldGuard {
    fn drop(&mut self) {
        WORLD_PTR.with(|p| p.set(std::ptr::null_mut()));
        REGISTRY_PTR.with(|p| p.set(std::ptr::null_mut()));
        SYSTEM_REGISTRY_PTR.with(|p| p.set(std::ptr::null_mut()));
        SPARK_QUEUE_PTR.with(|p| p.set(std::ptr::null_mut()));
        LOG_EMITTER.with(|e| *e.borrow_mut() = None);
    }
}

/// 在 Rhai 闭包内通过线程局部 emitter 发送日志。
fn rhai_emit(level: LogLevel, message: String) {
    LOG_EMITTER.with(|e| {
        let borrowed = e.borrow();
        emit_log(&*borrowed, "rhai", level, message.clone());
    });
    LOG_CAPTURE_ACTIVE.with(|active| {
        if active.get() {
            LOG_CAPTURE.with(|buf| {
                buf.borrow_mut().push(format!("[{:?}] {}", level, message));
            });
        }
    });
}

/// 在 guarded eval 内以可变引用访问线程局部的 World + Registry。
fn with_world<R>(f: impl FnOnce(&mut World, &mut ComponentActionRegistry) -> R) -> R {
    WORLD_PTR.with(|wp| {
        REGISTRY_PTR.with(|rp| {
            let world = wp.get();
            let registry = rp.get();
            assert!(!world.is_null(), "Rhai function called outside eval guard");
            unsafe { f(&mut *world, &mut *registry) }
        })
    })
}

fn with_world_ref<R>(f: impl FnOnce(&World, &ComponentActionRegistry) -> R) -> R {
    with_world(|w, r| f(w, r))
}

/// Register a host service without exposing the guarded raw pointers.
/// The callback must not recursively evaluate Rhai.
pub fn register_world_function(
    engine: &mut Engine,
    name: &'static str,
    handler: impl Fn(
        &mut World,
        &mut ComponentActionRegistry,
        serde_json::Value,
    ) -> Result<serde_json::Value, String>
    + Send
    + Sync
    + 'static,
) {
    engine.register_fn(
        name,
        move |input: Map| -> Result<Dynamic, Box<EvalAltResult>> {
            with_world(|world, registry| handler(world, registry, dynamic_to_json(input.into())))
                .map(json_to_dynamic)
                .map_err(|error| format!("{name}: {error}").into())
        },
    );
}

// ── JSON ↔ Dynamic 转换 ────────────────────────────────────────────────

/// 将 `serde_json::Value` 转为 Rhai `Dynamic`。
pub fn json_to_dynamic(value: serde_json::Value) -> Dynamic {
    match value {
        serde_json::Value::Null => Dynamic::UNIT,
        serde_json::Value::Bool(b) => Dynamic::from(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Dynamic::from(i)
            } else if let Some(f) = n.as_f64() {
                Dynamic::from(f)
            } else {
                Dynamic::from(n.to_string())
            }
        }
        serde_json::Value::String(s) => Dynamic::from(s),
        serde_json::Value::Array(arr) => {
            let rhai_arr: Array = arr.into_iter().map(json_to_dynamic).collect();
            Dynamic::from(rhai_arr)
        }
        serde_json::Value::Object(map) => {
            let mut rhai_map = Map::new();
            for (k, v) in map {
                rhai_map.insert(k.into(), json_to_dynamic(v));
            }
            Dynamic::from(rhai_map)
        }
    }
}

/// 将 Rhai `Dynamic` 转为 `serde_json::Value`。
pub fn dynamic_to_json(value: Dynamic) -> serde_json::Value {
    if value.is_unit() {
        serde_json::Value::Null
    } else if value.is_bool() {
        serde_json::Value::Bool(value.as_bool().unwrap())
    } else if value.is_int() {
        serde_json::json!(value.as_int().unwrap())
    } else if value.is_float() {
        serde_json::json!(value.as_float().unwrap())
    } else if value.is_string() {
        serde_json::Value::String(value.into_string().unwrap())
    } else if value.is_array() {
        let arr: Array = value.cast();
        serde_json::Value::Array(arr.into_iter().map(dynamic_to_json).collect())
    } else if value.is_map() {
        let map: Map = value.cast();
        let mut obj = serde_json::Map::new();
        for (k, v) in map {
            obj.insert(k.to_string(), dynamic_to_json(v));
        }
        serde_json::Value::Object(obj)
    } else {
        serde_json::Value::String(format!("{value}"))
    }
}

// ── 引擎构建 ───────────────────────────────────────────────────────────

/// 构建注册了所有 ECS 函数的 Rhai `Engine`。
pub fn build_ecs_engine() -> Engine {
    let mut engine = Engine::new();

    // take_head(text, n) -> String
    //
    // Small compatibility helper for older TickAction scripts. Rhai's built-in
    // string APIs are easy for agents to misremember, so expose the common
    // truncation operation directly.
    engine.register_fn("take_head", |text: &str, limit: i64| -> String {
        let limit = limit.max(0) as usize;
        text.chars().take(limit).collect()
    });

    // -- 核心 ECS 读取函数 --

    // all_entities() -> Array<i64>
    engine.register_fn("all_entities", || -> Array {
        with_world_ref(|world, _reg| {
            let mut entities = Array::new();
            for archetype in world.archetypes().iter() {
                for arch_entity in archetype.entities() {
                    let idx = arch_entity.id().index().index() as i64;
                    entities.push(Dynamic::from(idx));
                }
            }
            entities
        })
    });

    // entities_with(name: &str) -> Array<i64>
    engine.register_fn(
        "entities_with",
        |name: &str| -> Result<Array, Box<EvalAltResult>> {
            with_world_ref(|world, reg| {
                let store = reg.dynamic_store();
                let query = parse_query(world, name).map_err(|e| format!("entities_with: {e}"))?;
                let string_reader = query_reader(store);
                let json_reader = snapshot_reader(store);
                let results = execute_dynamic_query_with_reader(
                    world,
                    &query,
                    Some(&string_reader),
                    Some(&json_reader),
                );
                Ok(results
                    .into_iter()
                    .map(|r| Dynamic::from(r.entity.index().index() as i64))
                    .collect())
            })
        },
    );

    // has_component(eid: i64, name: &str) -> bool
    engine.register_fn("has_component", |eid: i64, name: &str| -> bool {
        with_world_ref(|world, reg| {
            let store = reg.dynamic_store();
            let comps = get_component_names(world, store, eid as u64);
            let name_short = name.rsplit("::").next().unwrap_or(name);
            comps.iter().any(|c| {
                let c_short = c.rsplit("::").next().unwrap_or(c);
                c_short == name_short
            })
        })
    });

    // get_component(eid: i64, name: &str) -> Dynamic
    engine.register_fn("get_component", |eid: i64, name: &str| -> Dynamic {
        with_world_ref(|world, reg| {
            let Ok(entity) = resolve_entity(world, eid as u64) else {
                return Dynamic::UNIT;
            };
            let store = reg.dynamic_store();
            if let Some(resolved) = store.resolve_name(name) {
                let entry_id = store.get_id(resolved).unwrap();
                if let Some(val) = dynamic_component::read(world, entity, entry_id) {
                    return json_to_dynamic(val);
                }
            }
            let (out, is_err) = execute_get_component(world, store, eid as u64, name);
            if is_err {
                return Dynamic::UNIT;
            }
            if let Some(colon_pos) = out.find(": ") {
                let json_str = &out[colon_pos + 2..];
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    return json_to_dynamic(val);
                }
            }
            Dynamic::from(out)
        })
    });

    // -- 写入函数 --

    // set_component(eid: i64, name: &str, val: Dynamic)
    engine.register_fn(
        "set_component",
        |eid: i64, name: &str, val: Dynamic| -> Result<(), Box<EvalAltResult>> {
            let json_val = dynamic_to_json(val);
            with_world(|world, reg| {
                let entity = resolve_entity(world, eid as u64)?;
                reg.patch_component(world, entity, name, &json_val.to_string())
                    .map_err(|e| format!("set_component: {e}").into())
            })
        },
    );

    // remove_component(eid: i64, name: &str)
    engine.register_fn(
        "remove_component",
        |eid: i64, name: &str| -> Result<(), Box<EvalAltResult>> {
            with_world(|world, reg| {
                let entity = resolve_entity(world, eid as u64)?;
                reg.remove_component_by_name(world, entity, name)
                    .map_err(|e| format!("remove_component: {e}").into())
            })
        },
    );

    // components_of(eid: i64) -> Array<String>
    engine.register_fn("components_of", |eid: i64| -> Array {
        with_world_ref(|world, reg| {
            let store = reg.dynamic_store();
            let names = get_component_names(world, store, eid as u64);
            names.into_iter().map(|n| Dynamic::from(n)).collect()
        })
    });

    // -- 查询和生命周期 --

    // query(dsl: &str) -> Array<Map>
    engine.register_fn("query", |dsl: &str| -> Result<Array, Box<EvalAltResult>> {
        with_world_ref(|world, reg| {
            let store = reg.dynamic_store();
            let query = parse_query(world, dsl).map_err(|e| format!("query: {e}"))?;
            let string_reader = query_reader(store);
            let json_reader = snapshot_reader(store);
            let results = execute_dynamic_query_with_reader(
                world,
                &query,
                Some(&string_reader),
                Some(&json_reader),
            );
            Ok(results
                .into_iter()
                .map(|r| {
                    let mut map = Map::new();
                    map.insert(
                        "_entity_id".into(),
                        Dynamic::from(r.entity.index().index() as i64),
                    );
                    for (name, value) in r.values {
                        let short = name.rsplit("::").next().unwrap_or(&name);
                        // Native query display values are Debug text; scripts need structured data.
                        if let Some(json) =
                            big_head_save::read::read_component_value(world, store, r.entity, &name)
                        {
                            map.insert(short.into(), json_to_dynamic(json));
                            continue;
                        }
                        match value {
                            Some(v) => {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&v) {
                                    map.insert(short.into(), json_to_dynamic(json));
                                } else {
                                    map.insert(short.into(), Dynamic::from(v));
                                }
                            }
                            None => {
                                map.insert(short.into(), Dynamic::UNIT);
                            }
                        }
                    }
                    Dynamic::from(map)
                })
                .collect())
        })
    });

    // spawn_entity(components: Map) -> i64
    engine.register_fn(
        "spawn_entity",
        |components: Map| -> Result<i64, Box<EvalAltResult>> {
            with_world(|world, reg| {
                let entity = world.spawn_empty().id();
                for (name, val) in components {
                    let name_str = name.to_string();
                    let json_val = dynamic_to_json(val);
                    let json_str = serde_json::to_string(&json_val).unwrap_or_default();
                    if let Err(e) = reg.insert_component(world, entity, &name_str, &json_str) {
                        world.despawn(entity);
                        return Err(format!(
                            "spawn_entity: component '{name_str}': {e}; new entity rolled back"
                        )
                        .into());
                    }
                }
                Ok(entity.index().index() as i64)
            })
        },
    );

    // despawn(eid: i64)
    engine.register_fn("despawn", |eid: i64| -> Result<(), Box<EvalAltResult>> {
        with_world(|world, _reg| {
            let entity = resolve_entity(world, eid as u64)?;
            world.despawn(entity);
            Ok(())
        })
    });

    // log(msg: &str)
    engine.register_fn("log", |msg: &str| {
        rhai_emit(LogLevel::Info, msg.to_string());
    });

    // -- 注册函数 --

    // define_component(name: &str, schema: Map)
    engine.register_fn("define_component", |name: &str, schema: Map| -> Result<(), Box<EvalAltResult>> {
        let mut fields = std::collections::BTreeMap::new();
        for (key, val) in schema {
            let field_name = key.to_string();
            let type_str = val.to_string().to_lowercase();
            let field_type = match type_str.as_str() {
                "string" => FieldType::String,
                "integer" | "int" => FieldType::Integer,
                "number" | "float" | "f64" => FieldType::Number,
                "boolean" | "bool" => FieldType::Boolean,
                "any" => FieldType::Any,
                other => {
                    return Err(format!(
                        "define_component '{}': unknown type '{}' for field '{}', use string/integer/number/boolean/any",
                        name, other, field_name
                    ).into());
                }
            };
            fields.insert(field_name, field_type);
        }
        with_world(|world, reg| {
            match reg.define_dynamic_component(world, name, DynamicSchema { fields }) {
                Ok(_) => Ok(rhai_emit(LogLevel::Info, format!("defined component '{}'", name))),
                Err(e) => Err(format!("define_component '{name}': {e}").into()),
            }
        })
    });

    // define_system(id: &str, source: &str)
    engine.register_fn(
        "define_system",
        |id: &str, source: &str| -> Result<(), Box<EvalAltResult>> {
            check_syntax(&Engine::new(), source)?;
            SYSTEM_REGISTRY_PTR.with(|srp| {
                let ptr = srp.get();
                assert!(!ptr.is_null(), "define_system called outside eval guard");
                let sys_reg = unsafe { &mut *ptr };
                let def = SystemDefinition {
                    id: id.to_string(),
                    description: String::new(),
                    system_prompt: std::borrow::Cow::Borrowed(""),
                    kind: SystemKind::RhaiScript {
                        source: source.to_string(),
                    },
                    tool_names: Vec::new(),
                    max_iterations: 1,
                    is_tool: false,
                };
                match sys_reg.register(def) {
                    Ok(()) => Ok(rhai_emit(
                        LogLevel::Info,
                        format!("defined system '{}'", id),
                    )),
                    Err(e) => Err(format!("define_system '{id}': {e}").into()),
                }
            })
        },
    );

    // define_tool(id: &str, description: &str, source: &str)
    engine.register_fn(
        "define_tool",
        |id: &str, description: &str, source: &str| -> Result<(), Box<EvalAltResult>> {
            check_syntax(&Engine::new(), source)?;
            SYSTEM_REGISTRY_PTR.with(|srp| {
                let ptr = srp.get();
                assert!(!ptr.is_null(), "define_tool called outside eval guard");
                let sys_reg = unsafe { &mut *ptr };
                let def = SystemDefinition {
                    id: id.to_string(),
                    description: description.to_string(),
                    system_prompt: std::borrow::Cow::Borrowed(""),
                    kind: SystemKind::RhaiScript {
                        source: source.to_string(),
                    },
                    tool_names: Vec::new(),
                    max_iterations: 1,
                    is_tool: true,
                };
                match sys_reg.register(def) {
                    Ok(()) => Ok(rhai_emit(LogLevel::Info, format!("defined tool '{}'", id))),
                    Err(e) => Err(format!("define_tool '{id}': {e}").into()),
                }
            })
        },
    );

    // push_spark(text: &str) — spawn Spark entity + 推入 spark_queue
    engine.register_fn(
        "push_spark",
        |text: &str| -> Result<(), Box<EvalAltResult>> {
            SPARK_QUEUE_PTR.with(|sqp| {
                let ptr = sqp.get();
                if ptr.is_null() {
                    return Err("push_spark: spark_queue not available in this context".into());
                }
                with_world(|world, _reg| {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let spark = Spark {
                        text: text.to_string(),
                        timestamp: now,
                        status: SparkStatus::Pending,
                        cycle_id: 0,
                        result_summary: String::new(),
                        ..Default::default()
                    };
                    let entity =
                        unsafe { big_head_ecs::execution::enqueue(world, &mut *ptr, spark, false) };
                    rhai_emit(
                        LogLevel::Info,
                        format!(
                            "push_spark: spawned entity {:?} with text '{}'",
                            entity, text
                        ),
                    );
                });
                Ok(())
            })
        },
    );

    // timestamp() -> i64 (unix 秒)
    engine.register_fn("timestamp", || -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    });

    // ─── HTTP bridge to localhost services (e.g. big-head-view :3030) ───
    //
    // Lets Rhai scripts (and therefore TickActions, define_tool handlers,
    // and define_system bodies) call out to the view server's snapshot API
    // so an agent can introspect frontend truth without a WebSocket.
    //
    // Backed by `curl` to avoid pulling reqwest into the script engine
    // (single dependency, well-known flags, 5-second timeout by default).
    engine.register_fn("http_get", |url: &str| -> String {
        use std::process::Command;
        let out = Command::new("curl")
            .args(["-s", "--max-time", "5", url])
            .output();
        match out {
            Ok(o) => {
                let mut s = String::from_utf8_lossy(&o.stdout).to_string();
                if s.len() > 32_000 {
                    s.truncate(32_000);
                    s.push_str("\n[... truncated at 32KB ...]");
                }
                s
            }
            Err(e) => format!("[http_get error: {e}]"),
        }
    });

    engine.register_fn("http_get_status", |url: &str| -> String {
        use std::process::Command;
        let out = Command::new("curl")
            .args([
                "-s",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "--max-time",
                "5",
                url,
            ])
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(e) => format!("-1:{e}"),
        }
    });

    engine
}

/// 辅助函数：获取实体的组件名称列表。
fn get_component_names(
    world: &World,
    _store: &DynamicComponentStore,
    entity_id: u64,
) -> Vec<String> {
    let Ok(entity) = resolve_entity(world, entity_id) else {
        return Vec::new();
    };
    let Ok(Some(location)) = world.entities().get(entity) else {
        return Vec::new();
    };
    let Some(archetype) = world.archetypes().get(location.archetype_id) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for &cid in archetype.components().iter() {
        if let Some(info) = world.components().get_info(cid) {
            let full = info.name().to_string();
            let short = full.rsplit("::").next().unwrap_or(&full).to_string();
            names.push(short);
        }
    }
    names
}

/// 在可访问 ECS 世界的上下文中执行 Rhai 脚本。
pub fn eval_with_world(
    engine: &Engine,
    source: &str,
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    system_registry: &mut SystemRegistry,
    spark_queue: Option<&mut VecDeque<Entity>>,
    emitter: Option<LogEmitter>,
) -> Result<Dynamic, Box<EvalAltResult>> {
    let _guard = WorldGuard::set(world, registry, system_registry, spark_queue, emitter);
    engine.eval::<Dynamic>(source)
}

/// 类似 [`eval_with_world`]，但将所有 `log()` 输出捕获到缓冲区。
pub fn eval_with_world_captured(
    engine: &Engine,
    source: &str,
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    system_registry: &mut SystemRegistry,
    spark_queue: Option<&mut VecDeque<Entity>>,
    emitter: Option<LogEmitter>,
) -> (Result<Dynamic, Box<EvalAltResult>>, Vec<String>) {
    eval_with_world_captured_args(
        engine,
        source,
        world,
        registry,
        system_registry,
        spark_queue,
        emitter,
        None,
    )
}

/// Evaluate a tool with per-call JSON arguments in a fresh Rhai scope.
pub fn eval_with_world_captured_args(
    engine: &Engine,
    source: &str,
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    system_registry: &mut SystemRegistry,
    spark_queue: Option<&mut VecDeque<Entity>>,
    emitter: Option<LogEmitter>,
    args: Option<&serde_json::Value>,
) -> (Result<Dynamic, Box<EvalAltResult>>, Vec<String>) {
    let args = match args {
        Some(value) if value.is_object() => value.clone(),
        None => serde_json::json!({}),
        Some(_) => return (Err("args must be a JSON object".into()), Vec::new()),
    };
    let _guard = WorldGuard::set(world, registry, system_registry, spark_queue, emitter);
    LOG_CAPTURE_ACTIVE.with(|a| a.set(true));
    LOG_CAPTURE.with(|buf| buf.borrow_mut().clear());
    let mut scope = rhai::Scope::new();
    scope.push("args", json_to_dynamic(args));
    let result = engine.eval_with_scope::<Dynamic>(&mut scope, source);

    let logs = LOG_CAPTURE.with(|buf| buf.borrow().clone());
    LOG_CAPTURE_ACTIVE.with(|a| a.set(false));

    (result, logs)
}

/// Evaluate a cached AST with ECS access and per-step arguments, discarding frame logs.
pub fn eval_ast_with_world_args(
    engine: &Engine,
    ast: &rhai::AST,
    world: &mut World,
    registry: &mut ComponentActionRegistry,
    system_registry: &mut SystemRegistry,
    args: &serde_json::Value,
) -> Result<Dynamic, Box<EvalAltResult>> {
    let _guard = WorldGuard::set(world, registry, system_registry, None, None);
    LOG_CAPTURE_ACTIVE.with(|a| a.set(true));
    LOG_CAPTURE.with(|buf| buf.borrow_mut().clear());
    let mut scope = rhai::Scope::new();
    scope.push("args", json_to_dynamic(args.clone()));
    let result = engine.eval_ast_with_scope::<Dynamic>(&mut scope, ast);
    LOG_CAPTURE_ACTIVE.with(|a| a.set(false));
    LOG_CAPTURE.with(|buf| buf.borrow_mut().clear());
    result
}

/// 编译校验 Rhai 脚本语法，不执行。
pub fn check_syntax(engine: &Engine, source: &str) -> Result<(), String> {
    engine
        .compile(source)
        .map(|_| ())
        .map_err(|e| format!("Rhai syntax error: {}", e))
}
