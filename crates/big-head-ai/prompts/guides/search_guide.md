# 联网搜索（search）

**统一入口** —— 一个 `search` tool, 4 个 `kind` 值:

| `kind` | 来源 | 适用场景 |
|--------|------|---------|
| `wiki` | Wikipedia (en / zh,按 query 嘅 CJK 自动拣) | 概念/人物/历史/术语 —— 权威、结构化 |
| `web`  | 非中文: Bing → Mojeek → DDG；中文/CJK: Baidu → cn.bing → Mojeek → DDG | 时效信息、版本号、新闻、博客、产品规格 |
| `code` | GitHub Search API (按 stars 排序) | 搵开源库、比较 stars/forks、生态成熟度 |
| `news` | Hacker News Algolia API | 社区评价、Show HN、技术圈点睇 |

**经验法则** —— 按用户问嘅嘢分类:

| 用户问紧... | 用 `search` 加... | 例子 |
|-------------|---------------------|------|
| "X 是什么 / 谁发明的 / 原理是" | `kind: "wiki"` | "Tesla Model Y" |
| "X 嘅维基条目 (中文)" | `kind: "wiki"`(自动 zh) | "特斯拉 Model Y" |
| "2026 Model Y 续航 / Rust 1.78 特性" | `kind: "web"` | "rust 1.78 let chains" |
| "X 嘅开源库 / 框架" | `kind: "code"` | "rust async runtime language:rust" |
| "HN 睇法 / Show HN" | `kind: "news"` | "tokio runtime 2026" |
| "X 嘅中文新闻" | `kind: "web"`(自动加 Baidu) | "特斯拉 Model Y 2026" |

**核心区别**:
- `wiki` → 已知事实(权威)
- `web`  → 实时信息(多源 fallback)
- `code` → 找代码库(结构化 metadata)
- `news` → 社区情绪(HN signal)

## 调用格式

```json
{ "query": "Tesla Model Y", "kind": "wiki" }
{ "query": "rust async tokio", "kind": "code", "limit": 5 }
{ "query": "特斯拉 Model Y 2026", "kind": "web" }
{ "query": "tokio runtime 2026", "kind": "news", "limit": 10 }
```

## 返回(envelope)

成功:
```json
{
  "success": true,
  "query": "...",
  "kind": "web",
  "provider_used": "mojeek",
  "result_count": 5,
  "results": [
    { "kind": "web", "title": "...", "url": "...", "description": "...", "position": 1 }
  ]
}
```

每个 hit 嘅 `kind` 决定 shape:
- `"web"` → `{ title, url, description, position }`
- `"wiki"` → `{ title, url, snippet, lang, position }`
- `"repo"` (code) → `{ full_name, html_url, description, language, stars, forks, position }`
- `"story"` (news) → `{ title, url, author, points, num_comments, created_at, position }`

**根据 hit 嘅 `kind` tag 分支处理**,唔使再发多一个 query。

## 行为规则

1. **不要重复同一 query 超过 3 次** —— 如果第一次没结果，换关键词或换 `kind`。
2. **`result_count == 0` 怎么办**:
   - 先看是否 query 太长 / 太口语化(试"特斯拉 Model Y 2026"而不是"最新款特斯拉 Model Y 续航怎么样")
   - 再试换 `kind`(wiki 没结果就试 web;web 没结果就试 code/news)
3. **搜索结果只是索引,不是正文读取**:
   - 可以用 hit 的 `title/url/description/snippet` 判断下一步和做轻量摘要。
   - 不要声称已经阅读了网页正文。
   - 需要精确引用、法规、价格、版本、发布日期、长文细节时,从搜索结果中选择 URL,再调用 `fetch_url` 读取正文。
4. **回答时保留来源线索**:提到搜索得到的信息时,带上命中的 `title` 或 `url`,让用户知道依据来自哪个结果。
5. **时效问题用 `web`**:用户问“最新/今天/当前/价格/版本/新闻”时,不要用 `wiki` 当最终依据；先用 `web`,必要时多换一组关键词交叉确认。

## 读取搜索结果正文

`search` 找候选 URL，`fetch_url` 读取某个 URL 的正文。

推荐流程：

1. `search({ "query": "...", "kind": "web" })`
2. 从结果中选 1-3 个最相关 URL
3. `fetch_url({ "url": "...", "mode": "readable", "limit": 8000 })`
4. 用返回的 `title/final_url/text/links/truncated` 判断是否继续读取或换源
5. 回答时带上 `title` 或 `url`

`fetch_url` 返回 envelope:

```json
{
  "success": true,
  "url": "...",
  "final_url": "...",
  "backend": "native",
  "title": "...",
  "content_type": "text/html",
  "text": "清洗后的正文...",
  "links": [{ "text": "Docs", "url": "https://..." }],
  "truncated": false,
  "bytes_downloaded": 12345
}
```

`mode`:
- `readable` 默认。HTML 会清洗成正文文本。
- `raw` 返回响应体文本，适合 JSON、纯文本、调试。

如果运行时配置了 `BIG_HEAD_CRAWL4AI_CMD`，`fetch_url` 会优先调用该命令读取 URL，并在失败时 fallback 到 native 后端。

## 配合 shard_content

如果搜索结果数很多或单条 description 很长,先把返回的 JSON 字符串传给
`shard_content content=<JSON字符串> strategy=overlap paradigm=<搜索主题>`,
把原始结果投影到 ECS。之后按需查询 ShardData,再显式创建事实、索引或关系。

`fetch_url` 正文也可能很大。大结果会被统一 ContextGate 投影到 ECS；看到 projection notice 后，使用
`get_shards` / `get_shard_content` 逐段读取。

**结果已结构化**:`code` 嘅 `stars` / `language` / `forks` 同 `news` 嘅 `points` / `num_comments`
都已经系数字 —— 直接当作 signal,唔使再 `shard_content`。

## Rate limit / HTTP 代理

每个源对一个 cycle 3-5 次都安全。**agent 不需要追踪具体源顺序**，只需要：
- `result_count == 0` 超过 2 次 → 换工具或换 query
- 看到 0 results 时**不要**在同一个 query 上硬试 5 次

如果运行环境需要代理（比如中国大陆访问海外搜索源），启动 binary 时设环境变量即可：

```bash
BIG_HEAD_PROXY=http://127.0.0.1:7897 cargo run          # 项目专用入口（最高优先级）
# 或沿用标准 env（优先级低于 BIG_HEAD_PROXY）：
https_proxy=http://127.0.0.1:7897 cargo run
http_proxy=http://127.0.0.1:7897 cargo run
```

所有 HTTP 工具（LLM API、`search`、`fetch_url`）都自动走这个 proxy。**agent 不需要知道 proxy 的存在**。
