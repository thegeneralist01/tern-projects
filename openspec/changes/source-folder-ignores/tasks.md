# Delivery checklist

- [x] Inspect durable library, session projections, path normalization and frontend controls; define the reversible ignore contract.
- [x] Implement schema migration and ignore/list/unignore requests without changing transcripts or chat/project state.
- [x] Apply normalized component-boundary filtering before all list/search counts and pagination.
- [x] Implement Discovered source-folder confirmation and Settings inspection/unignore controls.
- [x] Verify synthetic migration, aliases/missing paths, overlap, existing/future chats, assignments/archive preservation, rebuild and restoration through the real CLI.
- [x] Verify the production panel's ignore/unignore journey in isolated real Tern with the managed CLI; run integrated Rust and Luau gates.
- [x] Resolve one independent review and document evidence and residual limitations.
- [x] Commit and push the feature branch to the fork; publish upstream PR without merging or changing the installed plugin: https://github.com/thegeneralist01/tern-projects/pull/3.
