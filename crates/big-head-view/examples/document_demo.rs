//! Isolated document preview. No ECS world or model is touched.
use big_head_view::{ViewChannel, ViewServer, normalize_document};
use serde_json::json;

#[tokio::main]
async fn main() {
    let port = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3049);
    let (channel, sink) = ViewChannel::new();
    let command = normalize_document(&json!({
        "id":"conversation","title":"讓對話自然，讓事情落地",
        "summary":"你只需要講想做甚麼。實作細節由 agent 安排，重要選擇留給你；做過的事情，則有記錄可以查。",
        "sections":[
            {"heading":"一句話開始","body":"例如：幫我在 ~/Desktop/p 建一個讀書筆記項目，先用最簡單的版本。\n\n這已經是開始工作的授權。Agent 應該選擇合理的做法、建立項目，再確認結果，而不是再問一次要不要開始。"},
            {"heading":"甚麼時候才需要選擇？","body":"如果缺少會影響結果的重要資料，它會把問題放到 Questions。你可以選一項，也可以寫自己的答案。","bullets":["一般的實作細節，由 agent 決定。","你的回答會連回原本的工作。","未回答之前，依賴這個決定的操作不會繼續。"]},
            {"heading":"長內容，有一個可以慢慢讀的地方","body":"方案、解釋與學習筆記可以放在這樣的文件頁面。聊天裡留下最重要的一兩句話，你想了解細節時，再打開閱讀。\n\n這個預覽只展示閱讀介面，沒有建立項目或修改你的日常世界。"}
        ]
    })).unwrap();
    channel.send(serde_json::from_value(command).unwrap());
    println!("Document preview: http://127.0.0.1:{port}/#panel=document-conversation");
    ViewServer::new(sink, port).run().await.unwrap();
}
