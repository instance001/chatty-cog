# Provenance: chatty-cog

## Project identity

- Canonical repository: <https://github.com/instance001/chatty-cog>
- Public originating identity: `instance001` (repository owner recorded by GitHub API; copyright ownership is not inferred).
- Status: `NEEDS_HUMAN_VERIFICATION`
- Verified repository creation record: 2026-04-14T23:48:13Z. This is not asserted to be an original publication date.

## Intellectual contribution

GitHub repository description (repository-supplied context): The everyday use variant of the chatty ai exoskeleton series. Chatty-cog (short for cognitive prosthetic) features sandbox, local networking, peer to peer functions, standard handshake plug for module expansion allowing for functionality expansion. GPU main Ai, CPU only bookkeeper, tri-helix memory system allowing for cross module context awareness

No independent originality, priority, validation, authorship, or implementation relationship is asserted by this baseline record. A project-specific evidence review is required before making any such claim.

## Prior art and influences

Not exhaustively reviewed by this baseline. Do not treat absence of a listed influence as a claim of novelty.

## Implementation relationships

None asserted automatically. Similar names, topics, or portfolio membership are not evidence of derivation.

## Attribution and citation

Preserve the repository's existing legal notices and applicable licence when copying protected expression. Use `CITATION.cff` for a voluntary repository citation. Independent implementation of an abstract method is not automatically governed by a software licence.

## Licensing

GitHub API licence field at inventory time: `NOASSERTION`. This is a metadata observation, not a relicensing statement. See the repository's actual licence files and third-party notices.

## Evidence and limitations

Evidence: authenticated GitHub API metadata captured by the FMI provenance audit. Contribution, prior art, ownership, contributors, document licensing, relationships, releases, and publication status require human verification.

## Evidence-led lineage position

`docs/ARCHITECTURE.md`, `docs/NETWORKING.md`, `handoff-guide.md`, and the implemented `src/` surfaces support an `IMPLEMENTED` desktop host architecture. Its documented combination is: operator-selected local GGUF or BYO-cloud model lanes; separately selectable orchestrator and Bookkeeper roles; a host-owned memory/governance boundary across active context, rolling summary, append-only cold logs, semantic placements, and user-selected recall; manifest-driven module hosting and bridge/rundown records; approval-gated sandbox actions confined to `Chatty_Sandbox`; and opt-in ChattyCog-only LAN handoff/workflow-bundle exchange.

The host, rather than a model or module, normalizes capabilities and controls context attachment, memory mutation, sandbox approval, and network transfer. This is an implementation-derived architectural contribution worth preserving, but it does not establish novelty over local inference, retrieval, role separation, sandboxes, plugins, audit logs, or peer networking individually. NIST guidance on human oversight and least privilege is relevant prior art/context, not a claim of FMI derivation: <https://airc.nist.gov/docs/AI_RMF_Playbook.pdf> and <https://nvlpubs.nist.gov/nistpubs/SpecialPublications/800-171r3/NIST.SP.800-171r3.html>. `llama.cpp` is an identified local inference dependency, not FMI-originated work: <https://github.com/ggml-org/llama.cpp>.

ChattyCog does not thereby implement every mechanism described in Leviathan, Mímir, RD Engine, or other FMI specifications. Where its documentation names another repository, treat the reference as an integration, workflow, or topical link unless a project-specific source establishes a stronger relationship.
