# qdev Quality, Verification Gates & IEC 62304 Compliance

> SaMD verification architecture, ISO 14971 risk tracking, code hygiene filters, preflight Git synchronizations, and zero-noise gate execution.

## Table of Contents

- [Multi-Entity Citations & Code Hygiene Filter](#code-hygiene)
- [The Generic Ratchet & Gate Engine](#gate-engine)
- [IEC 62304 Medical Device Architecture & Compliance](#iec-compliance)
- [Git Integration & Preflight Synchronization](#git-preflight)

---

<a id="code-hygiene"></a>

## 7. Multi-Entity Citations & Code Hygiene Filter

A pervasive pathology when pair-programming with advanced LLMs (such as Opus 5) is that models frequently inject **forensic development memoirs** directly into source code comments.

### The Anti-Pattern: Forensic Memoir Pollution

```rust
// ❌ ANTI-PATTERN: FORENSIC ESSAY COMMENT IN PRODUCTION CODE
// ⭐ **STORY 2.10 — THE FIRST `import RustBridge` IN THE RUNNING APP.**
//
// `DiscoveryPaneView` (the TCA pane) is replaced here by the Rust-driven
// one. The reducer itself is untouched: `DatabaseFeature`'s
// `Scope(state: \.discoveryPane, …)` still runs, `DiscoveryPaneFeatureTests`
// still passes, and the exit below goes back through the reducer's own
// `Delegate` arms — so the navigation an operator gets is byte for byte
// the navigation the reducer already produced. `AW-NFR-16` requires the
// evidence at a commit PRECEDING the deletion, which is 2.11's.
RustDiscoveryPaneMount(...)
```

### The Solution: Multi-Entity Compact Citations

`qdev` standardizes concise inline citations referencing any entity in the system. The `/qdev-develop` prompt explicitly instructs the LLM:

> [!NOTE]
> **Prompt Directive for Developer Agent**
>
> *"CRITICAL: Write clean, standard code comments. Reference architectural decisions, stories, and rulings using compact bracket tags: `// [DEC-XXX] ...`, `// [S...E...S...] ...`, `// [AD-XX] ...`. NEVER generate narrative historical memoirs, story summaries, or review round commentary in code."*

| Entity Type | Compact Inline Citation Example | Where Full Context Lives |
| --- | --- | --- |
| **Story Reference** | `// [S5E2S10] Replaced TCA pane with Rust-driven mount (ref AD-43)` | Story spec: `docs/specs/sprint-5/S5E2S10.md` |
| **Decision Reference** | `// [DEC-293] Show the extended attributes table here per Simon's ruling` | SQLite `decisions` table / Story scratchpad |
| **ADR Reference** | `// [AD-43] Worker pool bounded join; no async sleep permitted` | Architecture Spine / ADR registry |
| **Hazard Control** | `// [HAZ-014] Safety PIP engagement threshold latch` | `HAZARD-MATRIX.md` / Risk register |
| **Deferred Work** | `// [DW-421] Temporary zero-copy buffer bypass` | SQLite `deferred_work` table |

### 1. Distillation to Compact Citations

Historical narrative is stripped from source files. Code comments are compressed into clean docstrings with compact story/decision tags.

### 2. Relocation to Story Scratchpad

The forensic reasoning (review rounds, deleted banners, trade-offs) is automatically relocated into the story's **scratchpad** or **decision ledger**.

---

<a id="gate-engine"></a>

## 14. The Generic Ratchet & Gate Engine

### On Pass (Token Savings: ~95%)

Instead of dumping 400 lines of Cargo build output:

```text
[PASS] gate:c-abi-round-trip | 18 tests passed | sha: 8f1b2c | duration: 1.2s
```

### On Failure (Noise Stripping)

Strips compiler warnings and extracts only the failing assertion:

```text
[FAIL] gate:c-abi-round-trip (exit 101)
crates/bridge/tests/c_abi_round_trip.rs:142
assertion failed: `(left == right)`
  left: `CoreResponse::EffectsUnavailable`,
 right: `CoreResponse::EventNotUnderstood`
```

---

<a id="iec-compliance"></a>

## 15. IEC 62304 Medical Device Architecture & Compliance

| Standard Requirement | Clause | Automated Mechanism in `qdev` |
| --- | --- | --- |
| **SOUP & Vulnerability Audits** | §5.3.3, §5.3.4 | `qdev soup audit` runs dependency checks; exports release SBOM. |
| **Known Residual Anomalies** | §5.8.7, Cl. 9 | ISO 14971 risk tagging on `DW-*`; auto-generated Anomaly Report on release baseline. |
| **Architectural Segregation** | §5.3.5 | Automated gate enforcing downward-only ring dependency between modules. |
| **Verification Independence** | §5.5.3, §5.7.3 | Strict phase separation (`develop` vs `review`); logged reviewer evidence receipts. |
| **Change Impact Analysis** | §5.7.4, Cl. 6.2.3 | Graph traversal determining affected stories, modules, and mandatory regression tests. |

---

<a id="git-preflight"></a>

## 16. Git Integration & Preflight Synchronization

> [!CAUTION]
> **The Git Preflight Guard**
>
> Before allowing `/qdev-develop` or `/qdev-create-story` to begin, `qdev` checks Git status against the configured remote:
>
> - If uncommitted changes exist outside the active story scope: **Refuses execution**.
> - If the local working branch is behind `origin/main`: **Refuses execution** and prints: <br>`⚠️ Error: Local branch is behind origin/develop by 3 commits. Run 'git pull' before starting S5E2S4 to prevent merge collisions.`

---

