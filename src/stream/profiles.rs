use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TabInfo {
    pub target_id: String,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualityProfile {
    pub name: &'static str,
    pub quality: u8,
    pub max_width: u32,
    pub max_height: u32,
    pub every_nth_frame: u8,
}

pub const PROFILE_ULTRA: QualityProfile = QualityProfile {
    name: "ultra",
    quality: 32,
    max_width: 800,
    max_height: 520,
    every_nth_frame: 3,
};

pub const PROFILE_LOW: QualityProfile = QualityProfile {
    name: "low",
    quality: 38,
    max_width: 960,
    max_height: 600,
    every_nth_frame: 2,
};

pub const PROFILE_MEDIUM: QualityProfile = QualityProfile {
    name: "medium",
    quality: 52,
    max_width: 1152,
    max_height: 740,
    every_nth_frame: 2,
};

pub const PROFILE_HIGH: QualityProfile = QualityProfile {
    name: "high",
    quality: 74,
    max_width: 1440,
    max_height: 920,
    every_nth_frame: 1,
};

pub fn resolve_profile(preset: &str, rtt_ms: f64) -> &'static QualityProfile {
    match preset {
        "ultra" => &PROFILE_ULTRA,
        "low" => &PROFILE_LOW,
        "medium" => &PROFILE_MEDIUM,
        "high" => &PROFILE_HIGH,
        "auto" => {
            if rtt_ms > 750.0 {
                &PROFILE_ULTRA
            } else if rtt_ms > 350.0 {
                &PROFILE_LOW
            } else if rtt_ms > 140.0 {
                &PROFILE_MEDIUM
            } else {
                &PROFILE_HIGH
            }
        }
        _ => &PROFILE_MEDIUM,
    }
}

/// Validates whether a frame response matches the client's current tab epoch & targetId,
/// dropping stale frames raced across tab switches while accepting legitimate server recovery.
#[cfg(test)]
pub fn should_apply_frame(
    req_epoch: u64,
    current_epoch: u64,
    current_target_id: Option<&str>,
    response_target_id: Option<&str>,
    response_tabs: Option<&[TabInfo]>,
    recovered: bool,
) -> bool {
    if req_epoch != current_epoch {
        return false;
    }
    if let (Some(cur), Some(resp)) = (current_target_id, response_target_id) {
        if cur != resp {
            let server_dropped_cur = recovered
                || response_tabs
                    .map(|tabs| !tabs.iter().any(|t| t.target_id == cur))
                    .unwrap_or(false);
            if !server_dropped_cur {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_profile_selection_covers_all_rtt_bands() {
        assert_eq!(resolve_profile("auto", 900.0).name, "ultra");
        assert_eq!(resolve_profile("auto", 900.0).every_nth_frame, 3);
        assert_eq!(resolve_profile("auto", 900.0).max_width, 800);
        assert_eq!(resolve_profile("auto", 420.0).name, "low");
        assert_eq!(resolve_profile("auto", 420.0).every_nth_frame, 2);
        assert_eq!(resolve_profile("auto", 220.0).name, "medium");
        assert_eq!(resolve_profile("auto", 60.0).name, "high");
        assert_eq!(resolve_profile("high", 900.0).name, "high");
        assert_eq!(resolve_profile("unknown", 0.0).name, "medium");
    }

    #[test]
    fn stale_epoch_and_target_filtering_works() {
        let tabs = vec![
            TabInfo {
                target_id: "t1".into(),
                title: "One".into(),
                url: "https://one.test".into(),
            },
            TabInfo {
                target_id: "t2".into(),
                title: "Two".into(),
                url: "https://two.test".into(),
            },
        ];
        // Stale epoch is always dropped
        assert!(!should_apply_frame(2, 3, Some("t2"), Some("t1"), Some(&tabs), false));
        // Mismatched target while current tab is still live is dropped
        assert!(!should_apply_frame(3, 3, Some("t2"), Some("t1"), Some(&tabs), false));
        // Legitimate external close recovery is accepted
        let only_t1 = vec![tabs[0].clone()];
        assert!(should_apply_frame(3, 3, Some("t2"), Some("t1"), Some(&only_t1), false));
        assert!(should_apply_frame(3, 3, Some("t2"), Some("t1"), None, true));
    }
}
