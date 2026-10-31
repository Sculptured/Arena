//! Instance and save ids arenad chooses (`^[a-z0-9_]{1,63}$`): instances `b_<branch>_<part>`,
//! saves `s_<uuid>`, base saves `base_<bench>_<version>_<part>`, gold saves `gold_<bench>_<ver>_<task>_<part>`.

use parts::proto::{valid_id, BenchDescribe};

use super::Saves;

/// FNV-1a, for short stable ids.
fn fnv64(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

/// `<prefix>_<middle>_<part>`, or with the part replaced by a hash when that is not a valid id
/// (too long, or the part name has other characters).
fn id_for(prefix: &str, middle: &str, part: &str) -> String {
    let id = format!("{prefix}_{middle}_{part}");
    if valid_id(&id) {
        return id;
    }
    let id = format!("{prefix}_{middle}_{:016x}", fnv64(part));
    if valid_id(&id) {
        id
    } else {
        format!("{prefix}_{:016x}", fnv64(&format!("{middle}/{part}")))
    }
}

/// The instance of `part` in `branch`'s world.
pub fn instance_id(branch: &str, part: &str) -> String {
    id_for("b", branch, part)
}

/// A fresh save id.
pub fn new_save_id() -> String {
    format!("s_{}", uuid::Uuid::new_v4().simple())
}

/// A branch id for a short-lived world (making a base or gold save).
pub fn temp_branch_id() -> String {
    format!("t{}", uuid::Uuid::new_v4().simple())
}

/// `<bench>_<version>`, with `.` and `-` turned into `_`.
fn bench_middle(bench: &BenchDescribe) -> String {
    format!("{}_{}", bench.name, bench.version).replace(['.', '-'], "_")
}

/// The base save of each part of `bench`'s world.
pub fn base_names(bench: &BenchDescribe) -> Saves {
    names("base", &bench_middle(bench), bench)
}

/// The gold save of each part for `task`. A task id or part that doesn't fit an id is replaced
/// by a hash.
pub fn gold_names(bench: &BenchDescribe, task: &str) -> Saves {
    names("gold", &format!("{}_{task}", bench_middle(bench)), bench)
}

fn names(prefix: &str, middle: &str, bench: &BenchDescribe) -> Saves {
    let id = |p: &str| id_for(prefix, middle, p);
    bench
        .world
        .iter()
        .map(|p| (p.part.clone(), id(&p.part)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use parts::proto::WorldPart;
    use serde_json::json;

    fn bench(version: &str) -> BenchDescribe {
        BenchDescribe {
            name: "retail".into(),
            version: version.into(),
            protocol: 1,
            world: vec![WorldPart {
                part: "retail".into(),
                integration: "postgres".into(),
                config: json!({}),
            }],
            tasks: vec![],
            instructions: String::new(),
            tools: vec![],
        }
    }

    #[test]
    fn instance_ids_fit_the_protocol() {
        let b = "0123456789abcdef0123456789abcdef";
        assert_eq!(instance_id(b, "retail"), format!("b_{b}_retail"));
        let long = instance_id(b, "a_very_long_part_name_that_does_not_fit_here");
        assert!(
            valid_id(&long) && long.starts_with(&format!("b_{b}_")),
            "{long}"
        );
        assert_ne!(
            long,
            instance_id(b, "a_very_long_part_name_that_does_not_fit_herf")
        );
        assert!(valid_id(&instance_id(b, "ridgeline-plant")));
        assert!(valid_id(&instance_id(&temp_branch_id(), "retail")));
    }

    #[test]
    fn save_ids_fit_the_protocol() {
        assert_eq!(
            base_names(&bench("0.1.0-rc.1"))["retail"],
            "base_retail_0_1_0_rc_1_retail"
        );
        let gold = |task: &str| gold_names(&bench("0.1.0"), task)["retail"].clone();
        assert_eq!(gold("27"), "gold_retail_0_1_0_27_retail");
        // Other task ids are hashed, deterministically and apart from each other.
        assert!(valid_id(&gold("../x")) && gold("../x") == gold("../x"));
        assert_ne!(gold("../x"), gold("../y"));
        assert!(valid_id(&gold(&"t".repeat(80))));
        let s = new_save_id();
        assert!(valid_id(&s) && s.len() == 34);
    }
}
