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
    quality: 48,
    max_width: 1024,
    max_height: 640,
    every_nth_frame: 4,
};

pub const PROFILE_LOW: QualityProfile = QualityProfile {
    name: "low",
    quality: 58,
    max_width: 1152,
    max_height: 720,
    every_nth_frame: 3,
};

pub const PROFILE_MEDIUM: QualityProfile = QualityProfile {
    name: "medium",
    quality: 70,
    max_width: 1280,
    max_height: 800,
    every_nth_frame: 2,
};

pub const PROFILE_HIGH: QualityProfile = QualityProfile {
    name: "high",
    quality: 84,
    max_width: 1440,
    max_height: 920,
    every_nth_frame: 1,
};

pub fn profile_by_name(name: &str) -> &'static QualityProfile {
    match name {
        "ultra" => &PROFILE_ULTRA,
        "low" => &PROFILE_LOW,
        "medium" => &PROFILE_MEDIUM,
        "high" => &PROFILE_HIGH,
        _ => &PROFILE_MEDIUM,
    }
}

pub fn resolve_profile(preset: &str, rtt_ms: f64) -> &'static QualityProfile {
    if preset != "auto" {
        return profile_by_name(preset);
    }
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

#[derive(Default)]
pub struct AdaptiveProfile {
    smoothed_rtt_ms: Option<f64>,
    current: Option<&'static QualityProfile>,
    slow_samples: u8,
    fast_samples: u8,
}

impl AdaptiveProfile {
    pub fn profile(&self) -> &'static QualityProfile {
        self.current.unwrap_or(&PROFILE_HIGH)
    }

    pub fn observe(&mut self, rtt_ms: f64) -> &'static QualityProfile {
        const RTT_ALPHA: f64 = 0.25;
        if !rtt_ms.is_finite() || rtt_ms < 0.0 {
            return self.profile();
        }
        let smoothed = self.smoothed_rtt_ms.map_or(rtt_ms, |previous| {
            previous * (1.0 - RTT_ALPHA) + rtt_ms * RTT_ALPHA
        });
        self.smoothed_rtt_ms = Some(smoothed);
        let candidate = resolve_profile("auto", smoothed);
        let Some(current) = self.current else {
            self.current = Some(candidate);
            return candidate;
        };

        // Require fresh samples to agree with the EWMA's direction. A single
        // spike can keep the average high even after the network has recovered.
        let raw = resolve_profile("auto", rtt_ms);
        self.slow_samples = if candidate.quality < current.quality && raw.quality < current.quality
        {
            self.slow_samples + 1
        } else {
            0
        };
        self.fast_samples = if candidate.quality > current.quality && raw.quality > current.quality
        {
            self.fast_samples + 1
        } else {
            0
        };
        if self.slow_samples == 2 || self.fast_samples == 3 {
            self.current = Some(candidate);
            self.slow_samples = 0;
            self.fast_samples = 0;
        }
        self.profile()
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
    fn preserves_legible_motion_profiles() {
        for (preset, expected) in [
            ("ultra", (48, 1024, 640, 4)),
            ("low", (58, 1152, 720, 3)),
            ("medium", (70, 1280, 800, 2)),
            ("high", (84, 1440, 920, 1)),
        ] {
            let p = resolve_profile(preset, 0.0);
            assert_eq!(
                (p.quality, p.max_width, p.max_height, p.every_nth_frame),
                expected
            );
        }
    }

    #[test]
    fn smooths_rtt_and_requires_two_slow_or_three_fast_samples() {
        let mut adaptive = AdaptiveProfile::default();
        assert_eq!(adaptive.observe(120.0), &PROFILE_HIGH);
        assert_eq!(adaptive.observe(240.0), &PROFILE_HIGH);
        assert_eq!(adaptive.smoothed_rtt_ms, Some(150.0));
        assert_eq!(adaptive.observe(120.0), &PROFILE_HIGH);
        assert_eq!(adaptive.smoothed_rtt_ms, Some(142.5));
        assert_eq!(adaptive.observe(180.0), &PROFILE_HIGH);
        assert_eq!(adaptive.observe(180.0), &PROFILE_MEDIUM);
        assert_eq!(adaptive.observe(0.0), &PROFILE_MEDIUM);
        assert_eq!(adaptive.observe(0.0), &PROFILE_MEDIUM);
        assert_eq!(adaptive.observe(0.0), &PROFILE_HIGH);
    }

    #[test]
    fn adaptive_profiles_and_stale_frame_filtering() {
        assert_eq!(resolve_profile("auto", 900.0).name, "ultra");
        assert_eq!(resolve_profile("auto", 900.0).every_nth_frame, 4);
        assert_eq!(resolve_profile("auto", 900.0).max_width, 1024);
        assert_eq!(resolve_profile("auto", 420.0).name, "low");
        assert_eq!(resolve_profile("auto", 420.0).every_nth_frame, 3);
        assert_eq!(resolve_profile("auto", 220.0).name, "medium");
        assert_eq!(resolve_profile("auto", 60.0).name, "high");
        assert_eq!(resolve_profile("high", 900.0).name, "high");
        assert_eq!(resolve_profile("unknown", 0.0).name, "medium");

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
        assert!(!should_apply_frame(
            2,
            3,
            Some("t2"),
            Some("t1"),
            Some(&tabs),
            false
        ));
        // Mismatched target while current tab is still live is dropped
        assert!(!should_apply_frame(
            3,
            3,
            Some("t2"),
            Some("t1"),
            Some(&tabs),
            false
        ));
        // Legitimate external close recovery is accepted
        let only_t1 = vec![tabs[0].clone()];
        assert!(should_apply_frame(
            3,
            3,
            Some("t2"),
            Some("t1"),
            Some(&only_t1),
            false
        ));
        assert!(should_apply_frame(3, 3, Some("t2"), Some("t1"), None, true));
    }
}
