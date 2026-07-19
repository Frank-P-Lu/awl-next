//! Exhaustive law for the composed present-transaction lifecycle.
//!
//! `App::sync_present_txn` has three independent claim owners: the resize and
//! move settle streams, plus the preview crossing bracket. This test-only
//! model sweeps their product space through every lifecycle transition. Its
//! matches deliberately have no wildcard arms: a new phase or transition must
//! be classified here before the suite compiles again.

use super::crossing::{self, CrossingEvent, CrossingPhase};

/// A resize or move stream's contribution to the transaction bracket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamPhase {
    Disarmed,
    Armed,
}

impl StreamPhase {
    const ALL: [Self; 2] = [Self::Disarmed, Self::Armed];

    const fn claims_present_sync(self) -> bool {
        match self {
            Self::Disarmed => false,
            Self::Armed => true,
        }
    }

    const fn arm(self) -> Self {
        match self {
            Self::Disarmed => Self::Armed,
            Self::Armed => Self::Armed,
        }
    }

    const fn disarm(self) -> Self {
        match self {
            Self::Disarmed => Self::Disarmed,
            Self::Armed => Self::Disarmed,
        }
    }
}

/// Every event owner that can change a present-transaction claim. `Teardown`
/// is the explicit GPU/window suspend reset; a present only disarms the
/// preview source after its settle has handed off to pending teardown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PresentTxnTransition {
    ResizeArm,
    ResizeSettle,
    MoveArm,
    MoveSettle,
    PreviewArm,
    PreviewSettle,
    PresentSkipped,
    PresentSucceeded,
    Teardown,
}

impl PresentTxnTransition {
    const ALL: [Self; 9] = [
        Self::ResizeArm,
        Self::ResizeSettle,
        Self::MoveArm,
        Self::MoveSettle,
        Self::PreviewArm,
        Self::PreviewSettle,
        Self::PresentSkipped,
        Self::PresentSucceeded,
        Self::Teardown,
    ];
}

/// The full product state read by `App::sync_present_txn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PresentTxnPhase {
    resize: StreamPhase,
    move_: StreamPhase,
    crossing: CrossingPhase,
}

impl PresentTxnPhase {
    const IDLE: Self = Self {
        resize: StreamPhase::Disarmed,
        move_: StreamPhase::Disarmed,
        crossing: CrossingPhase::Idle,
    };

    fn all() -> impl Iterator<Item = Self> {
        StreamPhase::ALL.into_iter().flat_map(|resize| {
            StreamPhase::ALL.into_iter().flat_map(move |move_| {
                CrossingPhase::ALL.into_iter().map(move |crossing| Self {
                    resize,
                    move_,
                    crossing,
                })
            })
        })
    }

    const fn claims(self) -> (bool, bool, bool) {
        (
            self.resize.claims_present_sync(),
            self.move_.claims_present_sync(),
            self.crossing.claims_present_sync(),
        )
    }

    const fn claims_present_sync(self) -> bool {
        let (resize, move_, crossing) = self.claims();
        resize || move_ || crossing
    }
}

/// Advance one owner through one lifecycle transition.
const fn transition(phase: PresentTxnPhase, transition: PresentTxnTransition) -> PresentTxnPhase {
    use PresentTxnTransition::{
        MoveArm, MoveSettle, PresentSkipped, PresentSucceeded, PreviewArm, PreviewSettle,
        ResizeArm, ResizeSettle, Teardown,
    };

    match transition {
        ResizeArm => PresentTxnPhase {
            resize: phase.resize.arm(),
            ..phase
        },
        ResizeSettle => PresentTxnPhase {
            resize: phase.resize.disarm(),
            ..phase
        },
        MoveArm => PresentTxnPhase {
            move_: phase.move_.arm(),
            ..phase
        },
        MoveSettle => PresentTxnPhase {
            move_: phase.move_.disarm(),
            ..phase
        },
        PreviewArm => PresentTxnPhase {
            crossing: crossing::transition(phase.crossing, CrossingEvent::PreviewStep),
            ..phase
        },
        PreviewSettle => PresentTxnPhase {
            crossing: crossing::transition(phase.crossing, CrossingEvent::SettleElapsed),
            ..phase
        },
        PresentSkipped => PresentTxnPhase {
            crossing: crossing::transition(phase.crossing, CrossingEvent::PresentSkipped),
            ..phase
        },
        PresentSucceeded => PresentTxnPhase {
            crossing: crossing::transition(phase.crossing, CrossingEvent::PresentSucceeded),
            ..phase
        },
        Teardown => PresentTxnPhase::IDLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_composed_phase_matches_the_live_present_sync_owner() {
        for phase in PresentTxnPhase::all() {
            let (resize, move_, crossing) = phase.claims();
            assert_eq!(
                phase.claims_present_sync(),
                super::super::present_sync_armed(resize, move_, crossing),
                "phase {phase:?} must compose exactly like App::sync_present_txn"
            );
        }
    }

    #[test]
    fn every_transition_is_total_over_every_composed_phase() {
        for phase in PresentTxnPhase::all() {
            for lifecycle in PresentTxnTransition::ALL {
                let next = transition(phase, lifecycle);
                let (resize, move_, crossing) = next.claims();
                assert_eq!(
                    next.claims_present_sync(),
                    super::super::present_sync_armed(resize, move_, crossing),
                    "{lifecycle:?} from {phase:?}"
                );
            }
        }
    }

    #[test]
    fn each_lifecycle_owner_changes_only_its_own_claim_until_global_teardown() {
        for phase in PresentTxnPhase::all() {
            use PresentTxnTransition::{
                MoveArm, MoveSettle, PresentSkipped, PresentSucceeded, PreviewArm, PreviewSettle,
                ResizeArm, ResizeSettle, Teardown,
            };

            for lifecycle in PresentTxnTransition::ALL {
                let next = transition(phase, lifecycle);
                match lifecycle {
                    ResizeArm | ResizeSettle => {
                        assert_eq!(next.move_, phase.move_);
                        assert_eq!(next.crossing, phase.crossing);
                    }
                    MoveArm | MoveSettle => {
                        assert_eq!(next.resize, phase.resize);
                        assert_eq!(next.crossing, phase.crossing);
                    }
                    PreviewArm | PreviewSettle | PresentSkipped | PresentSucceeded => {
                        assert_eq!(next.resize, phase.resize);
                        assert_eq!(next.move_, phase.move_);
                    }
                    Teardown => assert_eq!(next, PresentTxnPhase::IDLE),
                }
            }
        }
    }

    #[test]
    fn crossing_settle_holds_the_bracket_until_a_successful_present() {
        let previewed = transition(PresentTxnPhase::IDLE, PresentTxnTransition::PreviewArm);
        let awaiting = transition(previewed, PresentTxnTransition::PreviewSettle);
        assert_eq!(awaiting.crossing, CrossingPhase::AwaitingPresent);
        assert!(
            awaiting.claims_present_sync(),
            "settle hands off, never disarms"
        );

        let skipped = transition(awaiting, PresentTxnTransition::PresentSkipped);
        assert_eq!(
            skipped, awaiting,
            "a skipped present cannot consume teardown"
        );

        let disarmed = transition(skipped, PresentTxnTransition::PresentSucceeded);
        assert_eq!(disarmed, PresentTxnPhase::IDLE);
        assert!(!disarmed.claims_present_sync());
    }

    #[test]
    fn one_sources_settle_or_teardown_never_strips_another_sources_claim() {
        let resize_and_move = transition(
            transition(PresentTxnPhase::IDLE, PresentTxnTransition::ResizeArm),
            PresentTxnTransition::MoveArm,
        );
        assert!(
            transition(resize_and_move, PresentTxnTransition::ResizeSettle).claims_present_sync()
        );

        let resize_and_preview = transition(
            transition(PresentTxnPhase::IDLE, PresentTxnTransition::ResizeArm),
            PresentTxnTransition::PreviewArm,
        );
        let awaiting = transition(resize_and_preview, PresentTxnTransition::PreviewSettle);
        let after_present = transition(awaiting, PresentTxnTransition::PresentSucceeded);
        assert!(
            after_present.claims_present_sync(),
            "resize still owns the bracket"
        );
        assert_eq!(after_present.resize, StreamPhase::Armed);
    }
}
