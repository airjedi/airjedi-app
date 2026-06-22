# Parallel Worktree Development Skill - Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create a global Claude Code skill that parallelizes independent tasks across git worktrees using tmux-based Claude Code workers.

**Architecture:** A single SKILL.md file at `~/.claude/skills/parallel-worktree-dev/SKILL.md` that instructs Claude to orchestrate worktree creation, worker launch via claude-session-driver scripts, monitoring, review, and integration. No code to write -- this is a process skill (documentation only).

**Tech Stack:** Claude Code skills (YAML frontmatter + markdown), claude-session-driver plugin scripts, git worktrees, tmux

---

### Task 1: Create the skill directory and SKILL.md

**Files:**
- Create: `~/.claude/skills/parallel-worktree-dev/SKILL.md`

**Step 1: Create the skill directory**

```bash
mkdir -p ~/.claude/skills/parallel-worktree-dev
```

**Step 2: Write the SKILL.md file**

The skill file must include:

1. **YAML frontmatter** with `name` and `description` fields. The description should trigger on phrases like "parallel development", "work on multiple tasks", "fan out", "parallel worktrees", "execute plan in parallel".

2. **Overview section** explaining what the skill does and when to use it.

3. **Prerequisites section** listing required tools (tmux, jq, claude CLI, claude-session-driver plugin).

4. **Input Modes section** describing plan-driven and ad-hoc modes.

5. **Process Flow** as a numbered checklist the agent follows:
   - Determine SCRIPTS path (find claude-session-driver plugin scripts)
   - Parse input (plan doc or ad-hoc tasks)
   - Identify independent task groups
   - Verify `.worktrees/` directory exists and is gitignored
   - Create one worktree per independent task
   - Launch one tmux worker per worktree
   - Send task prompt to each worker (include CLAUDE.md context)
   - Monitor workers (poll events, report progress to user)
   - Wait for all workers to complete (with timeout)
   - Review each worker's output (via `read-turn.sh`)
   - Present results to user
   - Ask user: merge directly, create PRs, or skip integration
   - Execute chosen integration strategy
   - Clean up worktrees and tmux sessions

6. **Worker Prompt Template** showing what to send each worker:
   - Task description from the plan
   - Project context (CLAUDE.md path, relevant source files)
   - Instructions to commit work when done
   - Instructions to run tests before finishing

7. **Monitoring section** with commands for checking worker status, viewing live output via `tmux attach`, and reading events.

8. **Integration section** with merge and PR workflows.

9. **Error handling section** covering worker crashes, timeouts, merge conflicts, and test failures.

10. **Quick reference table** of common commands.

11. **Red flags** section listing what to never do and what to always do.

12. **Example workflow** showing a complete run from start to finish.

Write the complete SKILL.md with all sections. The file should be self-contained -- an agent reading it should be able to execute the entire parallel workflow without referring to other documents.

Key implementation details to include:

**Finding scripts path:**
```bash
SCRIPTS=$(find ~/.claude/plugins/cache -path "*/claude-session-driver/*/scripts" -type d 2>/dev/null | head -1)
```

**Worktree creation pattern:**
```bash
git worktree add .worktrees/<branch-name> -b parallel/<branch-name>
```

**Worker launch pattern:**
```bash
RESULT=$("$SCRIPTS/launch-worker.sh" "pw-<task-name>" "<worktree-path>")
SESSION_ID=$(echo "$RESULT" | jq -r '.session_id')
```

**Worker prompt pattern:**
```bash
"$SCRIPTS/converse.sh" "pw-<task-name>" "$SESSION_ID" "<task-prompt>" 600
```

**Monitoring pattern:**
```bash
# Check if worker is still running
tmux has-session -t "pw-<task-name>" 2>/dev/null

# Read events
"$SCRIPTS/read-events.sh" "$SESSION_ID" --last 3
```

**Integration patterns:**
```bash
# Merge
git checkout main && git merge parallel/<branch-name>

# PR
gh pr create --head parallel/<branch-name> --title "<title>" --body "<body>"

# Cleanup
git worktree remove .worktrees/<branch-name>
git branch -d parallel/<branch-name>
```

**Step 3: Verify the skill is discoverable**

Run: `ls -la ~/.claude/skills/parallel-worktree-dev/SKILL.md`
Expected: File exists with non-zero size

**Step 4: Commit**

This is a global skill, not part of the project repo. No git commit needed.

---

### Task 2: Test the skill with a dry-run scenario

**Step 1: Start a new Claude Code session and check skill is listed**

The skill should appear when listing available skills. Verify by checking that Claude Code can find it:

```bash
find ~/.claude/skills -name "SKILL.md" -type f
```

Expected: `~/.claude/skills/parallel-worktree-dev/SKILL.md` in the output

**Step 2: Verify prerequisites check works**

In a test session, ask Claude to use the parallel worktree skill. It should:
- Check for tmux (should be installed)
- Check for jq (should be installed)
- Check for claude-session-driver scripts (should find them in plugin cache)

**Step 3: Verify worktree setup works**

In the airjedi-bevy project, verify the `.worktrees/` directory is gitignored:

```bash
cd /Users/ccustine/development/aviation/airjedi-bevy
git check-ignore .worktrees/
```

Expected: `.worktrees/` is ignored

---

### Task 3: Update the design doc with final skill location

**Files:**
- Modify: `docs/plans/2026-02-26-parallel-worktree-skill-design.md`

**Step 1: Add the final skill path to the design doc**

Add a "Final Location" section noting `~/.claude/skills/parallel-worktree-dev/SKILL.md`.

**Step 2: Commit the design doc**

```bash
git add docs/plans/2026-02-26-parallel-worktree-skill-design.md
git commit -m "Add parallel worktree development skill design doc"
```
