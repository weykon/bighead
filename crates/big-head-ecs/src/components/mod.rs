//! ECS 组件 —— agent 运行时的数据组件。

mod agent;
mod audit;
mod managed_process;
mod spark;
mod tick_action;
mod tool_history;

pub use agent::{AgentProfile, AgentState};
pub use audit::{ActionRecord, TransactionRecord};
pub use managed_process::ManagedProcess;
pub use spark::{Spark, SparkStatus};
pub use tick_action::TickAction;
pub use tool_history::{ToolHistory, ToolHistoryEntry};
