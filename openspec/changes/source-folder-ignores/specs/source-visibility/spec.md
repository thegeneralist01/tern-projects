# Source visibility

## ADDED Requirements

### Requirement: Durable independent folder ignores
Projects SHALL persist reversible folder ignore rules in the durable library, independently of project folders, retained membership, assignment, pin and archive state. Adding or removing a rule SHALL NOT modify original source repositories or transcripts.

#### Scenario: Existing library upgrade and rebuild
- GIVEN a supported existing library containing projects, annotations and retained memberships
- WHEN the new backend opens it, adds an ignore, and the disposable index is rebuilt
- THEN the library upgrades transactionally, existing state is preserved, and the rule remains effective.

### Requirement: Consistent pre-pagination visibility
A matching ignore SHALL exclude sessions from Discovered, project chat projections, recents, pins, archived browsing, and all search scopes before counts and pagination. Matching SHALL consider cwd, repository root and worktree root. Curated definitions SHALL survive filtering; all-ignored cards without visible chats SHALL be absent from normal project lists.

#### Scenario: Existing and future source chats
- GIVEN an ignored source folder with existing chats and a similarly prefixed sibling folder
- WHEN additional descendant chats are indexed
- THEN existing and new matching chats are hidden, sibling chats remain visible, and totals and pages contain only visible chats.

#### Scenario: Explicit and retained memberships
- GIVEN ignored chats with explicit project/none assignments, retained memberships, archive state and pins
- WHEN the ignore is removed
- THEN normal visibility and membership are computed from the unchanged underlying states, including continued archival hiding.

### Requirement: Path identity and reversible overlap
Rules SHALL use normalized component-boundary matching, resolve available filesystem aliases, and support missing historical paths. Each listed rule SHALL retain a stable exact removal identity. Removing one rule SHALL NOT remove overlapping rules.

#### Scenario: Nested independent rules
- GIVEN parent and child folder rules
- WHEN the child rule is removed
- THEN the parent still hides the child; removing the parent subsequently restores ordinary visibility.

### Requirement: Discoverable management
Discovered source folders SHALL offer an Ignore source folder action with explicit folder/subfolder scope and reversal instructions. Settings SHALL list ignored paths and permit individual unignore, including absent paths, with keyboard-operable loading, empty and error states.
