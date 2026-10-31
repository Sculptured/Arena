//! How each part of a world is restored on a rerun, from what its integration declares and the
//! restore asked for (the spec's restore table):
//!
//! | Part declares            | exact                  | approximate                  |
//! |--------------------------|------------------------|------------------------------|
//! | exact save               | from its save          | same                         |
//! | replay and approximate   | replay                 | from its save, approximately |
//! | replay only              | replay                 | replay                       |
//! | approximate only         | refused, naming it     | from its save, approximately |
//! | none                     | refused                | refused                      |

use std::collections::BTreeMap;

use parts::proto::{ExactKind, Mode, Restore};

use crate::model::{RestoredHow, RestoredPart};

/// How one part is restored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// Start from its save, in this mode.
    Save(Mode),
    /// Start fresh and repeat its recorded inputs.
    Replay,
}

impl How {
    pub fn label(self) -> RestoredHow {
        match self {
            How::Save(Mode::Exact) => RestoredHow::Save,
            How::Save(Mode::Approximate) => RestoredHow::Approximate,
            How::Replay => RestoredHow::Replay,
        }
    }
}

/// A part that can't be restored in the mode asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub part: String,
    pub reason: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for Refused {}

/// True when the part keeps saves (exact ones, or approximate ones); a part with only replay
/// (or nothing) is never saved.
pub fn keeps_saves(r: &Restore) -> bool {
    r.exact == ExactKind::Save || r.approximate
}

/// How `part` (declaring `r`) is restored in `mode`.
pub fn how(part: &str, r: &Restore, mode: Mode) -> Result<How, Refused> {
    match (r.exact, r.approximate, mode) {
        (ExactKind::Save, _, _) => Ok(How::Save(Mode::Exact)),
        (ExactKind::Replay, true, Mode::Approximate) => Ok(How::Save(Mode::Approximate)),
        (ExactKind::Replay, _, _) => Ok(How::Replay),
        (ExactKind::None, true, Mode::Approximate) => Ok(How::Save(Mode::Approximate)),
        (ExactKind::None, true, Mode::Exact) => Err(Refused {
            part: part.to_string(),
            reason: format!(
                "the part {part} can't be restored exactly; choose an approximate restore"
            ),
        }),
        (ExactKind::None, false, _) => Err(Refused {
            part: part.to_string(),
            reason: format!("the part {part} can't be restored"),
        }),
    }
}

/// How every part is restored in `mode`, by part name; the first part that can't be is refused.
pub fn restore_plan<'a>(
    describes: impl IntoIterator<Item = (&'a str, &'a Restore)>,
    mode: Mode,
) -> Result<BTreeMap<String, How>, Refused> {
    describes
        .into_iter()
        .map(|(part, r)| how(part, r, mode).map(|h| (part.to_string(), h)))
        .collect()
}

/// The `restored` record of a branch: each part and how it was restored.
pub fn restored(plan: &BTreeMap<String, How>) -> Vec<RestoredPart> {
    plan.iter()
        .map(|(part, h)| RestoredPart {
            part: part.clone(),
            how: h.label(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(exact: ExactKind, approximate: bool) -> Restore {
        Restore { exact, approximate }
    }

    #[test]
    fn the_restore_table() {
        use ExactKind::*;
        use Mode::*;
        let save = Ok(How::Save(Exact));
        let approx = Ok(How::Save(Approximate));
        let cases = [
            (r(Save, false), Exact, save.clone()),
            (r(Save, false), Approximate, save.clone()),
            (r(Save, true), Approximate, save),
            (r(Replay, true), Exact, Ok(How::Replay)),
            (r(Replay, true), Approximate, approx.clone()),
            (r(Replay, false), Exact, Ok(How::Replay)),
            (r(Replay, false), Approximate, Ok(How::Replay)),
            (r(None, true), Approximate, approx),
        ];
        for (decl, mode, want) in cases {
            assert_eq!(how("p", &decl, mode), want, "{decl:?} {mode:?}");
        }
    }

    #[test]
    fn refusals_name_the_part() {
        let e = how("plant", &r(ExactKind::None, true), Mode::Exact).unwrap_err();
        assert_eq!(e.part, "plant");
        assert!(e.to_string().contains("plant") && e.to_string().contains("approximate"));
        for mode in [Mode::Exact, Mode::Approximate] {
            let e = how("isaac", &r(ExactKind::None, false), mode).unwrap_err();
            assert_eq!(e.to_string(), "the part isaac can't be restored");
        }
    }

    #[test]
    fn a_plan_covers_every_part_or_refuses() {
        let (s, a) = (r(ExactKind::Save, false), r(ExactKind::None, true));
        let plan = restore_plan([("db", &s), ("plant", &a)], Mode::Approximate).unwrap();
        assert_eq!(plan["plant"], How::Save(Mode::Approximate));
        assert_eq!(
            restored(&plan),
            [
                RestoredPart {
                    part: "db".into(),
                    how: RestoredHow::Save
                },
                RestoredPart {
                    part: "plant".into(),
                    how: RestoredHow::Approximate
                },
            ]
        );
        let e = restore_plan([("db", &s), ("plant", &a)], Mode::Exact).unwrap_err();
        assert_eq!(e.part, "plant");
    }

    #[test]
    fn which_parts_keep_saves() {
        assert!(keeps_saves(&r(ExactKind::Save, false)));
        assert!(keeps_saves(&r(ExactKind::Replay, true)));
        assert!(keeps_saves(&r(ExactKind::None, true)));
        assert!(!keeps_saves(&r(ExactKind::Replay, false)));
        assert!(!keeps_saves(&r(ExactKind::None, false)));
    }
}
