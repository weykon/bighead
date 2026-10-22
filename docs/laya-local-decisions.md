# Local Laya ECS decisions

This integration provides three layers: advisory `laya_decide`, fixed-workflow
`laya_assess`, and the versioned `laya_run_spec` closed loop. DecisionSpec v2 can
dispatch one bounded existing action only under explicit runtime authority, then reads
state back and records the outcome. It does not replace MiniMax, generate Rhai, define
new capabilities or install a TickAction. Every write still uses the normal audited
ECS tool path. See [DecisionSpec v2](laya-decision-spec-v2.md) for the full contract.

The next stage adds `laya_assess`, `inspect_laya_shadow` and bounded automatic
TickAction shadow observations. See [shadow workflows](laya-shadow-workflows.md)
for activation, evidence collection, limits and the measured decision-quality gap.

## Install and start

From the repository root:

```sh
uv venv --python 3.12 .big-head/laya-venv
uv pip install --python .big-head/laya-venv/bin/python -r integrations/laya/requirements.lock
.big-head/laya-venv/bin/python integrations/laya/service.py
```

The default checkpoint is `convaiinnovations/laya-multilingual`, revision
`052592a15d198d9ad47da779604259b10b47b7aa`. Only its five runtime files are downloaded.
The default backend is native MLX on the Apple GPU. The model is loaded once.
`/health` and every prediction report `backend`, `backend_version`, `precision`,
`laya_version` and device. MLX failures do not silently switch to PyTorch.
First inference may include GPU warmup.
Upstream T4 timings are not Mac benchmarks.

An already downloaded checkpoint can be used offline:

```sh
.big-head/laya-venv/bin/python integrations/laya/service.py --model-dir .big-head/laya-model
```

Service URL: `http://127.0.0.1:3031`, separate from the view on 3030. Choose another
port with `--port`; set `BIGHEAD_LAYA_URL=http://127.0.0.1:<port>` on Big Head to match.
The Rust client accepts only literal IPv4 loopback HTTP and does not follow redirects
or use proxies. The service binds only to loopback and accepts no browser-origin POSTs.
Model loading can use the operator's `HF_ENDPOINT`; that is a download setting, not
a destination for world observations. Prediction is local after startup.

Rebuild and normally restart Big Head to expose the new tool and `guide_laya`.
Do not interrupt a live checkpoint save or edit a running world's files.

## Advisory v1 contract

```json
{
  "goal": "Set the existing counter to 1",
  "state": {"entity": 42, "components": {"Counter": {"count": 0}}},
  "candidates": [
    {"operation": "modify_component", "description": "Update existing component data"},
    {"operation": "spawn_entity", "description": "Create another entity"}
  ]
}
```

The entity above is illustrative; real calls must use observed facts. Read relevant
state first, and use exact currently available tool names. The runtime checks the
candidate names against the current stage's tool catalog. Candidate names are not
proof of semantic preconditions or valid arguments.

- 1..7 candidates, unique operation names; `defer` is added by the service.
- Goal <=512 characters, state <=8192 bytes, descriptions <=256 characters.
- Full v1 request <=16 KiB. The shared HTTP body limit is 32 KiB. These are transport
  limits, not promises of model fit.
- Actual tokenizer budgets are checked before inference: no clipped options,
  instructions or state. Shorten or split a rejected decision.
- Only one prediction runs at a time. Busy requests return 503; Rust calls time out
  after 30 seconds and bound the response to 64 KiB.
- Advice always reports `executed:false`, `behavior_verified:false`,
  `confidence_calibrated_for_ecs:false`, and `input_truncated:false`.
- Local service failure is identified separately from LLM provider/API failure.

The checkpoint has no demonstrated ECS training or calibration. It can be confident
and wrong. Do not set a confidence threshold that automatically permits writes.
Creating or updating a system still requires its existing contracts and read-back
verification. No weights are trained by this integration.

## Evaluation and records

```sh
.big-head/laya-venv/bin/python -m unittest discover -s integrations/laya -p 'test_*.py'
.big-head/laya-venv/bin/python integrations/laya/evaluate.py
CARGO_INCREMENTAL=0 cargo test -p big-head-loop laya --lib -- --include-ignored --test-threads=1
```

`cases.jsonl` contains 12 synthetic English/Chinese decisions with expected operation
labels. The evaluator reports every case, errors, accuracy and warm median end-to-end
latency in `outputs/laya/baseline.json`. Expected answers are never sent to the model.
Incorrect predictions are measured outcomes, not transport failures. This small set
is a smoke baseline, not a production qualification or a training dataset.

`outputs/laya/decisions-YYYY-MM-DD.jsonl` records compact requests, prediction ids,
model identity, scores and device. Records stay outside ECS checkpoints. V1 entries
include `verified_outcome:null`; v2 appends a separately linked runtime outcome after
policy/execution/read-back. Both remain `training_eligible:false`; the model's own
choice is never used as the correct label. Existing ActionRecord auditing links the
outer call and any nested v2 action/read-back to the enclosing Big Head transaction.
No autonomous DecisionSpec polling or automatic record deletion is installed.

## DecisionSpec v2 execution

Execute mode is available by default and needs no environment flag. A call still acts
only when it passes all Rust-side validation and deterministic policy checks. This does
not turn `laya_decide` or the background observer into actors. Select `mode:"shadow"`
for a non-mutating evaluation of the same spec. The local service remains on 3031 and
the web view remains on 3030.

For later domain training, collect independently verified outcomes and separate
sessions/tasks into training and held-out sets. Preserve failures, deferrals and
incomplete tasks. Keep these baseline cases out of training when using them to compare
versions. Upstream supplies a CUDA/Kaggle fine-tuning notebook; local MPS training and
production deployment are separate work, not claimed by this first integration.

## First measurement: 2026-09-20

On this MacBook Pro (M3 Max, 40-core GPU, 64 GB unified memory), the actual service
loaded on `mps`. The 12 synthetic cases all returned predictions without truncation.
Only **2/12 were correct (16.7%)**; the warm median HTTP round-trip was **53.01 ms**.
This is a small diagnostic set, not a general benchmark. Several wrong predictions
confused `remove_component` with `despawn_entity`; the base checkpoint is unsuitable
for autonomous ECS execution. Both successful cases selected `get_entity` to verify
state. Keep the failed cases in the evidence; do not optimize prompts against this
set and then present it as independent validation.

The pinned multilingual weights were downloaded through `hf-mirror.com` because the
Hub API was rate-limited and its mirror HEAD metadata was incompatible with the SDK.
The final local download was verified against the resolver's SHA-256:
`9d628fd971b700382ac6f65920a86f149777b2e748e0c955fb3b19695aa8f204`.
Serving with `--model-dir .big-head/laya-model` was tested with both
`HF_HUB_OFFLINE=1` and `TRANSFORMERS_OFFLINE=1`. No network is needed for inference.
Local responses identify the weight digest and the loaded decision configuration
digest. The Python environment and checkpoint occupy approximately 548 MiB and
647 MiB respectively at the initial PyTorch installation. The current
`requirements.lock` installs the MLX runtime without PyTorch or Transformers.

## Backend selection

`laya_decide` and port 3031 keep the same contract when the service backend changes.
Switching this Python service does not require a Big Head rebuild/restart. The same
original checkpoint directory can be loaded by either backend; no second weight
download or conversion is required. The MLX path, including token-budget validation,
does not import `torch` or `transformers`.

`--device gpu` (or the backwards-compatible `mps` alias) selects the MLX GPU;
`--device cpu` selects the MLX CPU. `--dtype float16` or `--dtype float32` chooses
MLX arithmetic precision; **float16 is the default**. PyTorch is available only through explicit selection:

```sh
uv pip install --python .big-head/laya-venv/bin/python -r integrations/laya/requirements-torch.txt
.big-head/laya-venv/bin/python integrations/laya/service.py --backend torch --device mps --model-dir .big-head/laya-model
```

Stop the previous service before starting another on the same port. The existing
virtualenv retains the original Torch packages for fallback, but new default installs
from `requirements.lock` contain only MLX dependencies.

## MLX switch measurement: 2026-09-21

The same 12 cases, checkpoint weight hash, configuration and input token IDs were
compared locally before replacing the live service. Requests were sequential, with
no overlapping GPU benchmark runs. These are short smoke measurements, not a broad
performance or accuracy claim.

| Backend | Correct | Warm median HTTP latency |
| --- | ---: | ---: |
| Existing PyTorch MPS | 2/12 | 29.49 ms |
| MLX FP16 (new default) | 3/12 | 9.27 ms |
| MLX FP32 | 3/12 | 9.71 ms |

MLX FP16 and FP32 selected the same answer on all 12 cases; their maximum absolute
probability difference was 0.0024. Compared to the old PyTorch backend, both MLX
precisions changed two selections: `remove-en` changed from `despawn_entity` to
`defer`, and `schema-zh` from `defer` to `define_component`. The largest observed
FP16 probability difference versus PyTorch was 0.6011, so the backends must not be
described as numerically equivalent on these ECS fixtures. This one additional
correct answer does not demonstrate a general accuracy improvement. Both remain
advisory and untrained for ECS.

Raw reports: `outputs/laya/baseline-torch-before-mlx.json`,
`outputs/laya/baseline-mlx.json`, and `outputs/laya/baseline-mlx-fp32.json`.
Loading, token-budget validation and actual MLX prediction were additionally checked
in a fresh Python process without importing `torch` or `transformers`.
