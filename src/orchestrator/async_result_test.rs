use super::state::{STATUS_PR_OPEN, STATUS_RUNNING};
use super::world::{new_world, spawn_single, succeed, BdTicket, Prompt};
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A valid file is only half of completion: the agent must finish too.
#[test]
fn a_result_written_by_a_working_agent_does_not_advance_the_stage() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let (prompted_tx, prompted) = sync_channel::<Prompt>(1);
    let (polled_tx, polled) = sync_channel::<()>(1);
    let prompted_tx = Mutex::new(prompted_tx);
    w.session(move |p| {
        if p.stage == "implement" {
            prompted_tx.lock().unwrap().send(p.clone()).unwrap();
            return ("STATUS: done\n".to_string(), "working".to_string());
        }
        succeed(p)
    });
    w.hook(move |_, argv| {
        if argv.join(" ").starts_with("herdr agent wait") {
            let _ = polled_tx.try_send(());
        }
        None
    });
    let o = Arc::new(o);
    let mut run = spawn_single(o.clone(), "hx-1");
    let timeout = Duration::from_secs(5);
    let p = prompted
        .recv_timeout(timeout)
        .expect("Implement was never prompted");
    match polled.recv_timeout(timeout) {
        Ok(()) => {}
        Err(RecvTimeoutError::Timeout) => panic!("orchestrator never waited for the working agent"),
        Err(err) => panic!("{err}"),
    }
    assert!(
        !run.finished(),
        "a working agent's result completed the pipeline"
    );
    let got = o.ticket("hx-1");
    assert!(
        got.stage == "implement" && got.status == STATUS_RUNNING,
        "advanced while Implement was still working: {got:?}"
    );
    assert!(
        w.called("herdr agent start h-hx-1-review").is_empty(),
        "Review started before Implement finished"
    );
    w.lock().agents.insert(p.pane.clone(), "idle".to_string());
    assert!(
        run.finished_within(timeout),
        "did not accept the result after the agent finished"
    );
    run.wait();
    let got = o.ticket("hx-1");
    assert_eq!(
        got.status, STATUS_PR_OPEN,
        "pipeline did not finish: {got:?}"
    );
}
