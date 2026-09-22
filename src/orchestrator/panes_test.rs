use super::herdr::{split_target, PaneRect, Rect};
use super::state::STATUS_PR_OPEN;
use super::world::{new_world, BdTicket};
use super::write_file;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

fn rects(sizes: &[(usize, usize)]) -> Vec<PaneRect> {
    sizes
        .iter()
        .enumerate()
        .map(|(i, &(width, height))| PaneRect {
            pane_id: ((b'a' + i as u8) as char).to_string(),
            rect: Rect { width, height },
        })
        .collect()
}

#[test]
fn a_new_stage_pane_comes_out_of_the_roomiest_pane_along_its_longer_side() {
    // (width, height) in cells; a cell is twice as tall as it is wide, so
    // 200x100 is a square on screen and 100x100 is a tall sliver.
    let cases: [(&str, Vec<PaneRect>, &str, &str); 5] = [
        (
            "a wide pane is cut down the middle",
            rects(&[(400, 100)]),
            "a",
            "right",
        ),
        (
            "a tall pane is cut across",
            rects(&[(100, 100)]),
            "a",
            "down",
        ),
        (
            "a square pane is cut across",
            rects(&[(200, 100)]),
            "a",
            "down",
        ),
        (
            "the roomiest pane is the one cut",
            rects(&[(100, 50), (400, 100), (100, 100)]),
            "b",
            "right",
        ),
        ("no layout, no answer", Vec::new(), "", ""),
    ];
    for (name, panes, pane, direction) in cases {
        assert_eq!(
            split_target(&panes),
            (pane.to_string(), direction.to_string()),
            "{name}"
        );
    }
}

#[test]
fn a_stage_pane_is_split_the_way_the_tab_is_shaped() {
    for (name, direction, rect) in [
        ("a tall window is cut across", "down", (100, 400)),
        ("a wide window is cut down the middle", "right", (400, 50)),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        w.lock().rect = rect;
        o.run_ticket("hx-1");

        let split = w.called("herdr pane split");
        assert!(!split.is_empty(), "{name}: no pane was split");
        assert!(
            split[0].contains(&format!("--direction {direction}")),
            "{name}: a {rect:?} pane was cut the other way: {:?}",
            split[0]
        );
    }
}

#[test]
fn a_pane_that_has_not_got_its_shell_yet_is_waited_for() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    // herdr refuses until the pane it just created has a shell.
    w.fail_once(
        "herdr agent start",
        r#"{"error":{"code":"agent_pane_busy","message":"agent target w1:p2 is not an available shell"}}"#,
    );

    o.run_ticket("hx-1");

    let got = o.ticket("hx-1");
    assert_eq!(
        got.status, STATUS_PR_OPEN,
        "a pane that was a moment from ready ended the Stage: {got:?}"
    );
}

#[test]
fn a_session_still_picking_up_its_prompt_is_not_a_finished_stage() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.tick = Duration::from_millis(10); // a 30ms grace for the session to get going
    w.session(|p| {
        // Reads idle at once, as a real session does, and writes its result
        // a moment later.
        let file = PathBuf::from(&p.file);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(5));
            write_file(&file, "STATUS: done\nPR: https://example.test/pr\n");
        });
        (String::new(), "idle".to_string())
    });

    o.run_ticket("hx-1");

    for line in w.main_lines() {
        assert!(
            !line.contains("WAKE"),
            "a session that was still starting was woken on: {line:?}"
        );
    }
    let got = o.ticket("hx-1");
    assert_eq!(
        got.status, STATUS_PR_OPEN,
        "the Pipeline did not finish: {got:?}"
    );
}
