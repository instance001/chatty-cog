# Cold Memory Bridge Plan

Status: planning document  
Editable repo: `chatty-cog` only  
Reference repos: read-only material under `C:\Users\User\Desktop\github_portal`

## Purpose

Chatty-Cog's long-memory system should behave less like a generic archive search box and more like an operator-controlled context stack:

```text
active context / Hot Memory
  active exact context for the current turn

rolling summary / Luke Warm
  current task/thread orientation

cold atlas
  broad remembered shape of older sessions

cold evidence log / audit history
  exact historical user/assistant records
```

The first bridge should stay narrow: user-triggered recall over interaction logs. Cold logs remain historical evidence. Retrieved material must not silently become current truth, active context, or rolling summary.

## Core Authority Rule

Search may propose.  
The user chooses.  
The host injects only selected exact evidence.

No Bookkeeper, Main AI, summarizer, or future memory support role should be able to bypass that boundary for user-visible conversation context.

## Reference Landscape

### `project-leviathan`

Reference files:

- `C:\Users\User\Desktop\github_portal\project-leviathan\DUAL_COLD_MEMORY_AND_DEEP_RECALL.md`
- `C:\Users\User\Desktop\github_portal\project-leviathan\COGNITIVE_ECONOMY_GOVERNOR.md`
- `C:\Users\User\Desktop\github_portal\project-leviathan\SEMANTIC_ASSIMILATION_AND_WHY_LIBRARY.md`

What to mirror:

- The split between `cold atlas` and `cold evidence log`.
- The rule that atlas summaries locate, while exact evidence substantiates.
- Deep recall as a bounded descent from a current question into historical specifics.
- Honest recall states: exact recovered, partial, contradiction, no supporting record, budget exhausted.
- Reasoning budget: do not descend into cold evidence unless exactness matters or the operator asks.

What not to copy yet:

- Full relation atoms, worldview branching, Why Library promotion, or multi-level semantic-object machinery. Those are later layers.

### `llm-defined-persistent-memory`

Reference files:

- `C:\Users\User\Desktop\github_portal\llm-defined-persistent-memory\llm-defined-memory.md`
- `C:\Users\User\Desktop\github_portal\llm-defined-persistent-memory\deterministic-hot-context-injection.md`
- `C:\Users\User\Desktop\github_portal\llm-defined-persistent-memory\context-routing-packets.md`
- `C:\Users\User\Desktop\github_portal\llm-defined-persistent-memory\design-stance.md`

What to mirror:

- The host owns persistence.
- The model may propose memory actions but does not mutate memory directly.
- Cold logs are audit history, not the first active memory surface.
- Hot context cards and routing packets should be deterministic, inspectable, and source-aware.
- Context routing packets can become compact orientation cards with aliases, current status, adjacency, confidence, and evidence refs.

What not to copy yet:

- Automatic trigger-based hot context injection.
- Model-defined bucket maps as a prerequisite for Remember.
- Automatic memory updates after a retrieval hit.

### `rd-engine`

Reference files:

- `C:\Users\User\Desktop\github_portal\rd-engine\src\engine.rs`
- `C:\Users\User\Desktop\github_portal\rd-engine\src\save.rs`
- `C:\Users\User\Desktop\github_portal\rd-engine\examples\project_memory_host.rs`

What to mirror:

- Reducer-governed durable mutation.
- Action receipts and rejected actions.
- A small generic state/update pattern that can be renamed into Chatty-Cog's memory domain.

Best later use:

- If selected cold evidence is promoted into durable memory, that promotion should go through a reducer-like host boundary with a receipt.
- The Remember v1 attachment path should remain read-only injection, not reducer mutation.

### `4roomciv`

Reference files:

- `C:\Users\User\Desktop\github_portal\4roomciv\README.md`
- `C:\Users\User\Desktop\github_portal\4roomciv\4room-civ-mvp.zip\sqlite_schema.sql`
- `C:\Users\User\Desktop\github_portal\4roomciv\4room-civ-mvp.zip\schemas\helix_spine.schema.json`

What to mirror:

- SQLite + FTS5 as a possible cold-atlas/search implementation.
- Validated `spine` records containing timestamp, topic, claim, rationale, evidence refs, confidence, and short notes.
- Topic side tables for exact topic filtering.
- A visible `SPINE_AUTOWRITE=0` style lever as a design reminder: auto-writing must be explicitly controllable.

What not to copy yet:

- Automatic spine extraction and auto-write into memory.
- RAG directly into model context without user selection.

### `MemorySpine`

Reference files:

- `C:\Users\User\Desktop\github_portal\MemorySpine\memoryspine.py`
- `C:\Users\User\Desktop\github_portal\MemorySpine\README.md`

What to mirror:

- Preserve conversation shape: author, timestamp, message order, conversation identity.
- Prefer boring, inspectable conversion over clever opaque processing.
- Keep raw fallback material when parsing cannot confidently normalize a conversation.

Best use:

- Cold-log import/export and exact transcript display.
- Provenance display in Remember result expansion.

### `semantic-signal-alphabet`

Reference files:

- `C:\Users\User\Desktop\github_portal\semantic-signal-alphabet\README.md`
- `C:\Users\User\Desktop\github_portal\semantic-signal-alphabet\schemas\bucket_map.schema.json`
- `C:\Users\User\Desktop\github_portal\semantic-signal-alphabet\docs\03_bucket_generation_process.md`
- `C:\Users\User\Desktop\github_portal\semantic-signal-alphabet\docs\04_unassigned_and_wildcard.md`

What to mirror later:

- Bucket count is bandwidth.
- Bucket maps are frozen, versioned, and model-specific.
- `UNASSIGNED` is an overflow/review surface, not trash.

What not to copy yet:

- Do not require semantic bucket maps for the first cold-log bridge.
- Do not create a universal memory taxonomy too early.

### `llm-semantic-dataset-sorter`

Reference files:

- `C:\Users\User\Desktop\github_portal\llm-semantic-dataset-sorter\README.md`
- `C:\Users\User\Desktop\github_portal\llm-semantic-dataset-sorter\docs\00-product-plan.md`
- `C:\Users\User\Desktop\github_portal\llm-semantic-dataset-sorter\docs\02-schema-plan.md`
- `C:\Users\User\Desktop\github_portal\llm-semantic-dataset-sorter\docs\05-user-manual.md`

What to mirror:

- Preflight before expensive semantic work.
- Frozen plans before assignment.
- Mandatory junk/overflow bucket.
- Human-facing rationale plus machine-facing artifacts.
- Snapshot/compare review patterns.

Best later use:

- Cold atlas generation and review.
- Summarizer runs that produce inspectable run artifacts instead of quiet summary drift.

### `ef-engine`

Reference files:

- `C:\Users\User\Desktop\github_portal\ef-engine\README.md`
- `C:\Users\User\Desktop\github_portal\ef-engine\src\engine.rs`

What to mirror later:

- Failed recall should be evidence too.
- Repeated stale/incorrect memory retrievals can become constraints.
- Memory failures should be vaulted, compared, and turned into routing rules only when the pattern is real.

Best later use:

- "This memory was wrong/stale/misleading" receipts.
- Constraints that prevent recurring bad recall shapes.

## Chatty-Cog Integration Points

Current local files:

- `src/memory/bookkeeper.rs`
  - writes `memory/cold_log.jsonl`
  - performs existing semantic search where embeddings are available
  - maintains the Luke Warm rolling summary
- `memory/cold_log.jsonl`
  - current exact evidence source
  - JSONL envelope is schemaless but has common fields
- `src/main.rs`
  - owns generation prompt assembly
  - natural place for host-owned selected context injection
- `src/chat_ui.rs`
  - natural place for a deliberate Remember control near conversation/tool controls
- `docs/ARCHITECTURE.md`
  - should be updated as bridge pieces stabilize

## Candidate Object Model

Initial v1 concepts:

```text
ColdMemoryQuery
  current user input
  maximum candidates
  surrounding exchange count
  optional scope filters

ColdMemorySourceRef
  log/session/source id
  event index or source offset
  timestamp where available
  source path

ColdMemoryCandidate
  candidate id
  score or match reason
  provenance
  matched exchange
  bounded context block
  preview text

ColdMemoryAttachment
  selected candidate ids
  attached timestamp
  exact host-rendered payload
```

Later semantic-index and atlas concepts:

```text
ColdSemanticBucket
  bucket id
  label/description
  sort intent
  status/currentness
  linked cold evidence refs
  weak-fit or overflow refs

ColdEvidenceRef
  source log path/session id
  exchange id or event range
  timestamps
  role/source metadata
  source hash where available

ColdBucketPlacement
  evidence ref
  bucket id
  fit strength
  reason/rationale
  created by sorter/model/host
  review status

ColdBucketSiftProposal
  bucket id
  proposed moves/merges/stale marks/removals
  before state
  after state
  rationale
  selected/approved changes

ColdAtlasRecord
  title
  summary
  topics/entities/projects
  confidence/status
  evidence refs
  revision metadata

DeepRecallRequest
  current question
  missing detail
  candidate atlas refs
  maximum budget
  stop conditions

DeepRecallResult
  hydrated evidence refs
  exact details
  unresolved gaps
  contradiction notes
  receipt
```

## First Vertical Slice

Goal: let the user deliberately ask Chatty-Cog to remember from interaction logs and decide what, if anything, crosses into the next Main AI turn.

Steps:

1. Add a reusable governor/retrieval module under `src/memory/`.
2. Read existing `memory/cold_log.jsonl` without rewriting it.
3. Focus v1 on normal user/assistant chat events, not sandbox/tool/module events.
4. Search from the current user input automatically; do not require a separate manual query.
5. Produce candidates that include:
   - one previous complete user/assistant exchange where available
   - the matched exchange
   - one following complete user/assistant exchange where available
6. Preserve provenance:
   - source log path
   - event indexes
   - timestamp/date if present
   - matched event/exchange identity
7. Show a Remember result tray/window in the Chat UI:
   - compact cards or bullets
   - select checkboxes
   - expandable exact payload
   - attach selected
   - cancel/no injection
8. Inject selected material into the next Main AI prompt only under:

```text
USER-SELECTED COLD LOG CONTEXT
```

9. Mark injected material as historical evidence, not automatically current truth.
10. Clear the attachment after the next generation prompt is built.
11. Add tests for:
   - bounded surrounding exchange extraction
   - no automatic active-context / rolling-summary promotion
   - no injection without selection
   - exact payload rendering with provenance boundary

Current implementation notes:

- `src/memory/governor.rs` contains the first host-owned retrieval and attachment receipt layer.
- Remember v1 reads `memory/cold_log.jsonl` and only considers normal user/assistant interaction events.
- Candidate context is the matched exchange plus one exchange before and after where available.
- Candidate payloads include source path, event indexes, UTC timestamps, raw Unix milliseconds, and exact bounded transcript text.
- `ColdMemoryAttachment::from_selected_ids` returns no receipt when no candidate IDs are selected.
- Main prompt injection is next-send-only and appears under `USER-SELECTED COLD LOG CONTEXT`.
- The Remember UI presents three deliberate outcomes: inspect only, attach selected to the next turn, or cancel with no injection.
- Current tests cover bounded extraction, ignoring non-interaction events, explicit selection receipts, timestamp formatting, and historical-evidence boundary language.

## Non-Goals For V1

- No vector database.
- No new persistence system.
- No automatic cold-log promotion into active context.
- No automatic rolling-summary rewrite from Remember.
- No semantic bucket map requirement.
- No automatic trigger-based hot context.
- No module/sandbox/tool event retrieval unless explicitly added later.
- No summarizer authority to inject directly into Main AI conversation.

## Phase 2: Cold Semantic Index

Goal: add a meaning-sorted retrieval lane beside the linear timestamped cold log.

The cold log stays the exact chronological truth trail. The semantic index is a navigational structure over that evidence, not a replacement for it.

Desired flow:

```text
user/assistant exchange enters cold_log.jsonl
        ↓
host assigns or derives stable evidence refs
        ↓
semantic sorter proposes bucket placements
        ↓
bucket index stores evidence refs plus lightweight metadata
        ↓
Remember searches semantic buckets first
        ↓
matched evidence refs hydrate surrounding context from cold_log.jsonl
        ↓
user selects exact transcript blocks
        ↓
host injects selected bounded evidence
```

What to mirror from `llm-semantic-dataset-sorter`:

- fixed bucket plans for a run
- preflight before expensive sorting
- a visible misc/overflow/weak-fit lane
- human-facing rationale for bucket shape and placements
- machine-facing JSONL artifacts
- reviewable snapshots/comparisons

What to mirror from `semantic-signal-alphabet`:

- bucket count as bandwidth
- frozen/versioned bucket maps
- model-specific bucket maps when the runtime model matters
- `UNASSIGNED` as reviewable residue, not discarded junk

Possible storage:

- `memory/cold_semantic/bucket_map.json`
- `memory/cold_semantic/placements.jsonl`
- `memory/cold_semantic/sift_proposals.jsonl`
- `memory/cold_semantic/sift_receipts.jsonl`

Current implementation notes:

- `src/memory/semantic_index.rs` defines the first inert semantic-index data contracts.
- `ColdEvidenceRef` points to log ranges and metadata instead of storing replacement transcript text.
- `ColdSemanticIndexPaths::under_memory_dir` resolves the planned `memory/cold_semantic/` layout.
- `read_bucket_map_json` and `write_bucket_map_json` provide explicit `bucket_map.json` persistence.
- Bucket-map validation requires frozen counts to match and exactly one misc bucket to exist.
- `read_placements_jsonl` and `append_placement_jsonl` provide explicit placement JSONL persistence.
- `append_validated_placement_jsonl` requires `bucket_map.json` and validates the placement before writing.
- Placement validation rejects missing bucket refs and overflow placements that do not target misc.
- `load_semantic_index_snapshot` reads bucket map and placements together, rejecting orphan placements without a map.
- No automatic sorter, background writer, or bucket mutation path exists yet.

Important rules:

- A valid semantic bucket map has positive buckets plus exactly one misc bucket.
- Buckets store refs and metadata; exact text is hydrated from cold logs.
- The same evidence may appear in multiple buckets if the fit is real.
- Weak fits should stay visible rather than forced into a false bucket.
- Overflow placements must target the misc bucket.
- Bucket search should return candidate evidence refs, then the governor hydrates exact surrounding transcript context from the linear log.
- Bucket placement relevance is not present-day truth.
- The user still decides what enters Main AI context.

Example:

```text
current input:
  "What was the thing we worked out about Miso's vet anxiety?"

semantic bucket search:
  entity:miso
  topic:pet_health
  topic:anxiety_patterns

bucket hit:
  evidence ref -> cold_log event range 420..427

hydration:
  previous exchange
  matched exchange
  following exchange

Remember UI:
  candidate shown with exact payload
  user attaches or cancels
```

## Phase 3: Bucket Sifting

Goal: keep semantic buckets slim, useful, and temporally relevant without letting the system silently rewrite memory.

Sifting should propose changes only. The user approves selected changes before the host mutates the bucket index.

Possible sift actions:

- merge duplicate refs
- mark stale or superseded placements
- move weak-fit refs to a better bucket
- move noisy refs to overflow/review
- split an overloaded bucket
- retire a low-value bucket
- preserve temporally important old refs even if no longer current

User-gated review shape:

```text
sifter proposes before/after bucket changes
        ↓
Remember/Memory UI shows:
  - proposed action
  - affected refs
  - before bucket
  - after bucket
  - rationale
  - checkbox per change
        ↓
user approves selected changes or cancels
        ↓
host applies approved reducer-style mutations
        ↓
receipt records what changed and why
```

Sifting must not:

- delete cold log evidence
- remove provenance
- silently promote a summary to truth
- hide stale history as though it never existed
- collapse temporally distinct events into one unsupported memory
- auto-approve model cleanup proposals

## Phase 4: Cold Atlas

Goal: add a compact broad-memory layer above cold evidence.

Possible storage:

- JSONL first for inspectability, or
- SQLite + FTS5 if search/query pressure justifies it.

Atlas records should:

- summarize a bounded scope
- point down to exact cold-log evidence
- carry status/currentness/confidence
- preserve revision history
- never replace the underlying evidence

The cold-log summarizer should produce candidates or drafts first. A host/user review path should decide what becomes atlas memory.

## Phase 5: Routing Packets

Goal: reduce repeated archive skimming for known projects/entities/concepts.

Packets should include:

- canonical name
- aliases
- current role/status
- short hot-context line
- adjacency hints
- evidence refs
- stale/current status

They should be triggered deterministically later, but early versions can be user-invoked or manually inspected.

## Phase 6: Reducer-Governed Promotion

Goal: selected evidence or reviewed summaries can become durable memory only through an explicit host-owned mutation boundary.

Possible reducers:

- `remember`
- `append_evidence`
- `mark_stale`
- `request_confirmation`
- `clear_memory`
- `create_packet`
- `add_adjacency`
- `revise_atlas_record`

Each mutation should emit a receipt. Rejected mutations should be visible.

## Phase 7: Model-Native Standing Buckets

Goal: add model-specific standing memory buckets only after the evidence, atlas, and promotion boundaries are stable.

Rules:

- new model may need a new bucket map
- bucket maps are frozen artifacts
- historical logs adapt runtime memory after bucket creation, not before
- `UNASSIGNED` remains visible and reviewable

## Open Design Questions

- Should Remember candidates be attached only to the next turn, or optionally pinned for the current mini-thread?
- Should selected cold evidence create a small attachment receipt in a separate log?
- Should semantic bucket placement happen synchronously as events are logged, or in a background batch pass?
- Should v1 semantic buckets be user-authored, model-authored, or generated from a fixed default map?
- Should an evidence ref be allowed in multiple buckets by default, or only after review?
- What minimum evidence identity do we need before bucket indexes become reliable: event indexes, exchange IDs, source hashes, or all three?
- How should bucket sifting show before/after state without overwhelming the user?
- Should the first atlas store be JSONL for simplicity or SQLite FTS5 for immediate search quality?
- Should Bookkeeper be allowed to rank Remember candidates, or should v1 stay deterministic keyword-first with Bookkeeper as optional search assist?
- What is the exact UI boundary between "read this result to refresh myself" and "attach this to Main AI"?
- How should stale or contradicted cold results be marked once atlas records exist?

## Working Doctrine

Active context keeps selected details close.  
Rolling summary orients the current thread.  
Cold semantic index remembers the shape.  
Audit history remembers the specifics.  
Remember is deliberate recall, not automatic belief.
