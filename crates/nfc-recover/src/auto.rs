use crate::AttackKind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrngProfile {
    #[default]
    Unknown,
    Weak,
    Hard,
    StaticNonce,
    StaticEncrypted,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardProfile {
    pub prng: PrngProfile,
    pub has_known_key: bool,
    pub darkside_vulnerable: bool,
    pub ev1_signature: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryStrategy {
    Auto,
    Dictionary,
    DarkSide,
    Nested,
    StaticNested,
    HardNested,
    Fm11rf08s,
}

/// Select the attack order without touching a reader. The actual execution
/// and PM3-compatible sequencing land in phases 06-07.
pub fn plan(profile: &CardProfile) -> Vec<RecoveryStrategy> {
    let mut plan = vec![RecoveryStrategy::Dictionary];
    match profile.prng {
        PrngProfile::Unknown => {}
        PrngProfile::Weak if !profile.has_known_key && profile.darkside_vulnerable => {
            plan.push(RecoveryStrategy::DarkSide);
            plan.push(RecoveryStrategy::Nested);
            plan.push(RecoveryStrategy::HardNested);
        }
        PrngProfile::Weak => {
            plan.push(RecoveryStrategy::Nested);
            plan.push(RecoveryStrategy::HardNested);
        }
        PrngProfile::Hard => {
            if profile.ev1_signature || profile.has_known_key {
                plan.push(RecoveryStrategy::HardNested);
            }
        }
        PrngProfile::StaticNonce => plan.push(RecoveryStrategy::StaticNested),
        PrngProfile::StaticEncrypted => plan.push(RecoveryStrategy::Fm11rf08s),
    }
    plan
}

pub fn attack_kind(strategy: RecoveryStrategy) -> Option<AttackKind> {
    match strategy {
        RecoveryStrategy::Auto | RecoveryStrategy::Dictionary => None,
        RecoveryStrategy::DarkSide => Some(AttackKind::DarkSide),
        RecoveryStrategy::Nested => Some(AttackKind::Nested),
        RecoveryStrategy::StaticNested => Some(AttackKind::StaticNested),
        RecoveryStrategy::HardNested => Some(AttackKind::HardNested),
        RecoveryStrategy::Fm11rf08s => Some(AttackKind::Fm11rf08s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_prng_without_key_starts_with_darkside() {
        let plan = plan(&CardProfile {
            prng: PrngProfile::Weak,
            darkside_vulnerable: true,
            ..CardProfile::default()
        });
        assert_eq!(
            plan,
            vec![
                RecoveryStrategy::Dictionary,
                RecoveryStrategy::DarkSide,
                RecoveryStrategy::Nested,
                RecoveryStrategy::HardNested
            ]
        );
    }

    #[test]
    fn static_profiles_use_their_specialized_path() {
        assert_eq!(
            plan(&CardProfile {
                prng: PrngProfile::StaticNonce,
                ..CardProfile::default()
            }),
            vec![RecoveryStrategy::Dictionary, RecoveryStrategy::StaticNested]
        );
        assert_eq!(
            plan(&CardProfile {
                prng: PrngProfile::StaticEncrypted,
                ..CardProfile::default()
            }),
            vec![RecoveryStrategy::Dictionary, RecoveryStrategy::Fm11rf08s]
        );
    }
}
