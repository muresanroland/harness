//! Limited: a Ticket held because the App its Stage runs on hit its
//! provider's usage limit, read from the last lines of the Stage's pane.
//! Never a Wake: every Stage on that App holds until the reset.

use std::path::Path;
use std::sync::atomic::Ordering;

use chrono::{DateTime, Datelike, Duration, Local, Month, NaiveDate, NaiveTime, TimeZone, Weekday};
use regex::Regex;

use super::app::{app, App};
use super::result::{read_stage_result, ResultRequirements};
use super::stage::{Held, Orchestrator, Stage};
use super::state::TicketState;

/// Only the pane's last lines are read, so an old limit line in the
/// scrollback, or an agent quoting one, is not taken for a live limit.
pub(super) const LAST_LINES: usize = 20;

/// Claude's usage-limit options menu, which opens instead of its own wait
/// for a reset more than a day away (the research/usage-limits branch).
const MENU: &str = "What do you want to do?";

/// How long after the reset the App still holds: Claude carries on by
/// itself at the reset, and a session still idle after this is told to.
const GRACE: Duration = Duration::minutes(2);

/// A usage limit shown in a Stage's pane.
#[derive(Debug)]
pub(crate) struct Limit {
    pub(crate) app: &'static str,
    /// Which limit, as the App names it: "session limit", "usage limit".
    pub(crate) what: String,
    pub(crate) reset: DateTime<Local>,
    /// A reset more than a day away, or Claude's options menu: the run
    /// ends rather than hold.
    pub(crate) long: bool,
    /// The reset gives its date, so its line is never an old one that
    /// reads a day or a week on.
    pub(crate) dated: bool,
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
                None => Some((now + Duration::hours(1), false)), // look again then
            };
            let Some((reset, dated)) = reset.filter(|(reset, _)| *reset > now) else {
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
                dated,
            });
        }
    }
    None
}

/// A reset as the Apps print it, in the machine's own zone (Claude's
/// "(Zone)" is ignored): "3:45pm", "Mon 12:00am", "Sep 25, 3pm",
/// "Sep 24th, 2026 3:05 PM". A time alone is its next occurrence, a
/// weekday its next such day; a date without a year the one nearest
/// today, in this year, the last or the next. True with a date.
fn parse_reset(text: &str, now: DateTime<Local>) -> Option<(DateTime<Local>, bool)> {
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
    // ponytail: Claude's "(Zone)" is its own process's zone, on this machine,
    // so it is read as Local. A Claude pane run under another TZ than Harness
    // is misread; resolving the name needs chrono-tz, a new crate (a ticket).
    let local = |date: NaiveDate| Local.from_local_datetime(&date.and_time(time)).earliest();
    let today = now.date_naive();
    if let Some(mon) = caps.name("mon") {
        let month = mon.as_str().parse::<Month>().ok()?.number_from_month();
        let day = num("day")?;
        let date = match num("year") {
            Some(year) => NaiveDate::from_ymd_opt(year as i32, month, day)?,
            // The yearless date nearest today: 2 Jan read on 31 Dec is next
            // year's, an old 30 Dec line read on 1 Jan last year's.
            None => (-1..=1)
                .filter_map(|n| NaiveDate::from_ymd_opt(now.year() + n, month, day))
                .min_by_key(|date| (*date - today).num_days().abs())?,
        };
        return local(date).map(|r| (r, true));
    }
    if let Some(wd) = caps.name("wd") {
        let want = wd.as_str().parse::<Weekday>().ok()?;
        return (0..8)
            .map(|n| today + Duration::days(n))
            .filter(|d| d.weekday() == want)
            .filter_map(local)
            .find(|reset| *reset > now)
            .map(|r| (r, false));
    }
    [today, today + Duration::days(1)]
        .into_iter()
        .filter_map(local)
        .find(|reset| *reset > now)
        .map(|r| (r, false))
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
    /// When `app`'s usage limit resets, while it holds.
    fn limited_until(&self, app: &str) -> Option<DateTime<Local>> {
        let reset = *self.state.lock().unwrap().limits.get(app)?;
        holds(reset, (self.cfg.clock)()).then_some(reset)
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
    /// for the App its session runs on. A time or a weekday alone is its
    /// next occurrence, so the line of a limit already reset reads a day or
    /// a week on: one at the time of day of the session's last reset, and
    /// later, is that old line, no limit. A dated line is never that line.
    pub(super) fn limit_shown(&self, ts: &TicketState, st: &Stage, tail: &str) -> Option<Limit> {
        let session = ts.sessions.get(st.name)?;
        let limit = find(app(&session.app)?, tail, (self.cfg.clock)())?;
        let old = !limit.dated
            && session
                .reset
                .is_some_and(|last| limit.reset > last && limit.reset.time() == last.time());
        (!old).then_some(limit)
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
            ..
        } = limit;
        self.change_state(|state| {
            let saved = state.limits.entry(app.to_string()).or_insert(reset);
            *saved = (*saved).max(reset);
        });
        self.update(ticket, |ts| {
            if let Some(session) = ts.sessions.get_mut(st.name) {
                session.reset = Some(reset);
            }
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

    /// Whether a long usage limit ended the run.
    pub(crate) fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// A Ticket leaving a run a long usage limit ended closes its tab; its
    /// session ids stay saved, so /continue resumes each Stage by its id.
    pub(super) fn close_on_limit(&self, ticket: &str) {
        let tab = self.ticket(ticket).tab;
        if !self.closed() || tab.is_empty() {
            return;
        }
        let _ = self.herdr(&["tab", "close", &tab]);
        self.update(ticket, |ts| {
            ts.tab.clear();
            ts.panes.clear();
        });
    }
}
