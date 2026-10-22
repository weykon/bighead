//! Real-model smoke run in a fresh world; never opens local-default.
use big_head_ai::{Agent, ApiProvider};
use big_head_loop::AgentRuntime;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let agent = Agent::from_env(ApiProvider::MiniMax)
        .ok_or("MiniMax_API_KEY is not configured in the environment or .env")?;
    let mut runtime = AgentRuntime::new(agent);
    let directory = std::path::PathBuf::from("outputs/dream-practice").join(&runtime.session_id);
    std::fs::create_dir_all(&directory)?;
    let checkpoint = directory.join("checkpoint.json");
    runtime.set_checkpoint_path(&checkpoint);
    println!("Isolated practice checkpoint: {}", checkpoint.display());
    let result = runtime.run_dream_practice(
        "Complete a small real Dream-RSI loop using only your two tools. Create experiment positive-sum: sum strictly positive integer elements of input. Cases: [-2,3,0,4] -> 7; [] -> 0; [-3,-1] -> 0; [5,5,-2] -> 10. budget=4, workers=1. In the first round follow next/submit until all four baseline attempts are recorded, even if an earlier candidate passes, to measure the fixed controller. Inspect the completed tree. Then propose and evaluate 1 or 2 exploration policies grounded in its actual outcomes; preserve quality and reduce unnecessary attempts. Let the runtime decide adoption. Run one more real online round with the deployed policy, letting it stop or reach budget. End after these two rounds and one dreaming call; never start a third round. Report measured attempt counts and scores and whether adoption occurred. Do not claim general intelligence or unseen-task improvement. Use the latest revision from tool receipts where required. Scripts and policies must be authored by you; do not fabricate scores.".into()
    ).await;
    runtime.checkpoint(&checkpoint)?;
    let inspection = runtime.inspect_dream_experiments()?;
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&inspection)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&inspection)?);
    match result {
        Ok(reply) => println!("Agent report: {reply}"),
        Err(error) => return Err(error.into()),
    }
    let experiment = &inspection["experiments"][0];
    if experiment["completed_rounds"] != 2
        || !experiment["active_attempts"].is_null()
        || experiment["last_dream"].is_null()
    {
        return Err(
            "Agent ended before completing two rounds and a replay; inspect saved evidence".into(),
        );
    }
    Ok(())
}
