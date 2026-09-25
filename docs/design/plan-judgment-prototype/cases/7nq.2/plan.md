# harness-7nq.2: TICKETS as Sections, and the status row's counts

## Context
Map ticket harness-0sx.4 chose layout B "Sections" (prototype `prototype/screen-updates`,
`src/shell/draw/prototype_screen.rs`, `sections_b` / `status_row` / `Proto::st|look|stage|listed|epic_status`).
This Ticket builds its TICKETS tree and live status row into `src/shell/draw.rs`. RECENT, MERGE TO
UNBLOCK, the / and @ lists stay for Tickets 3 and 4. RECENT keeps its box and newest-first order here.

## Changes

### `src/orchestrator/scheduler.rs`
- `BdDependency.depends_on_id` and `kind` become `pub(crate)`, so the draw can read `blocks` deps.
  `bd list --json --brief --all` already returns them (checked), and `load_epics` keeps them.

### `src/shell/draw.rs` (lift from the prototype, drop the variants)
- `Status` becomes `Working, NeedsYou, Waiting, ToMerge, Merged, Parked, Queued`.
  `status(s, t)`: state first (parked; running → NeedsYou if `s.running && s.blocked()`, else Working;
  pr-open → ToMerge; merged → Merged), then bd closed → Merged, then `s.running && waits_on(s, t)` →
  Waiting, then bd in_progress → Working, else Queued.
- New `waits_on(s, t) -> Option<&str>`: the PR URL for an open Ticket with a bd `blocks` dependency on a
  Ticket whose State status is pr-open. Ticket 3's MERGE TO UNBLOCK box can reuse it.
- `EPIC_COLORS = [PURPLE, CYAN, ORANGE, PINK, BLUE, GREEN]`, picked by each Epic's place among the listed Epics.
- `listed(s)`: idle lists every Epic; live lists `e.id == state.epic` or any Ticket in `state.tickets`.
- `sections(s, width) -> Vec<Line>` replaces `ticket_table`:
  - Rule line `━━ ▾ <id>  <title> ━━━… <detail>  <WORD:<11>`. The id and title are bold in the Epic
    color, cut with …. The fill is `lerp((c, BORDER), 0.55)`. The detail is MUTED and WORD is bold.
  - Idle: detail gives the nonzero counts `N closed · N in progress · N open` (closed = Merged,
    open = Queued, in progress = the rest). WORD is RESUMABLE purple for the saved Epic, else
    ALL CLOSED green (n>0, all Merged: fold to `▸` with `all N closed` and no Ticket rows), else
    IN PROGRESS TEXT (any Ticket not Queued), else NOT STARTED MUTED.
  - Live: detail `M/N merged`, WORD RUNNING purple.
  - Ticket rows `   ├─ ` / `   └─ ` (dimmed Epic color): indicator, `<suffix> <title>` cut with …,
    stage `{:>16}`, label `{:<11}`, per the prototype's `look`. Live: ● WORKING (Ticket color, pulses),
    ◆ NEEDS YOU orange, ◇ WAITING muted with `waits on PR #N`, ○ TO MERGE blue with `PR #N`, ✓ MERGED
    green with `PR #N merged`, ◌ PARKED, · queued with no label. Idle: ● IN PROGRESS, ✓ CLOSED.
    Otherwise the stage is `<stage> <round>`, as today.
  - Add a small `cut(text, width)` helper for the … cut.
- `draw`: no TICKETS box. Tree height = `lines.len().min(free - 4).max(3)`, keeping today's Question
  squeeze. Render at x+1 like the status row. The scroll is clamped here: `from = scroll.min(len - h)`,
  written back. An overflowing tree ends in `   … N more, PgDn`.
- `status_line`, live: `<spin> RUNNING  ● N working  ◆ N needs you  ◇ N waiting on a merge  ○ N to merge
  ◌ N parked  ✓ N merged`, two spaces apart. Each glyph is bold in its part's color and each count in
  that color (working TEXT, needs you ORANGE, waiting MUTED, to merge BLUE, parked MUTED, merged GREEN).
  Parked shows only when nonzero. Counts cover every Ticket of the listed Epics. If the line is wider
  than the row, it is rebuilt without glyphs, and the Line render cuts the end. STOPPING and the
  hidden-Questions suffix stay. The idle row does not change.
- Drop the under-60-column label drop (the prototype B has none). Remove unused `Table`/`Row`/`Cell` imports.

### `src/shell.rs`
- `Screen.scroll: usize` becomes `Cell<usize>`, the same as `Question.scroll`. The draw knows the height
  and clamps it, so PgUp/PgDn/Up/Down (input empty) move it only when the tree overflows. Key arms use
  `saturating_add_signed`. Delete `Screen::rows()`, which has no other use. Up/Down stay on TICKETS.

## Tests (TDD, `src/shell/shell_test.rs`)
New helper `screen_over(bd_json, state)`: `super::load_epics` over `Fake::new`, then `Screen::new`.
One bd list JSON: Epic A is saved and gets mixed Tickets, including `blocks` on A.3. Epic B has every
Ticket closed. Epic C has one in_progress Ticket. Epic D has only open Tickets.
1. Idle: one rule per Epic, with the id colored PURPLE, CYAN, ORANGE and PINK by place. The WORDs are
   RESUMABLE, ALL CLOSED (`▸`, `all 2 closed`, no Ticket rows), IN PROGRESS and NOT STARTED. The detail
   shows the counts. The Ticket rows read IN PROGRESS and CLOSED.
2. Live (`running = true`, State plus a Question on one Ticket): only A is listed, in PURPLE, as
   `2/N merged RUNNING`. The rows are WORKING+stage, NEEDS YOU, WAITING `waits on PR #31`, TO MERGE
   `PR #31`, MERGED `PR #29 merged`, PARKED, and queued with no label.
3. Status row: the counts per label, with parked hidden at zero and shown when nonzero. The open PR
   counts as `1 to merge`. At a narrow width the glyphs drop. Narrower still, the end is cut (no `merged`).
4. Scroll: a tall tree scrolls with PgDn/Up and is clamped by the draw. A tree that fits does not scroll.
Existing tests move to the new text (ACTIVE→WORKING/IN PROGRESS, DONE→CLOSED/MERGED, BLOCKED→NEEDS YOU,
`●  9` → `● 9`, the status row strings, `" TICKETS "` gone, `s.rows()` gone). Their intent stays.

## Decisions (not given in the Ticket)
- Keep `wait_dependents`' Event line. "State, not an Event line" is read as: the Shell derives Waiting.
- Drop the label under 60 columns. Keep the 10-row page step.
- NeedsYou and Waiting show only live. Idle rows fall back to IN PROGRESS/queued.

## Verification
`cargo check`, `cargo clippy`, `cargo test shell`, then the full `cargo test`. `/code-review` against
`main`, then commit. The result file goes to `.harness/runs/harness-7nq.2/implement.md`.
