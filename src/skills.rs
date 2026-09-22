//! The skills the Harness ships: start-work and one Stage skill per Stage,
//! plus create-pr. 'harness init' copies them into a Target repo. A new skill
//! is one more line here.

/// (name, body of skills/<name>/SKILL.md), in the order Go's embed.FS listed them.
pub(crate) const SKILLS: &[(&str, &str)] = &[
    ("create-pr", include_str!("../skills/create-pr/SKILL.md")),
    (
        "stage-address",
        include_str!("../skills/stage-address/SKILL.md"),
    ),
    ("stage-fix", include_str!("../skills/stage-fix/SKILL.md")),
    (
        "stage-implement",
        include_str!("../skills/stage-implement/SKILL.md"),
    ),
    (
        "stage-moderate",
        include_str!("../skills/stage-moderate/SKILL.md"),
    ),
    (
        "stage-review",
        include_str!("../skills/stage-review/SKILL.md"),
    ),
    ("start-work", include_str!("../skills/start-work/SKILL.md")),
];
