use super::stage::{result_name, Stage, DEBATE, FIX};
use super::state::STATUS_PR_OPEN;
use super::world::{new_world, spawn_single, succeed, BdTicket};
use super::write_file;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

struct Case {
    name: &'static str,
    stage: &'static Stage,
    review: &'static str,
    verdict: &'static str,
    invalid: &'static str,
    accepted: &'static str,
    reason: &'static str,
    pr: &'static str,
}

/// Drive the Pipeline so these cases also verify that Review counts and the
/// final-Fix requirement reach every completion path, and accepted content is
/// carried back to the next Stage and Ticket state.
#[test]
fn result_acceptance_across_completion_paths() {
    const FIX_ITEM: &str = "- [fix] (high) a.go:1 — fix me";
    const SKIP_ITEM: &str = "- [skip] (low) b.go:2 — leave me";
    let cases = [
        Case {
            name: "Verdict",
            stage: &DEBATE,
            review: "STATUS: done\n- (high) a.go:1 — fix me\n- (low) b.go:2 — leave me\n",
            verdict: "",
            invalid: "STATUS: done\n- [fix] (high) a.go:1 — fix me\n",
            accepted:
                "STATUS: done\n- [fix] (high) a.go:1 — fix me\n- [skip] (low) b.go:2 — leave me\n",
            reason: "Verdict settles 1 of the Review's 2 Findings",
            pr: "https://example.test/pr/hx-1",
        },
        Case {
            name: "final Fix",
            stage: &FIX,
            review: "STATUS: done\n",
            verdict: "STATUS: done\n",
            invalid: "STATUS: done\n",
            accepted: "STATUS: done\nPR: https://example.test/pr/accepted\n",
            reason: "wrote a done result without a 'PR:' line",
            pr: "https://example.test/pr/accepted",
        },
    ];
    for c in &cases {
        for path in ["live", "resume invalid", "resume accepted", "late"] {
            let name = format!("{}/{path}", c.name);
            let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
            let dir = o.run_dir("hx-1");
            write_file(&dir.join("implement.md"), "STATUS: done\n");
            write_file(&dir.join("review-1.md"), c.review);
            if !c.verdict.is_empty() {
                write_file(&dir.join("verdict-1.md"), c.verdict);
            }
            let file = dir.join(result_name(c.stage, 1));
            match path {
                "resume invalid" => write_file(&file, c.invalid),
                "resume accepted" => write_file(&file, c.accepted),
                _ => {}
            }
            let attempts = Arc::new(Mutex::new(0));
            let first_fix = Arc::new(Mutex::new(String::new()));
            {
                let (attempts, first_fix) = (attempts.clone(), first_fix.clone());
                let file = file.display().to_string();
                let (invalid, accepted) = (c.invalid, c.accepted);
                w.session(move |p| {
                    if p.stage == "fix" && p.round == 1 {
                        *first_fix.lock().unwrap() = p.text.clone();
                    }
                    if p.file != file {
                        return succeed(p);
                    }
                    let mut attempts = attempts.lock().unwrap();
                    *attempts += 1;
                    if path == "live" && *attempts == 1 {
                        return (invalid.to_string(), "idle".to_string());
                    }
                    if path == "late" {
                        return (String::new(), "idle".to_string());
                    }
                    (accepted.to_string(), "idle".to_string())
                });
            }

            let o = Arc::new(o);
            let mut run = spawn_single(o.clone(), "hx-1");
            let mut want_attempts = 1;
            match path {
                "live" => {
                    w.await_line(&format!("WAKE hx-1 {} {}", c.stage.name, c.reason));
                    w.control("retry-hx-1");
                    want_attempts = 2;
                }
                "resume accepted" => want_attempts = 0,
                "late" => {
                    w.await_line(&format!(
                        "WAKE hx-1 {} went idle without a done result",
                        c.stage.name
                    ));
                    write_file(&file, c.invalid);
                    thread::sleep(Duration::from_millis(20));
                    assert!(
                        !run.finished(),
                        "{name}: an invalid late result advanced the Pipeline"
                    );
                    write_file(&file, c.accepted);
                }
                _ => {}
            }
            run.wait();
            let ts = o.ticket("hx-1");
            assert!(
                ts.status == STATUS_PR_OPEN && ts.pr == c.pr,
                "{name}: Ticket state = {ts:?}, want PR open at {}",
                c.pr
            );
            assert_eq!(*attempts.lock().unwrap(), want_attempts, "{name}: sessions");
            let first_fix = first_fix.lock().unwrap();
            if c.stage.name == "debate" {
                assert!(
                    first_fix.contains(FIX_ITEM) && !first_fix.contains(SKIP_ITEM),
                    "{name}: Fix did not receive only accepted fix items:\n{first_fix}"
                );
            }
            assert!(
                w.called("herdr agent start h-hx-1-implement").is_empty(),
                "{name}: reran completed Implement Stage"
            );
        }
    }
}
