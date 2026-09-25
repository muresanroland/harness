# harness-7nq.5 — Plan review: the docked modal with the plan's markdown styled

## Context

Today a plan Question (`Ask::Plan`) shows in the Shell's QUESTION box: plain wrapped text, 10-row PgUp/PgDn, options numbered under it. The map ticket harness-0sx.5 settled layout C (Docked) from `prototype/modal`: the plan leaves the Question box and docks beside the live Shell, styled markdown, reading keys, feedback typed inside the modal; under 110 columns it folds to a rounded box over the dimmed Shell. This Ticket builds that. Scope: the Ticket's description + AC only (no Epic summary, no /config — those are .6 and .9).

## Changes

### `src/shell.rs` (state + keys)
- `Screen::modal() -> bool`: front Question showing and it is `About::Asked(Ask::Plan{..})`.
- New `Screen` fields, set by the draw (same pattern as the existing `scroll: Cell`s):
  - `page: Cell<usize>` — the plan body's page at the last draw (body rows − 2, min 1, as prototyped).
  - `heads: RefCell<Vec<usize>>` — body rows the headings start on at the last draw (for Tab).
  - `opened: Cell<Option<DateTime<Local>>>` — when the modal was first drawn; the draw sets it when None and clears it when no modal shows; `answer()` clears it so the next plan counts from its own opening. Feeds the "N new on RECENT" badge (events with `time > opened`).
- `key()`: in the Question-keys block, when `modal()`, guards first: ↑↓ a line, PgUp/PgDn/Space a page, Home/End, Tab/Shift-Tab (BackTab) next/previous heading → one `read(code)` method that moves `questions[0].scroll` (saturating; the draw clamps). ←→ move the option cursor (also harmless for other Questions); digits 1..=n pick (existing); Enter answers (existing `answer`), Esc hides (existing), other chars start a command (existing).
- While composing feedback, PgUp/PgDn call `read` (a page, not 10 rows). Enter → existing `reply("feedback", Answer::Prompt)`; Esc back to options (existing).
- Delete `scroll_plan` (replaced by `read`).

### `src/shell/draw.rs`
- `draw(f, s)`: when `s.modal()` → `modal::plan(f, s)`, else the Shell over the whole area; quantize stays last, over the whole buffer.
- Current body becomes `shell(f, area, s)` (the Shell drawn into any rect). Its Question box skips a plan (`s.showing() && !s.modal()`).
- `question_lines`: drop the Plan branch (plan rows + `judged` for Plan) and the Plan hint — dead now.
- `input_line`: while the modal composes, the typed text is in the modal; the input line draws `› ` alone.

### `src/shell/draw/modal.rs` (new, child of draw; uses draw's private `fg`, `bold`, `cut`, `shell`)
- `dock(f, s, title, hint) -> (Rect, bool)` — the frame of its own (reusable by /config, Ticket 9):
  - ≥110 cols: Shell in the left 42%, thick purple box in the right 58%.
  - <110 cols: Shell full, dimmed (fg lerped toward a dark tint) above the input line; a rounded purple box over it filling all but the input line, with margins (w/12, 2 rows) from 100x30 up.
  - Returns the inner rect and whether folded.
- `plan(f, s)`: title `PLAN · <suffix> <title>` (`s.name`), hint on the bottom border (short under 90 box cols; composing: "Enter sends the feedback, Esc goes back"). Inside:
  - badge line: `judged: <plan_said>` / under 100 cols `judged 0.62` (raw score, as prototyped); `· N more waiting` (questions.len()−1); `· N new on RECENT` / under 100 cols `· N new`.
  - lead: the Question's text (e.g. "plan ready in implement (pane 1-1)"), muted, cut.
  - body: `md(plan, width−1)` from the clamped scroll, scrollbar (ratatui `Scrollbar`, no end symbols) in the last column when it overflows; sets `page`, `heads`, `opened`.
  - foot: a rule, then docked the options listed vertically (`› 1. approve` …), folded one row (` 1 approve   2 feedback of your own …`, cursor highlighted; under 80 terminal cols the first word only). Composing: option "feedback of your own"'s row (fold: the whole row) becomes `feedback › <text>▌`, showing the tail when it overflows.
- `md(text, width) -> (Vec<Line>, Vec<usize>)`: ported from the prototype, heading rows returned directly. h1 purple bold underlined (Modifier::UNDERLINED), h2 cyan bold, h3 bold; `- `/`* ` → `• ` (nested `◦ `) and `N. ` with hanging indents; `> ` → `│ ` muted italic; inline `` `code` `` orange on a tint, `**bold**`; fenced code on a tinted ground, `+` green, `-` red, `@@` cyan, chunked to width. Local tint constants (CODE_BG, ADD_BG, DEL_BG, DIM_TO).

## Tests (`src/shell/shell_test.rs`, test-first)

1. **Renders** a sample plan with every markdown element (plus a Wake queued behind, a Judgment of 0.62, a line said after opening):
   - 160x45 docked: Shell (TICKETS, RECENT, input) left of a `┏` box titled `PLAN · 11 Questions`; badges `judged: plan follows the Ticket 0.62 · 1 more waiting · 1 new on RECENT`; h1 PURPLE+UNDERLINED, h2 CYAN, h3 BOLD, `• `/`◦ `, numbered item's continuation under its text, quote `│ ` italic MUTED, inline code ORANGE on tint, bold modifier, fenced `+` GREEN / `-` RED / `@@` CYAN on tints; scrollbar thumb in the last column; options vertical; no ` QUESTION ` box.
   - 100x30 folded: `╭` box with margins, long badges, options in one row, input line bottom row, Shell cells outside the box dimmed.
   - 80x24 folded: box from row 0 to 22, short badges (`judged 0.62`, `1 new`), full option words; at 79 cols the first words only.
2. **Keys** (Screen-only): Tab jumps to the next heading and Shift-Tab back; PgDn/Space move `page` rows; Home/End; ←→ move the cursor; Esc hides (Shell draws full, status row counts it), Esc on an empty input shows it again.
3. **Wake queued behind**: answering the plan shows the Wake in the Shell's QUESTION box, no modal.
4. **Feedback → Answer::Prompt** over the fake world: rewrite `a_plan_question_scrolls_its_plan_by_rows_and_sends_feedback_then_approval` for the modal — PgDn a page, pick 2 composes `feedback › cover y too` inside the modal, PgUp still scrolls while typing, Enter sends → "plan sent back with your feedback", revised plan back at row 0, approve; keep its log-order asserts.

## Decisions made without asking
- Badge/option-shortening thresholds read literally off the terminal width (under 100 / under 80), not the box width.
- h1 "underlined" = `Modifier::UNDERLINED`, not the prototype's extra ━ row.
- Lead = the Question's own text (names the Stage and pane); the prototype's sentence assumed a Judgment that may not exist.
- Badges sit inside the box as its first line in both layouts (not on the fold's border).
- Digits pick 1..=n (a kept feedback adds a 5th option), hint says `1-n`.
- Typing any other char while the modal shows starts a command on the input line, as with every Question today.
- "N new" counts events since the modal was first drawn; hiding and showing again restarts it.

## Verification
- `cargo check`, `cargo clippy` if installed, `cargo test shell`, then full `cargo test`.
- `/code-review` against `main`, fix findings; commit; result file.
