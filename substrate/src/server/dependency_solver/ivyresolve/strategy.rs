use std::cmp::Ordering;

/// Gradle-shaped port of:
/// - `ivyresolve.strategy.VersionParser`
/// - `ivyresolve.strategy.StaticVersionComparator`
///
/// Keep this module aligned with Gradle's dependency-management package rather
/// than the gRPC service shape. The resolver should call into this strategy
/// layer so future solver slices can be ported package-by-package.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let a_parts = gradle_version_parts(a);
    let b_parts = gradle_version_parts(b);

    for (pa, pb) in a_parts.iter().zip(b_parts.iter()) {
        match (pa.parse::<u64>(), pb.parse::<u64>()) {
            (Ok(na), Ok(nb)) => match na.cmp(&nb) {
                Ordering::Equal => continue,
                other => return other,
            },
            (Ok(_), Err(_)) => return Ordering::Greater,
            (Err(_), Ok(_)) => return Ordering::Less,
            (Err(_), Err(_)) => {
                match (
                    gradle_special_version_part(pa),
                    gradle_special_version_part(pb),
                ) {
                    (Some(a_meaning), Some(b_meaning)) => match a_meaning.cmp(&b_meaning) {
                        Ordering::Equal => continue,
                        other => return other,
                    },
                    (Some(a_meaning), None) => match a_meaning.cmp(&0) {
                        Ordering::Equal => continue,
                        other => return other,
                    },
                    (None, Some(b_meaning)) => match 0.cmp(&b_meaning) {
                        Ordering::Equal => continue,
                        other => return other,
                    },
                    (None, None) => match pa.cmp(pb) {
                        Ordering::Equal => continue,
                        other => return other,
                    },
                }
            }
        }
    }

    if a_parts.len() > b_parts.len() {
        if a_parts[b_parts.len()].parse::<u64>().is_ok() {
            Ordering::Greater
        } else {
            Ordering::Less
        }
    } else if b_parts.len() > a_parts.len() {
        if b_parts[a_parts.len()].parse::<u64>().is_ok() {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    } else {
        Ordering::Equal
    }
}

fn gradle_special_version_part(part: &str) -> Option<i32> {
    match part.to_ascii_lowercase().as_str() {
        "dev" => Some(-1),
        "rc" => Some(1),
        "snapshot" => Some(2),
        "final" => Some(3),
        "ga" => Some(4),
        "release" => Some(5),
        "sp" => Some(6),
        _ => None,
    }
}

fn gradle_version_parts(version: &str) -> Vec<&str> {
    let mut parts = Vec::with_capacity(8);
    let mut start = 0;
    let mut digit = false;

    for (i, ch) in version.char_indices() {
        if matches!(ch, '.' | '_' | '-' | '+') {
            parts.push(&version[start..i]);
            start = i + ch.len_utf8();
            digit = false;
        } else if ch.is_ascii_digit() {
            if !digit && i > start {
                parts.push(&version[start..i]);
                start = i;
            }
            digit = true;
        } else {
            if digit {
                parts.push(&version[start..i]);
                start = i;
            }
            digit = false;
        }
    }
    if start < version.len() {
        parts.push(&version[start..]);
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::compare_versions;
    use std::cmp::Ordering;

    #[test]
    fn compare_versions_matches_gradle_static_version_comparator_subset() {
        assert_eq!(compare_versions("1.0.0", "2.0.0"), Ordering::Less);
        assert_eq!(compare_versions("2.0.0", "1.0.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("1.2.3", "1.2.4"), Ordering::Less);
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0-rc-1", "1.0"), Ordering::Less);
        assert_eq!(
            compare_versions("1.0-snapshot", "1.0-rc-1"),
            Ordering::Greater
        );
        assert_eq!(compare_versions("1.0-ga", "1.0-final"), Ordering::Greater);
        assert_eq!(compare_versions("1.0-sp", "1.0-release"), Ordering::Greater);
        assert_eq!(compare_versions("1.0alpha1", "1.0alpha2"), Ordering::Less);
    }
}
