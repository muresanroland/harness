STATUS: failed

The Ticket's acceptance cases contradict each other: case 2 wants Count("  ") == 0 and
case 5 wants Count("  ") == 1 ("a blank string is one empty word"). No implementation
passes both. Tried strings.Fields (fails case 5) and strings.Split on " " (fails cases 2
and 4). Nothing committed. Someone has to pick which case is right.
