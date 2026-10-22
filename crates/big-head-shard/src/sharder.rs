//! 分片器 —— 切分、分片数计算、prompt 模板展开。

/// 分片配置。
#[derive(Debug, Clone)]
pub struct ShardConfig {
    /// 重叠窗口大小（overlap 策略，默认 50）
    pub default_overlap: usize,
    /// 最大分片数（防无限分片）
    pub max_shards: usize,
    /// 安全系数，计算 remain 时留余量（默认 0.9）
    pub safety_margin: f64,
}

impl Default for ShardConfig {
    fn default() -> Self {
        Self {
            default_overlap: 50,
            max_shards: 20,
            safety_margin: 0.9,
        }
    }
}

/// Agent 指定的切分策略。
#[derive(Debug, Clone)]
pub enum ShardStrategy {
    /// 重叠窗口 — 相邻分片共享 overlap 行，保证跨边界上下文不被丢失
    Overlap { overlap_size: usize },
    /// 指定行号范围 — Agent 精确控制
    CustomRanges { ranges: Vec<(usize, usize)> },
    /// 头尾完整 + 中间切片 — 适用于日志类内容
    HeadTail {
        head_lines: usize,
        tail_lines: usize,
    },
    /// Agent 自然语言描述切分方式 — 基础设施基于描述执行切分
    Custom { description: String },
}

/// 分片器。
#[derive(Debug, Clone)]
pub struct Sharder {
    config: ShardConfig,
}

impl Default for Sharder {
    fn default() -> Self {
        Self::new(ShardConfig::default())
    }
}

impl Sharder {
    pub fn new(config: ShardConfig) -> Self {
        Self { config }
    }

    /// 返回分片配置的引用。
    pub fn config(&self) -> &ShardConfig {
        &self.config
    }

    /// 根据 token 预算计算需要分成几份。
    ///
    /// `remain = (model_limit - used) * safety_margin`
    /// `n = ceil(content_tokens / remain)`
    pub fn calculate_shards(
        &self,
        model_limit: usize,
        used: usize,
        content_tokens: usize,
    ) -> usize {
        let remain =
            ((model_limit.saturating_sub(used)) as f64 * self.config.safety_margin) as usize;
        if remain == 0 {
            return self.config.max_shards;
        }
        let n = (content_tokens + remain - 1) / remain;
        n.clamp(1, self.config.max_shards)
    }

    /// 按策略切分文本。
    pub fn split(&self, content: &str, strategy: &ShardStrategy) -> Vec<String> {
        match strategy {
            ShardStrategy::Overlap { overlap_size } => self.split_overlap(content, *overlap_size),
            ShardStrategy::CustomRanges { ranges } => self.split_ranges(content, ranges),
            ShardStrategy::HeadTail {
                head_lines,
                tail_lines,
            } => self.split_head_tail(content, *head_lines, *tail_lines),
            ShardStrategy::Custom { .. } => {
                // Custom 策略保留整段内容，让 agent 在 ECS 中显式决定后续索引方式。
                vec![content.to_string()]
            }
        }
    }

    // ── 内部切分方法 ──────────────────────────────────────────

    fn split_overlap(&self, content: &str, overlap_size: usize) -> Vec<String> {
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        if total == 0 {
            return vec![String::new()];
        }

        // 估算合适的分片大小：尽量少分片，每个分片不超过 safe 大小
        let effective_overlap = overlap_size.min(total / 2);
        let chunk_lines = self
            .chunk_size(total, effective_overlap)
            .max(effective_overlap + 1);

        let mut chunks = Vec::new();
        let mut start = 0usize;
        while start < total {
            let end = (start + chunk_lines).min(total);
            let chunk = lines[start..end].join("\n");
            chunks.push(chunk);

            if end >= total {
                break;
            }
            start = end.saturating_sub(effective_overlap);
        }
        chunks
    }

    fn split_ranges(&self, content: &str, ranges: &[(usize, usize)]) -> Vec<String> {
        let lines: Vec<&str> = content.lines().collect();
        ranges
            .iter()
            .map(|&(start, end)| {
                let end = end.min(lines.len());
                if start >= end {
                    String::new()
                } else {
                    lines[start..end].join("\n")
                }
            })
            .collect()
    }

    fn split_head_tail(&self, content: &str, head_lines: usize, tail_lines: usize) -> Vec<String> {
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        if total <= head_lines + tail_lines {
            return vec![content.to_string()];
        }

        let mut chunks = Vec::new();

        // 头部完整
        chunks.push(lines[..head_lines].join("\n"));

        // 中间切片
        let middle = &lines[head_lines..total - tail_lines];
        let middle_len = middle.len();
        let effective_overlap = 20usize;
        let chunk_lines = self.chunk_size(middle_len, effective_overlap);

        let mut start = 0usize;
        while start < middle_len {
            let end = (start + chunk_lines).min(middle_len);
            chunks.push(middle[start..end].join("\n"));
            if end >= middle_len {
                break;
            }
            start = end.saturating_sub(effective_overlap);
        }

        // 尾部完整
        chunks.push(lines[total - tail_lines..].join("\n"));

        chunks
    }

    /// 估算合适的分片行数。
    fn chunk_size(&self, total_lines: usize, _overlap: usize) -> usize {
        // 尽量分成不超过 max_shards 份
        let min_chunks = (total_lines as f64 / self.config.max_shards as f64).ceil() as usize;
        min_chunks.max(100) // 每份至少 100 行
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_overlap_advances_and_reaches_last_line() {
        let content = (0..1000)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let chunks =
            Sharder::default().split(&content, &ShardStrategy::Overlap { overlap_size: 500 });
        assert!(chunks.len() <= 1000);
        assert!(chunks.last().unwrap().ends_with("999"));
    }

    fn test_content() -> String {
        (0..500)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_calculate_shards() {
        let sharder = Sharder::default();
        // 200K limit, 50K used, 600K content → (200K-50K)*0.9 = 135K remain
        // 600K / 135K = 4.44 → 5 shards
        let n = sharder.calculate_shards(200_000, 50_000, 600_000);
        assert_eq!(n, 5);
    }

    #[test]
    fn test_calculate_shards_clamped() {
        let sharder = Sharder::default();
        // remain=0 → clamps to max_shards
        let n = sharder.calculate_shards(1000, 1000, 100_000);
        assert_eq!(n, sharder.config.max_shards);
    }

    #[test]
    fn test_split_overlap() {
        let sharder = Sharder::default();
        let content = test_content(); // 500 lines
        let chunks = sharder.split(&content, &ShardStrategy::Overlap { overlap_size: 50 });
        assert!(chunks.len() > 1);
        // 检查重叠：分片 1 的第一行应该出现在分片 0
        let first_of_chunk1 = chunks[1].lines().next().unwrap();
        assert!(chunks[0].contains(first_of_chunk1));
    }

    #[test]
    fn test_split_overlap_empty() {
        let sharder = Sharder::default();
        let chunks = sharder.split("", &ShardStrategy::Overlap { overlap_size: 50 });
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].is_empty());
    }

    #[test]
    fn test_split_ranges() {
        let sharder = Sharder::default();
        let content = test_content();
        let chunks = sharder.split(
            &content,
            &ShardStrategy::CustomRanges {
                ranges: vec![(0, 10), (100, 110), (490, 500)],
            },
        );
        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].starts_with("line 0"));
        assert!(chunks[1].starts_with("line 100"));
    }

    #[test]
    fn test_split_head_tail() {
        let sharder = Sharder::default();
        let content = test_content();
        let chunks = sharder.split(
            &content,
            &ShardStrategy::HeadTail {
                head_lines: 20,
                tail_lines: 30,
            },
        );
        assert!(chunks.len() >= 3);
        // 第一块是头部
        assert!(chunks[0].starts_with("line 0"));
        assert!(chunks[0].contains("line 19"));
        // 最后一块是尾部
        assert!(chunks.last().unwrap().contains("line 499"));
    }

    #[test]
    fn test_split_custom() {
        let sharder = Sharder::default();
        let content = "single chunk content";
        let chunks = sharder.split(
            content,
            &ShardStrategy::Custom {
                description: "no split needed".into(),
            },
        );
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], content);
    }
}
