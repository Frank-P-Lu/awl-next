//! Exhaustive law for the theme-preview present-transaction bracket.
//!
//! The live app deliberately keeps the two small pieces of storage that fit its
//! event owners: a debounce stamp and a post-settle teardown bit. This test-only
//! vocabulary names their four reachable combinations and pins every event over
//! every phase. Both `match`es are wildcard-free: adding a phase or event makes
//! the suite stop compiling until the ordering law is extended deliberately.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CrossingPhase {
    Idle,
    Debouncing,
    AwaitingPresent,
    DebouncingAwaitingPresent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CrossingEvent {
    PreviewStep,
    SettleElapsed,
    PresentSkipped,
    PresentSucceeded,
    Suspend,
}

impl CrossingPhase {
    pub(super) const ALL: [Self; 4] = [
        Self::Idle,
        Self::Debouncing,
        Self::AwaitingPresent,
        Self::DebouncingAwaitingPresent,
    ];

    pub(super) const fn from_claims(debouncing: bool, awaiting_present: bool) -> Self {
        match (debouncing, awaiting_present) {
            (false, false) => Self::Idle,
            (true, false) => Self::Debouncing,
            (false, true) => Self::AwaitingPresent,
            (true, true) => Self::DebouncingAwaitingPresent,
        }
    }

    pub(super) const fn claims_present_sync(self) -> bool {
        match self {
            Self::Idle => false,
            Self::Debouncing => true,
            Self::AwaitingPresent => true,
            Self::DebouncingAwaitingPresent => true,
        }
    }
}

/// Advance the pure ordering model by one event.
///
/// `SettleElapsed` outside a live debounce and `PresentSucceeded` outside a
/// pending teardown are explicit identities: the real event owners gate those
/// calls, but naming the invalid/stale cases keeps the law total. A skipped
/// present never consumes the teardown claim. `Suspend` models the explicit
/// live-state reset during GPU/window teardown.
pub(super) const fn transition(phase: CrossingPhase, event: CrossingEvent) -> CrossingPhase {
    use CrossingEvent::{PresentSkipped, PresentSucceeded, PreviewStep, SettleElapsed, Suspend};
    use CrossingPhase::{AwaitingPresent, Debouncing, DebouncingAwaitingPresent, Idle};

    match (phase, event) {
        (Idle, PreviewStep) => Debouncing,
        (Debouncing, PreviewStep) => Debouncing,
        (AwaitingPresent, PreviewStep) => DebouncingAwaitingPresent,
        (DebouncingAwaitingPresent, PreviewStep) => DebouncingAwaitingPresent,

        (Idle, SettleElapsed) => Idle,
        (Debouncing, SettleElapsed) => AwaitingPresent,
        (AwaitingPresent, SettleElapsed) => AwaitingPresent,
        (DebouncingAwaitingPresent, SettleElapsed) => AwaitingPresent,

        (Idle, PresentSkipped) => Idle,
        (Debouncing, PresentSkipped) => Debouncing,
        (AwaitingPresent, PresentSkipped) => AwaitingPresent,
        (DebouncingAwaitingPresent, PresentSkipped) => DebouncingAwaitingPresent,

        (Idle, PresentSucceeded) => Idle,
        (Debouncing, PresentSucceeded) => Debouncing,
        (AwaitingPresent, PresentSucceeded) => Idle,
        (DebouncingAwaitingPresent, PresentSucceeded) => Debouncing,

        (Idle, Suspend) => Idle,
        (Debouncing, Suspend) => Idle,
        (AwaitingPresent, Suspend) => Idle,
        (DebouncingAwaitingPresent, Suspend) => Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_two_bit_crossing_phases_have_the_exact_sync_claim() {
        for debouncing in [false, true] {
            for awaiting in [false, true] {
                let phase = CrossingPhase::from_claims(debouncing, awaiting);
                assert_eq!(
                    phase.claims_present_sync(),
                    debouncing || awaiting,
                    "phase {phase:?} must compose exactly like the live two-bit OR"
                );
            }
        }
    }

    #[test]
    fn skipped_present_holds_the_bracket_until_a_successful_present() {
        let settled = transition(
            transition(CrossingPhase::Idle, CrossingEvent::PreviewStep),
            CrossingEvent::SettleElapsed,
        );
        assert_eq!(settled, CrossingPhase::AwaitingPresent);
        assert!(settled.claims_present_sync());

        let skipped = transition(settled, CrossingEvent::PresentSkipped);
        assert_eq!(skipped, CrossingPhase::AwaitingPresent);
        assert!(skipped.claims_present_sync());

        let presented = transition(skipped, CrossingEvent::PresentSucceeded);
        assert_eq!(presented, CrossingPhase::Idle);
        assert!(!presented.claims_present_sync());
    }

    #[test]
    fn rearm_while_awaiting_present_preserves_both_ordering_claims() {
        let awaiting = transition(
            transition(CrossingPhase::Idle, CrossingEvent::PreviewStep),
            CrossingEvent::SettleElapsed,
        );
        let both = transition(awaiting, CrossingEvent::PreviewStep);
        assert_eq!(both, CrossingPhase::DebouncingAwaitingPresent);

        // The old reshape presents, but the newer preview still owns the
        // debounce claim; it must not be disarmed by that successful present.
        let after_old_present = transition(both, CrossingEvent::PresentSucceeded);
        assert_eq!(after_old_present, CrossingPhase::Debouncing);
        assert!(after_old_present.claims_present_sync());

        let awaiting_new = transition(after_old_present, CrossingEvent::SettleElapsed);
        assert_eq!(awaiting_new, CrossingPhase::AwaitingPresent);
        assert_eq!(
            transition(awaiting_new, CrossingEvent::PresentSucceeded),
            CrossingPhase::Idle
        );
    }

    #[test]
    fn every_phase_has_explicit_stale_event_and_suspend_behavior() {
        for phase in CrossingPhase::ALL {
            assert_eq!(transition(phase, CrossingEvent::PresentSkipped), phase);
            assert_eq!(
                transition(phase, CrossingEvent::Suspend),
                CrossingPhase::Idle
            );
        }
        assert_eq!(
            transition(CrossingPhase::Idle, CrossingEvent::SettleElapsed),
            CrossingPhase::Idle
        );
        assert_eq!(
            transition(CrossingPhase::Debouncing, CrossingEvent::PresentSucceeded),
            CrossingPhase::Debouncing
        );
    }
}
