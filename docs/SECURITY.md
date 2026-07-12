# Security checks

CI runs `cargo audit --deny warnings`. Exceptions must name the transitive owner,
state why the code is not directly replaceable here, and carry a review date.

| Advisory | Dependency path | Decision | Review by |
|---|---|---|---|
| `RUSTSEC-2026-0173` | `age` → `i18n-embed-fl` → `proc-macro-error2` | Unmaintained build-time procedural macro; no reported vulnerability. `age` owns the dependency. Recheck for an `age` release that removes it. | 2026-10-12 |
