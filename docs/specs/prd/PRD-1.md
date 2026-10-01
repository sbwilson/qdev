---
id: PRD-1
title: "qdev PRD"
status: final
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
owners:
  - "simon"
  - "team:core"
vision: "To eliminate markdown context sprawl and AI hallucination in complex software development by replacing flat planning files with a relational, SQLite-indexed workflow engine over Git-native Markdown."
personas:
  - "Lead developer / architect"
  - "AI coding agent"
  - "Compliance / release manager"
  - "CI pipeline"
metrics:
  - "Median tokens in develop context payload <= 1200 (actual: 608)"
  - "Median tokens in review context payload <= 2500 (actual: 2495)"
  - "Cache hydration on boot, 1,000 entities, one changed <= 30 ms"
  - "Citation rot after 10 sprints = 0"
  - "Gate pass receipt size <= 2 lines"
  - "Commands that can block on stdin in non-interactive mode = 0"
---

# qdev PRD

## 1. Vision & Purpose
To eliminate markdown context sprawl and AI hallucination in complex software development by replacing flat planning files with a relational, SQLite-indexed workflow engine over Git-native Markdown. `qdev` synthesises BMAD's hierarchical decomposition with Shape Up's negative constraints (appetites, rabbit holes, no-gos), keeping AI coding agents and human developers aligned, gated, and traceable for high-integrity software such as medical devices.

`qdev` is not itself SaMD. It is a development tool whose outputs are designed to support, not guarantee, IEC 62304 and ISO 14971 processes.

## 2. Users & Personas
| Persona | Needs |
| --- | --- |
| **Lead developer / architect** | Plan epics and stories, define constraints and ADRs, review agent output, close sprints |
| **AI coding agent** (Claude Code, Cursor, Gemini) | Receive a small, exact context payload; know the boundaries; report progress; be told precisely why something was rejected |
| **Compliance / release manager** | Traceability from requirement to story to gate evidence; residual anomaly list; SOUP inventory |
| **CI pipeline** | Deterministic JSON, stable exit codes, no prompts |

## 3. Success Metrics
| Metric | Target | Actual |
| --- | --- | --- |
| Median tokens in a `develop` context payload | ≤ 1,200 | 608 (E4S10 measured) |
| Median tokens in a `review` context payload | ≤ 2,500 | 2,495 (E4S10 measured) |
| Cache hydration on boot, 1,000 entities, one changed | ≤ 30 ms | |
| Citation rot after 10 sprints | 0 renamed IDs | |
| Gate pass receipt size | ≤ 2 lines | |
| Commands that can block on stdin in non-interactive mode | 0 | |
