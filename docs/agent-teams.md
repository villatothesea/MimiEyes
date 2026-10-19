# Agent teams in MimiEyes

A team is a few sessions of one project working together — say a planner, a
worker and a researcher — each in its own pane. Panes belong to a tab, tabs to
a project; the tree sidebar shows it as **project → tab → pane**.

## What an agent can use

Every session's agent has the Zeron MCP tools. The team needs these:

| Tool | Use |
|---|---|
| `create_chat` / `create_chats` | Start teammates: `harness`, `model` (ids from `list_models`), `reasoning`, `title` (the name the user sees), `sandbox`, a first `prompt`. |
| `panes` | MimiEyes only. `list` the project's tabs and panes; `arrange` a tab (or a new one) as a split tree of sessions. |
| `send_message` / `send_messages` | Give work. Every message carries its sender, so a teammate knows whom to answer. |
| `wait_for_turn` | Collect results. Long tasks: send with `wait: false`, then wait. `wait: true` blocks your turn and can time out. |
| `read_chat` | Read a teammate's whole transcript. |
| `interrupt_chat` | Stop a teammate going the wrong way. |

Sessions are addressed by full id, a unique id prefix (the 8 characters on a
pane's tag), or exact title. Titles must be unique in the project to be used
as names.

## Laying out panes

`arrange` takes a tree. A pane is `{"chat": "self" | id | prefix | title}`; a
split is `{"split": "right" | "down", "ratio"?: 0.1–0.9, "a": …, "b": …}`
(`right`: a | b side by side; `down`: a over b; no ratio: rows and columns
share evenly).

```json
{ "op": "arrange",
  "layout": { "split": "right",
              "a": { "split": "down", "a": { "chat": "self" }, "b": { "chat": "Researcher" } },
              "b": { "chat": "Worker" } } }
```

`"tab": "new", "name": "Review"` makes a new tab; `"focus": true` brings it on
screen (otherwise a tab the user is not looking at only lights up in the
strip and the tree).

## Prompt: planner, worker, researcher

> Build a team for this task in this project.
> 1. Create two sessions with `create_chats`: **Worker** (Sonnet 5.5, medium
>    reasoning) writes the code; **Researcher** (Haiku 5.5, medium) reads and
>    does web research and never edits code. Look the model ids up with
>    `list_models`. Give each a first prompt that says its role, that you
>    (`whoami` → your chat id) are the planner, and to write its results to
>    `.team/<role>.md` and then message you a one-line summary.
> 2. Lay them out with `panes`: you over the Researcher on the left, the
>    Worker on the whole right.
> 3. Plan, then send each its tasks with `send_messages` (`wait: false`);
>    collect with `wait_for_turn`, read their files, and continue until done.

## Prompt: parallel coding, then a review tab

> Split this task for up to 3 more agents (4 with you). Write the ownership
> of files to `.team/plan.md`, create the agents with titles, and lay out one
> pane per agent in a 2×2 grid with `panes`. Send the tasks, wait for all to
> finish, then open a new tab named **Review** with a fresh reviewer session
> and yourself, and ask the reviewer to review the whole diff.
