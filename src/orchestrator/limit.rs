//! Limited: a Stage's App at its provider's usage limit, read from the last
//! lines of its pane. Never a Wake: the App holds until the reset.

use std::path::Path;
use std::sync::atomic::Ordering;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone};
use regex::Regex;

use super::app::{app, App};
use super::result::{read_stage_result, ResultRequirements};
use super::stage::{Held, Orchestrator, Stage};
use super::state::TicketState;

/// Only the pane's last lines are read, so an old limit line in the
/// scrollback, or an agent quoting one, is not taken for a live limit.
pub(super) const LAST_LINES: usize = 20;

/// Claude's usage-limit options menu, which opens instead of its own wait
/// for a reset more than a day away (docs/research/usage-limits.md).
const MENU: &str = "What do you want to do?";

/// Claude carries on by itself at the reset: a session still idle this long
/// after it is told to, and the App holds until then.
const GRACE: Duration = Duration::minutes(2);

/// A usage limit shown in a Stage's pane.
#[derive(Clone, Debug)]
pub(crate) struct Limit {
    pub(crate) app: &'static str,
    /// Which limit, as the App names it: "session limit", "usage limit".
    pub(crate) what: String,
    pub(crate) reset: DateTime<Local>,
    /// A reset more than a day away, or Claude's options menu: the run
    /// ends rather than hold.
    pub(crate) long: bool,
    /// The line it was read from.
    pub(crate) line: String,
}

/// The limit the last lines of `tail` show for `app`: the newest line one
/// of its patterns matches whose reset is still ahead of `now`.
pub(crate) fn find(app: &'static App, tail: &str, now: DateTime<Local>) -> Option<Limit> {
    let patterns: Vec<Regex> = app.limits.iter().map(|p| Regex::new(p).unwrap()).collect();
    let last: Vec<&str> = tail.lines().rev().take(LAST_LINES).collect();
    for line in &last {
        for caps in patterns.iter().filter_map(|p| p.captures(line)) {
            let reset = match caps.name("reset") {
                Some(text) => parse_reset(text.as_str(), now),
                None => Some(now + Duration::hours(1)), // look again then
            };
            let Some(reset) = reset.filter(|reset| *reset > now) else {
                continue;
            };
            return Some(Limit {
                app: app.name,
                what: caps
                    .name("what")
                    .map_or("usage limit", |w| w.as_str())
                    .to_string(),
                reset,
                long: reset > now + Duration::hours(24) || last.iter().any(|l| l.contains(MENU)),
                line: line.to_string(),
            });
        }
    }
    None
}

/// A reset as the Apps print it, in the machine's own zone (Claude's
/// "(Zone)" is ignored): "3:45pm", "Mon 12:00am", "Sep 25, 3pm",
/// "Sep 24th, 2026 3:05 PM". A time alone is its next occurrence, a
/// weekday its next such day; a date without a year is this year's.
fn parse_reset(text: &str, now: DateTime<Local>) -> Option<DateTime<Local>> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let re = Regex::new(
        r"(?i)^(?:(?P<wd>mon|tue|wed|thu|fri|sat|sun)[a-z]*,?\s+)?(?:(?P<mon>[a-z]{3})[a-z]*\s+(?P<day>\d{1,2})(?:st|nd|rd|th)?,?\s+(?:(?P<year>\d{4}),?\s+)?)?(?P<h>\d{1,2})(?::(?P<m>\d{2}))?\s*(?P<ap>am|pm)",
    )
    .unwrap();
    let caps = re.captures(text.trim())?;
    let num = |name: &str| caps.name(name).and_then(|m| m.as_str().parse::<u32>().ok());
    let pm = caps["ap"].eq_ignore_ascii_case("pm");
    let time = NaiveTime::from_hms_opt(
        num("h")? % 12 + if pm { 12 } else { 0 },
        num("m").unwrap_or(0),
        0,
    )?;
    let local = |date: NaiveDate| Local.from_local_datetime(&date.and_time(time)).earliest();
    let today = now.date_naive();
    if let Some(mon) = caps.name("mon") {
        let month = MONTHS
            .iter()
            .position(|m| mon.as_str().eq_ignore_ascii_case(m))? as u32
            + 1;
        // ponytail: a yearless date is this year's; one read on 31 Dec
        // for 2 Jan is past, a Wake. Claude dates only resets days away.
        let year = num("year").map_or(now.year(), |y| y as i32);
        return local(NaiveDate::from_ymd_opt(year, month, num("day")?)?);
    }
    if let Some(wd) = caps.name("wd") {
        let want = DAYS
            .iter()
            .position(|d| wd.as_str().eq_ignore_ascii_case(d))? as u32;
        return (0..8)
            .map(|n| today + Duration::days(n))
            .filter(|d| d.weekday().num_days_from_monday() == want)
            .filter_map(local)
            .find(|reset| *reset > now);
    }
    [today, today + Duration::days(1)]
        .into_iter()
        .filter_map(local)
        .find(|reset| *reset > now)
}

/// A reset as the screen and the events say it: "3:45pm" today, "Mon
/// 12:00am" on another day.
pub(crate) fn until(reset: DateTime<Local>, now: DateTime<Local>) -> String {
    match reset.date_naive() == now.date_naive() {
        true => reset.format("%-I:%M%P").to_string(),
        false => reset.format("%a %-I:%M%P").to_string(),
    }
}

/// Whether a limit that resets at `reset` still holds its App at `now`.
pub(crate) fn holds(reset: DateTime<Local>, now: DateTime<Local>) -> bool {
    now < reset + GRACE
}

impl Orchestrator {
    /// When `app`'s usage limit resets, while it holds; a passed one is
    /// forgotten.
    fn limited_until(&self, app: &str) -> Option<DateTime<Local>> {
        let reset = *self.state.lock().unwrap().limits.get(app)?;
        if holds(reset, (self.cfg.clock)()) {
            return Some(reset);
        }
        self.change_state(|state| {
            if state.limits.get(app) == Some(&reset) {
                state.limits.remove(app);
            }
        });
        None
    }

    /// Holds the Ticket while `app` is Limited, its row reading so: a Stage
    /// about to start on it waits, a session on it is left alone, and no
    /// deadline runs. None once the limit is over, Some(Park) on /park,
    /// Some(Stopped) on /stop-work, the saved row still limited.
    pub(super) fn wait_limit(&self, ticket: &str, label: &str, app: &str) -> Option<Held> {
        let reset = self.limited_until(app)?;
        let when = until(reset, (self.cfg.clock)());
        self.log(
            ticket,
            &format!("{label} holds: {app} limited until {when}"),
        );
        self.update(ticket, |ts| ts.limited = app.to_string());
        let held = loop {
            if self.consume(&format!("park-{ticket}")) {
                break Some(Held::Park);
            }
            if !self.sleep() {
                return Some(Held::Stopped);
            }
            if self.limited_until(app).is_none() {
                break None;
            }
        };
        self.update(ticket, |ts| ts.limited.clear());
        held
    }

    /// The usage limit the last lines of `tail`, the Stage's pane's, show
    /// for the App its session runs on; not the line the session in `pane`
    /// was resumed from, which a time alone would read as tomorrow's.
    pub(super) fn limit_shown(
        &self,
        ts: &TicketState,
        st: &Stage,
        pane: &str,
        tail: &str,
    ) -> Option<Limit> {
        let app = app(&ts.sessions.get(st.name)?.app)?;
        let limit = find(app, tail, (self.cfg.clock)())?;
        (self.resumed.lock().unwrap().get(pane) != Some(&limit.line)).then_some(limit)
    }

    /// A Stage's session at a usage limit, which holds its App from now. A
    /// long one ends the run: each Ticket closes its tab as it leaves, its
    /// session id saved for /continue. A short one leaves the session alone
    /// until the reset + GRACE (Claude carries on by itself), then tells it
    /// to continue if it is still idle with no result, and watches it again
    /// with a fresh deadline. No nudge, wait or retry is spent.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn limited(
        &self,
        ticket: &str,
        st: &Stage,
        label: &str,
        pane: &str,
        file: &Path,
        want: ResultRequirements,
        limit: Limit,
    ) -> Held {
        let Limit {
            app,
            what,
            reset,
            long,
            line,
        } = limit;
        self.change_state(|state| {
            let saved = state.limits.entry(app.to_string()).or_insert(reset);
            *saved = (*saved).max(reset);
        });
        let when = until(reset, (self.cfg.clock)());
        if long {
            self.closed.store(true, Ordering::SeqCst);
            self.stop();
            self.report(
                "",
                &format!(
                    "{app} {what} until {when}: sessions saved, panes closed, /continue after the reset"
                ),
            );
            return Held::Stopped;
        }
        let at = self.locate(pane);
        self.report(
            ticket,
            &format!("{app} {what} until {when}: {label} holds {at}"),
        );
        if let Some(held) = self.wait_limit(ticket, label, app) {
            return held;
        }
        self.resumed.lock().unwrap().insert(pane.to_string(), line);
        let idle = matches!(
            self.watch(ticket, st, pane).as_deref(),
            Some("idle" | "done")
        );
        if idle && !read_stage_result(file, want).1.is_empty() {
            if let Err(err) = self.herdr(&["agent", "prompt", pane, "continue"]) {
                return Held::Woke(format!("never took the continue: {err}"));
            }
        }
        self.report(
            ticket,
            &format!("{app} {what} over: {label} carries on {at}"),
        );
        self.hold(ticket, st, label, pane, file, want, true, None)
    }
}
