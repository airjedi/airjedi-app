# Parallel Worktree Development Skill - Design

## Problem

Executing multiple independent tasks from a plan (or ad-hoc) sequentially is slow. Existing skills either run tasks one-at-a-time (subagent-driven-development) or provide parallel investigation without structured integration (dispatching-parallel-agents). No skill combines git worktree isolation with parallel tmux-based workers and structured integration.

## Solution

A global skill (`~/.claude/skills/parallel-worktree-dev/SKILL.md`) that:

1. Accepts a plan doc or ad-hoc task descriptions
2. Identifies independent tasks
3. Creates one git worktree per task
4. Launches one Claude Code tmux worker per worktree (via claude-session-driver scripts)
5. Monitors all workers, reports progress
6. Reviews each worker's output
7. Integrates results (merge or PR, user's choice)
8. Cleans up worktrees and tmux sessions

## Architecture

```
Controller (main session)
  ├── parse plan/tasks → identify independent groups
  ├── create worktrees (.worktrees/<task-branch>)
  ├── launch workers (tmux sessions)
  │   ├── worker-1 → .worktrees/feature-a/
  │   ├── worker-2 → .worktrees/fix-b/
  │   └── worker-3 → .worktrees/refactor-c/
  ├── monitor (poll events, report progress)
  ├── review each worker's output
  └── integrate (merge or PR, per user choice)
```

## Input Modes

**Plan-driven**: Read plan doc, extract task sections, identify independent tasks, parallelize those, run dependent tasks sequentially after.

**Ad-hoc**: User describes 2+ tasks directly. All treated as independent unless user specifies dependencies.

## Worker Lifecycle

1. Create worktree: `git worktree add .worktrees/<branch> -b <branch>`
2. Launch: `launch-worker.sh worker-<n> <worktree-path>`
3. Prompt with task description + project context (CLAUDE.md, relevant files)
4. Monitor via event polling, report progress
5. Wait for `stop` event
6. Review: self-review prompt or separate review worker
7. Stop: `stop-worker.sh`

## Integration Phase

1. List completed branches with change summaries
2. Ask user: merge directly, create PRs, or skip
3. Merge sequentially (pause on conflicts) or create PRs via `gh`
4. Clean up worktrees: `git worktree remove`

## Error Handling

- Worker crash: detect via tmux check, offer relaunch
- Timeout: configurable (default 10 min), offer extend or kill
- Merge conflict: pause, show conflict, ask user
- Test failure: flag, ask user whether to integrate anyway

## Dependencies

- claude-session-driver plugin (tmux scripts)
- tmux, jq on PATH
- Existing `.worktrees/` directory or will create one (with gitignore check)

## Scope

Global skill at `~/.claude/skills/parallel-worktree-dev/SKILL.md`. Project-agnostic.
