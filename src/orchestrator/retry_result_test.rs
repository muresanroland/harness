use super::state::STATUS_PARKED;
use super::world::{new_world, spawn_single, succeed, BdTicket, Prompt};
use super::write_file;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// An old session can finish while its pane is being closed. That result
/// belongs to the old attempt, even if the replacement session goes idle.
#[test]
fn retry_discards_a_result_written_while_closing_the_old_pane() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let old: Arc<Mutex<Prompt>> = Default::default();
    let attempts = Arc::new(Mutex::new(0));
    {
        let (old, attempts) = (old.clone(), attempts.clone());
        w.session(move |p| {
            if p.stage != "implement" {
                return succeed(p);
            }
            let mut attempts = attempts.lock().unwrap();
            *attempts += 1;
            if *attempts == 1 {
                *old.lock().unwrap() = p.clone();
                return ("STATUS: failed\n".to_string(), "idle".to_string());
            }
            (String::new(), "idle".to_string()) // the replacement never produces its own result
        });
    }
    {
        let old = old.clone();
        w.hook(move |_, argv| {
            let old = old.lock().unwrap();
            if argv.join(" ") == format!("herdr pane close {}", old.pane) {
                write_file(Path::new(&old.file), "STATUS: done\n");
            }
            None
        });
    }
    let o = Arc::new(o);
    let mut run = spawn_single(o.clone(), "hx-1");
    w.await_line("hx-1 stuck in implement: session reported failure");
    w.control("retry-hx-1");
    run.wait();
    let ts = o.ticket("hx-1");
    assert!(
        ts.status == STATUS_PARKED && ts.reason.contains("without a result"),
        "a result from the old attempt completed its replacement: {ts:?}"
    );
    assert_eq!(*attempts.lock().unwrap(), 2, "Implement attempts");
    // The Go suite asserts the replacement on resume (scheduler_test); the
    // retry is the same fresh_pane path, so it is asserted here as well.
    let old_pane = old.lock().unwrap().pane.clone();
    assert_eq!(
        w.called(&format!("herdr pane close {old_pane}")).len(),
        1,
        "the retry did not replace the old pane"
    );
    assert!(
        w.called("herdr agent start h-hx-1-review").is_empty(),
        "Review started even though the replacement wrote no result"
    );
}
