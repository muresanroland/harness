# harness-7nq.4: the / and @ lists above the input

## Context

Ticket harness-7nq.4 (map harness-0sx.4, prototype `prototype/screen-updates:src/shell/draw/prototype_screen.rs`, layout B). Today the Shell lists its commands in a long placeholder and completes arguments with Tab (`Screen::complete`). The Ticket replaces both with inline lists above the input: `/` lists commands from one command table, `@<query>` lists open Epics and Tickets. Scope is only this Ticket's acceptance criteria.

## Changes

### `src/shell.rs`
- `const COMMANDS: [(&str, &str, &str); 9]`, the one command table (name, args, description), using the prototype's wording: /start-epic `<epic> [--max N]`, /start-ticket `<ticket>`, /continue, /stop-work, /retry, /park, /address `<ticket>`, /questions, /exit. Nothing else lists the commands: the placeholder no longer does.
- `Screen.pick: usize` is the list's cursor. It resets to 0 whenever a key types or deletes a character (the Char arm on both key paths, and Backspace).
- `pub(crate) fn list(&self) -> Vec<(&str, &str, &str)>` returns the open list's rows as (what fills in, middle column, text). Empty means no list is open.
  - `/` list: the input starts with `/` and has no space. It shows the commands that contain the input (lowercased). If none contain it, it shows the commands the input is a subsequence of.
  - `@` list: the text after the input's last `@` has no space. Rows are the open Epics `(id, "Epic", title)` and the open Tickets (status != closed) `(id, "Ticket", title)`, in tree order. The command before the `@` narrows them: `/start-epic` gives Epics only; `/start-ticket`, `/retry`, `/park` and `/address` give Tickets only. Rows are ranked id contains the query, then title contains it, then the query is a subsequence of the id. The sort is stable, and rows that match none of these are dropped.
  - No list while `composing`: a prompt of the user's own may hold `/` or `@`.
- `fn subsequence(q, text)`: copied from the prototype.
- `fn fill(&mut self, picked: &str)`: a command sets the input to `'<command> '`. An id replaces `@<query>` with `'<id> '`.
- Changes to `key()` general path, with `(open, picked)` computed from `list()` first:
  - `Up`/`Down` when a list is open move `pick`, clamped. These arms come before the RECENT arms. RECENT keeps its rule: it scrolls only when the input is empty and no list is open.
  - `Tab` fills in the picked row. With no list open, Tab does nothing. `complete()` is deleted.
  - `Enter` with a list open fills in, except when the input already is the picked row. Then it falls through and runs the line.
  - `Esc` already clears the input, which closes the list. No change.
- The leading `@` is stripped from a typed query at the start of `resolve()`, and in the `/retry|/park|/address` arm.
- Update the key() comments that describe arrows and Tab.

### `src/shell/draw.rs`
- `PLACEHOLDER = "  / for a command, @ for an Epic or Ticket"` (still `DARK_ORANGE`).
- `fn epic_color(s, id)` gives an Epic's color by its place in `listed(s)` (0 when not listed). `sections()` uses it too, so the tree and the @ list agree.
- `fn list_lines(s, width, height) -> Vec<Line>` renders a window of at most 8 rows (fewer if `height` is short) around `s.pick`, then the hint line `'  ↑↓ pick · Tab or Enter fills in · Esc clears'` in BORDER.
  - Each row is `'› '` bold PURPLE on the cursor row (else two spaces), then the key padded to the widest key, then the middle column padded, in MUTED, then the text cut with `…`.
  - The text is TEXT on the cursor row and MUTED otherwise.
  - Key color: PURPLE for a command, `epic_color` for an Epic, `ticket_color` for a Ticket. The key is bold on the cursor row.
  - Returns no lines when no list is open.
- Layout in `draw()`: a `Constraint::Length(list_h)` area goes between MERGE TO UNBLOCK and the notice line. The list takes its rows after MERGE TO UNBLOCK, leaving TICKETS its 3 rows (`height = free - 3`), and `free` shrinks by list_h before the TICKETS and Question sums. Update the module and draw() doc comments.

### `src/update.rs`
- Check GitHub Releases every hour while the Shell runs, not only at start: a `thread::spawn` loop over `check()` with a one-hour sleep, the notice shown as today.

## Decisions the Ticket left to me
1. **Enter on an exact command runs it.** "/exit" then Enter exits, and "/ex" then Enter fills in "/exit ". Otherwise every no-argument command would take two Enters. The existing type_line("/exit") and "/questions" tests keep passing.
2. **Nothing matches, no list.** Enter then runs the line as today ("unknown command", "no open Epic matches"). The prototype's "nothing matches" row is dropped.
3. **No list while composing** a prompt for a Question.
4. **@ at any point**: the list opens on the last `@` of the input, as in the prototype. No word-boundary check.
5. **Epic color** of an Epic that is not on the tree (possible in a live run) falls back to the first color.
6. **Up/Down with text on the line and no list open** still do nothing, as the existing test pins. The map's note says "input empty, no list open".

## Tests (`src/shell/shell_test.rs`, test-first)
New fixture `lists_screen()`, built inline with `issue()`:
- harness-0sx "Wayfinder map": .4 "The screen updates" (open), .8 "Limited" (open), .9 "Old" (closed)
- harness-kv9 "Other work": .1 "First"
- harness-7nq "Build": .5 "Plan review", .6 "Closed review" (closed)
- harness-rev "Rework": .1 "Anything"

New tests:
- `the_slash_list_filters_the_command_table_and_fills_in`:
  - `/` lists all nine commands. The test compares against a literal list and against `COMMANDS`.
  - `/pa` gives [/park], and Tab fills in `/park `, which closes the list.
  - `/st` gives /start-epic, /start-ticket and /stop-work. It leaves out /questions, which only matches as a subsequence.
  - `/sw` gives [/stop-work], a subsequence match.
  - `/zz` opens no list, and Enter reports "unknown command: /zz".
  - `/ex` then Enter fills in `/exit ` without quitting. A second Enter quits.
  - Esc clears the input and closes the list.
- `the_at_list_ranks_open_epics_and_tickets_narrowed_by_the_command`:
  - `/start-epic @0s` lists [harness-0sx], and Enter fills in `/start-epic harness-0sx `.
  - `/retry @` lists every open Ticket and no Epic.
  - `@rev` ranks harness-rev and harness-rev.1 (id contains the query), then harness-7nq.5 (title contains it), then harness-kv9 and harness-kv9.1 (id subsequence).
  - `/start-ticket @` does not list the closed Tickets.
- `up_and_down_move_an_open_lists_cursor_and_scroll_recent_when_none_is`:
  - With 10 events and `/` typed, Down twice leaves `recent` at 0. Tab then fills in `/continue `.
  - Down past the end clamps, and Up at the top stays.
  - After Esc, Up scrolls RECENT.
- `the_slash_list_renders_above_the_input_with_its_hint` at 120x40:
  - Rows 29 to 36 hold the first 8 commands, with exact text for `› /start-epic    <epic> [--max N]  run every Ticket of an open Epic`. Row 37 is the hint, and the input is on 39.
  - Colors: PURPLE and bold for the command on the cursor row, MUTED args, TEXT description on the cursor row, MUTED on the others.
  - After Down ×8 the window ends at `› /exit`.
- `the_at_list_renders_ids_in_their_epic_or_ticket_color`: `@0s` gives `› harness-0sx    Epic    Wayfinder map`, the id in PURPLE, and the ticket rows in `ticket_color`, with the hint.

Changed tests:
- `the_placeholder_draws_in_dark_orange` finds the new placeholder text.
- `start_epic_resolves_its_argument…`:
  - Replace the Tab-completion lines with "Tab in the argument slot changes nothing".
  - Add `s.command("/start-ticket @hx-")`, which gives the same "matches:" notice.
- `retry_and_park…`: `/park @hx-1` is refused as the plain id is (the @ is stripped).

- `update_test`: with the clock stepped an hour, a second check runs and a newer release shows its notice.

## Verification
`cargo check`, `cargo clippy`, `cargo test shell` while working, then the full `cargo test` and `cargo build --release`. Then run /code-review against origin/main and fix what it finds. Commit to harness-7nq.4, with no push, and write `.harness/runs/harness-7nq.4/implement.md`.
