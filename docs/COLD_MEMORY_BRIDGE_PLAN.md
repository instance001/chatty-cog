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
- Remember searches append compact recall-session metadata to `memory/remember_recalls.jsonl`, including searched/cancelled/attached outcomes without transcript payloads.
- Each Remember search gets a `recall_id` shaped like `remember-recall-{unix_ms}-{sequence}` that is reused by later cancel or attach receipts, so the audit trail can link proposal and user outcome. The id is an audit correlation key, not memory truth authority.
- Selected Remember attachments append compact audit metadata to `memory/remember_attachments.jsonl`; this records refs/provenance, links back by `recall_id`, and does not copy hydrated transcript payloads.
- Remember result feedback appends compact quality metadata to `memory/remember_feedback.jsonl` when the user marks a candidate stale, misleading, or noisy.
- Future Remember searches annotate matching bounded candidates with prior feedback counts, but do not suppress, rewrite, demote, or auto-reject the evidence.
- The Logs tab governance receipt viewer includes Remember sessions grouped by `recall_id` alongside semantic sort, placement review, and sift receipts.
- The governance receipt viewer has a non-mutating text filter for recall ids, outcomes, bucket ids, query text, and other rendered metadata.
- The Logs tab includes a recall route preview that summarizes atlas, semantic, direct cold-log lanes, and post-merge Remember candidate order for a phrase without attaching context or writing recall receipts.
- The Remember UI presents three deliberate outcomes: inspect only, attach selected to the next turn, or cancel with no injection.
- Remember candidate cards show the recall lane, semantic bucket/placement provenance where available, why the result surfaced, and a one-turn-only evidence warning.
- Remember candidate cards can record recall-quality feedback without changing cold logs, semantic buckets, atlas records, or current context.
- Cold atlas records can now be marked from the Logs tab as `needs_review`, `stale`, `retired`, or restored to `historical_shape`.
- Cold atlas status changes rewrite only status/update metadata and append compact transition receipts; they do not rewrite transcript evidence, promote context, or inject anything into Main AI.
- Cold atlas cards can hydrate each evidence ref into exact bounded cold-log context for inspection before the user revises or marks the record.
- Existing cold atlas records can be staged for revision from the Logs tab. The revision panel preloads editable JSON from that record's current evidence refs.
- Cold atlas revisions require a before/after preview for the exact current JSON, then commit a same-id in-place record replacement with a compact `revised` receipt.
- Existing cold atlas records can also be staged for supersede. The panel preloads replacement JSON, requires a new `atlas_id`, previews the old record to mark `superseded`, and commits linked receipts for the old and replacement records.
- The Logs tab renders atlas lineage rows from linked `related_atlas_id` receipts, keeping superseded broad-shape memory inspectable without making it normal recall material.
- Atlas lineage rows expose old/replacement evidence inspection buttons that hydrate exact cold-log context without recalling it into Main AI.
- The Logs tab includes an atlas query preview control for testing the atlas recall policy against a phrase without hydrating evidence, injecting context, or promoting memory.
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
- `memory/cold_semantic/placement_review_receipts.jsonl`
- `memory/cold_semantic/sift_proposals.jsonl`
- `memory/cold_semantic/sift_receipts.jsonl`

Current implementation notes:

- `src/memory/semantic_index.rs` defines the first inert semantic-index data contracts.
- `ColdEvidenceRef` points to log ranges and metadata instead of storing replacement transcript text.
- `ColdSemanticIndexPaths::under_memory_dir` resolves the planned `memory/cold_semantic/` layout.
- `read_bucket_map_json` and `write_bucket_map_json` provide explicit `bucket_map.json` persistence.
- `default_interaction_bucket_map` defines a small starter v1 map: entities, projects/code, preferences/workflow, open threads, plus misc.
- The Logs tab exposes a user-triggered "Create starter bucket map" control and a read-only bucket-map inspection view.
- Bucket-map validation requires frozen counts to match and exactly one misc bucket to exist.
- `read_placements_jsonl` and `append_placement_jsonl` provide explicit placement JSONL persistence.
- `append_validated_placement_jsonl` requires `bucket_map.json` and validates the placement before writing.
- Placement validation rejects missing bucket refs and overflow placements that do not target misc.
- `load_semantic_index_snapshot` reads bucket map and placements together, rejecting orphan placements without a map.
- `src/memory/semantic_sorter.rs` prepares bounded cold-log interaction evidence, renders a strict sorter prompt, parses pasted sorter JSON, and commits validated proposed placements with a sort receipt.
- `BookkeeperHandle::draft_semantic_sort` can draft sorter JSON from the prepared prompt using the existing Bookkeeper lane.
- `render_semantic_sort_review_preview` validates sorter JSON and renders a human-readable placement preview without writing placements.
- `render_semantic_bucket_placement_overview` renders the current bucket placement store by bucket, status, fit, source, event range, and reason without embedding transcript text.
- `render_semantic_receipts_overview` renders sort, placement review, and sift receipts as an inspectable governance trail.
- The Logs tab exposes the semantic sort lever as a deliberate staged operation: prepare prompt, optionally ask Bookkeeper to draft JSON, preview/edit the JSON, then commit.
- Commit requires a successful preview receipt for the exact current JSON; editing the JSON invalidates the receipt and requires previewing again.
- The Logs tab can refresh an inspectable bucket placement view, and successful commits refresh that view.
- Semantic sort commits create proposed placements. Proposed placements are inspectable but inert for Remember.
- The Logs tab can load proposed placements for review, let the user approve/reject selected placements, and apply those decisions deterministically.
- Placement review includes both newly proposed placements and `needs_review` placements created by feedback-driven sifting.
- Proposed placement review shows bucket labels/descriptions from the frozen bucket map and supports bucket/fit filtering.
- Proposed placement cards can hydrate exact bounded cold-log context for inspection before approval; inspection does not approve, inject, promote, or rewrite memory.
- Placement reviews rewrite placement statuses and append durable review receipts to `placement_review_receipts.jsonl`, including status transitions such as `needs_review -> approved`.
- The Logs tab can refresh governance receipts so semantic memory mutations remain visible after the fact.
- Semantic bucket search uses approved placements only.
- Semantic bucket query preview shows which approved evidence refs would surface for a phrase, with an `include_misc` toggle for the weak-fit lane.
- The memory test suite covers the full approved semantic recall loop: interaction cold log, proposed placement, user approval, semantic Remember hit, exact bounded hydration, explicit selection, and host-owned injection boundary.
- No automatic sorter, background writer, or autonomous bucket mutation path exists yet.

Important rules:

- A valid semantic bucket map has positive buckets plus exactly one misc bucket.
- Buckets store refs and metadata; exact text is hydrated from cold logs.
- The same evidence may appear in multiple buckets if the fit is real.
- Weak fits should stay visible rather than forced into a false bucket.
- Overflow placements must target the misc bucket.
- Bucket search should return candidate evidence refs, then the governor hydrates exact surrounding transcript context from the linear log.
- Bucket search should consider approved placements only; proposed placements are not recall-active.
- The Logs tab can preview bucket search policy without hydrating evidence, injecting context, or promoting memory.
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

Current implementation notes:

- `ColdBucketSiftProposal` and `ColdBucketSiftChange` model proposed bucket cleanup without touching cold logs.
- `build_bucket_sift_proposal` detects the first deterministic cleanup cases: duplicate active placements for the same evidence ref and weak positive-bucket fits that should move to misc.
- Bucket sifting can read Remember feedback receipts and propose `needs_review` changes for placements whose bounded evidence has prior noise or misleading marks.
- New sift changes are deselected by default. The user must explicitly check each change before applying it.
- The Logs tab exposes a "Bucket sifting" review panel with bucket selection, proposal creation, per-change checkboxes, exact cold-log context inspection, and an "Apply selected sift" action.
- Exact context inspection hydrates the referenced cold-log exchange through the governor. Inspection does not approve, inject, promote, or rewrite memory.
- `apply_selected_sift_changes_to_store` rewrites only the placement index, never the source cold log, and appends a durable receipt to `sift_receipts.jsonl`.
- Selected feedback-driven sift changes mark the placement `needs_review` rather than silently rejecting or deleting the evidence.
- Sift proposals are also appended to `sift_proposals.jsonl`, creating a visible trail of proposed cleanup even when the user applies nothing.

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

Current implementation notes:

- `src/memory/cold_atlas.rs` defines the first cold-atlas record, confidence, status, path, and receipt contracts.
- The v1 atlas store uses `memory/cold_atlas/atlas_records.jsonl` and `memory/cold_atlas/atlas_receipts.jsonl` for inspectable JSONL persistence.
- Atlas records require at least one `ColdEvidenceRef`, so broad shape memory must point down to exact cold-log evidence rather than floating free.
- Atlas receipts are compact governance metadata and do not duplicate transcript payloads.
- The Logs tab exposes a read-only "Refresh atlas" inspection control for broad shape records.
- The governance receipt viewer includes cold-atlas receipts alongside semantic and Remember receipts.
- The Logs tab exposes a staged cold-atlas draft flow: prepare from approved semantic placements, paste/edit atlas JSON, preview the exact current JSON, then commit.
- Atlas draft prompts carry approved semantic placement refs and reasons, not hydrated transcript payloads.
- The staged draft flow can ask Bookkeeper to draft atlas JSON from the prepared prompt; the draft remains editable and inert until previewed and committed by the host.
- Commit requires a successful preview for the exact current JSON, then writes atlas records plus compact receipts.
- Remember can query cold-atlas records as a broad-orientation lane, then hydrate each matching evidence ref through the governor into exact bounded cold-log transcript candidates.
- Normal Remember atlas lookup excludes `retired` records and default-excludes `superseded` records so replaced shapes do not compete with their replacements.
- Atlas recall candidates are labeled as cold-atlas sourced in the Remember UI and show atlas title, id, summary, scope, status, and confidence as orientation metadata.
- Atlas summary text explains why a candidate surfaced; it is kept out of the attachable exact transcript payload unless the same text exists in the source cold-log context.
- Atlas recall candidates remain user-selected, one-turn-only evidence.
- Atlas evidence inspection uses the same governor hydration path as Remember, semantic review, and bucket sifting. Inspection is read-only and cannot revise, inject, promote, or rewrite memory by itself.
- Existing atlas records can be revised through a staged before/after flow that uses only that record's current evidence refs and requires the same `atlas_id`.
- Revision commits replace the broad atlas record in place, preserve the original creation timestamp, reset status to `historical_shape`, and append a compact `revised` receipt without transcript payloads.
- Supersede commits append one replacement atlas record, mark the old record `superseded`, and include `related_atlas_id` links in both compact receipts.
- Superseded records are preserved for lineage and governance inspection, but are not normal atlas recall candidates unless a caller explicitly opts into superseded lookup.
- Atlas lineage is rendered from receipts rather than a separate relationship store, and interactive rows reuse the same read-only evidence inspection path as atlas cards.
- Atlas query preview shows broad-shape matches and evidence refs only; it is a diagnostic view, not a Remember attachment path.
- No automatic atlas summarizer, background writer, promotion path, or direct atlas-to-Main-AI injection path exists yet.

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
