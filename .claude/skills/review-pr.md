---
name: review-pr
description: >
  Review a pull request for code quality, correctness, test coverage,
  and adherence to project conventions. Posts a structured review
  summary and inline comments on specific issues. Invoke with a PR
  number: /review-pr 5
---

# Review PR

You are reviewing a pull request. Your job is to assess code quality,
correctness, test coverage, and project convention adherence.

## Getting Started

Determine the repo and PR number. The user will provide a PR number.

```bash
REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
```

## Steps

### 1. Read the PR

```bash
gh pr view "$PR_NUMBER"
gh pr diff "$PR_NUMBER"
```

Understand:
- What issue does it address? (check for "Closes #N")
- What changed and why?
- How large is the diff?

### 2. Read the Changed Files in Full

Don't just read the diff - read the full files to understand context.
For each changed file, understand:
- What module/plugin does it belong to?
- How does it interact with the rest of the system?
- Are there existing tests for this code?

### 3. Check for Issues

**Correctness:**
- Does the code do what the PR/issue says it should?
- Are there off-by-one errors, missing edge cases, or logic bugs?
- Does it handle error cases properly?

**Rust-Specific:**
- Unnecessary `.clone()` or `.unwrap()` in non-test code
- Missing error propagation (using `?` instead of `.unwrap()`)
- Ownership patterns - borrowing where possible
- Proper use of iterators vs manual loops

**Bevy-Specific:**
- Correct system ordering (`.after(ZoomSet::Change)` where needed)
- Proper use of ECS patterns (Components, Resources, Events)
- Camera layer assignments match the documented architecture
- Coordinate system usage (Web Mercator meters, LocalOrigin)

**Tests:**
- Are there tests for new functionality?
- Do existing tests still cover the changed code?
- Are edge cases tested?

**Style:**
- Does it match the existing code style?
- Commit messages use plain imperative (no conventional commit prefix)
- No unrelated changes mixed in

### 4. Post Review

Check for existing review comment:

```bash
gh pr view "$PR_NUMBER" --comments --json comments \
  | jq '.comments[] | select(.body | contains("> **review-agent**"))'
```

**Review comment format:**

```markdown
> **review-agent**

## PR Review

### Summary

[1-2 sentence summary of what the PR does and overall assessment]

### Verdict: [Approve / Request Changes / Needs Discussion]

### Strengths

- [What's done well]

### Issues Found

**[severity: Critical / Warning / Suggestion]** `file.rs:L42`
[Description of the issue and suggested fix]

### Test Coverage

[Assessment of test coverage for the changes]

### Checklist

- [x/] Code correctness
- [x/] Error handling
- [x/] Test coverage
- [x/] Style consistency
- [x/] No unrelated changes
```

For specific code issues, post inline PR comments:

```bash
gh api repos/$REPO/pulls/$PR_NUMBER/comments \
  -f body="[comment]" \
  -f path="[file]" \
  -f line=[line_number] \
  -f commit_id="$(gh pr view $PR_NUMBER --json headRefOid -q .headRefOid)"
```

## Guidelines

- Be constructive, not nitpicky. Focus on correctness and clarity.
- Distinguish between blocking issues and suggestions.
- If the PR is from an agent (claude/ branch), verify it matches the
  implementation plan posted on the issue.
- Don't request changes for style preferences that aren't in CLAUDE.md.
