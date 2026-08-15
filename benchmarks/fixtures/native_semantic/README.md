# Native semantic fixture

This fixed corpus tests repository-learned vocabulary without a model or
network service. Each query term appears beside three training identifiers,
which is the lexicon support floor. The judged target uses the learned
identifier term but deliberately omits the query prose.

The background symbols provide the contrast required for positive PMI. Gold
labels live in `benchmarks/gold/native_semantic.json`.
