---
name: discover-shennong-data
description: Discover and assess governed Shennong DB Resources for a biomedical question, including schema, identifiers, versions, normalization, cohort context, provenance, and bounded queries. Use before selecting public data or making stored-data claims.
---

# Discover Shennong data

1. Call `db.discover_resources` with broad disease, cohort, assay, or modality terms.
2. Call `db.inspect_resource` for each plausible Resource before querying it.
3. Confirm organism, assay, build, annotation release, normalization, identifiers, cohort axes, declared operations, version, and license.
4. Use `db.query_resource` only with a declared operation, exact context labels, and the smallest useful row limit.
5. Call `db.get_provenance` for every Resource used in a result.
6. Cite only EvidenceRef identifiers returned by governed tools and state truncation, missing annotations, and permission boundaries.

Do not treat an empty catalog search as proof that the biological feature is absent.
