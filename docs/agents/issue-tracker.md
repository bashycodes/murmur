# Issue tracker: TickTick

Issues and specs for this repo live as tasks in the TickTick list **Murmur**
(project id `6ade2cbf8f081c311be5b39d`, in the owner's "Projects" folder).
Agents reach it through the TickTick MCP tools (`mcp__claude_ai_TickTick__*`).
Code, branches and pull requests stay on GitHub (`bashycodes/murmur`);
TickTick holds only the work items.

## Mapping

| Skill concept | In TickTick |
|---|---|
| Issue / ticket | A task in Murmur (`kind` TEXT). Title = issue title; `content` = the body (markdown). |
| Issue reference (`#42`) | The task id. Agents print `title (id)` when they name one. |
| Label / triage role | A tag of the same name (see `triage-labels.md`). |
| Comment | `add_comment` (plain text, **max 1024 chars**). Longer text goes into the task `content` under a heading, and the comment says where. |
| Spec | A parent task tagged `spec`; its `content` is the spec. |
| Tickets of a spec | Subtasks of the spec task (`parentId` = spec task id). Title prefix `NN ` (`01 `, `02 `, ...) gives the order. |
| Blocking edges | A `Blocked by: <task id>, <task id>` line at the top of the ticket's `content`. TickTick has no native links. |
| Claimed by an implementer | Swap the `ready-for-agent` tag for `in-progress` with `update_task`, passing the task's current `etag` so a concurrent change makes the claim fail instead of double-claiming. Only the orchestrator claims; at most 3 tasks are `in-progress` at once. |
| Close as done | `complete_task`, after a comment linking the commit or PR. |
| Close as wontfix | `update_task` with `status: -1` (abandoned), after the explaining comment. |

## When a skill says "publish to the issue tracker"

`create_task` with `projectId` `6ade2cbf8f081c311be5b39d`, the title, the body in
`content`, and the right tags. For tickets of a spec, also set `parentId`.

## When a skill says "fetch the relevant ticket"

`get_task_by_id` with the id the user gave. If they gave a title, search the list
with `get_project_with_undone_tasks` and match the title; ask if more than one fits.
Read the comments too when the skill needs history.

## Listing work

- Open tasks in the list: `get_project_with_undone_tasks`.
- By state: `filter_tasks` with `projectIds: [6ade2cbf8f081c311be5b39d]` and `tag: [<state tag>]`.
  Always pass the project id: tags are shared with the owner's other lists.

## Agent briefs and triage notes

Briefs exceed the 1024-char comment limit. Write the brief into the task `content`
under `## Agent Brief` (replacing an older brief, keeping the original report above
it), then add a short comment: "Agent brief added to the description." Triage notes
work the same way under `## Triage Notes`. Comments still start with the triage
disclaimer.

## Wayfinding operations

Used by `/wayfinder`.

- **Map**: a parent task tagged `map`; its `content` holds Notes / Decisions so far / Fog.
- **Child ticket**: a subtask of the map, title prefix `NN `, the question in `content`. A `Type:` line records `research` / `prototype` / `grilling` / `task`; a `Status:` line records `claimed` / `resolved`.
- **Blocking**: a `Blocked by:` line of task ids. A ticket is unblocked when every listed task is resolved.
- **Frontier**: open, unblocked, unclaimed subtasks of the map; lowest `NN` wins.
- **Claim**: set `Status: claimed` in `content` and save before any work.
- **Resolve**: append the answer under `## Answer`, set `Status: resolved`, complete the task, then add a pointer (gist + task id) to the map's Decisions so far.
