//! Resource handoff state machine. A prepared payload is owned by exactly one side.
//!
//! Destination adapters must return the unchanged payload when installation fails.
//! Runtime resources must never be retired as part of transfer or rollback.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransferPhase {
    Prepared,
    Installed,
    Committed,
    RolledBack,
}

pub(crate) struct TransferTransaction<P> {
    phase: TransferPhase,
    payload: Option<P>,
}

impl<P> TransferTransaction<P> {
    pub(crate) fn prepare(payload: P) -> Self {
        Self {
            phase: TransferPhase::Prepared,
            payload: Some(payload),
        }
    }

    pub(crate) fn phase(&self) -> TransferPhase {
        self.phase
    }

    pub(crate) fn install<E>(
        &mut self,
        install: impl FnOnce(P) -> Result<(), (E, P)>,
    ) -> Result<(), E> {
        assert_eq!(self.phase, TransferPhase::Prepared);
        let payload = self
            .payload
            .take()
            .expect("prepared transfer owns its payload");
        match install(payload) {
            Ok(()) => {
                self.phase = TransferPhase::Installed;
                Ok(())
            }
            Err((error, payload)) => {
                self.payload = Some(payload);
                Err(error)
            }
        }
    }

    pub(crate) fn commit(&mut self) {
        assert_eq!(self.phase, TransferPhase::Installed);
        self.phase = TransferPhase::Committed;
    }

    pub(crate) fn rollback(&mut self) -> Option<P> {
        if self.phase != TransferPhase::Prepared {
            return None;
        }
        self.phase = TransferPhase::RolledBack;
        self.payload.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct LivePayload {
        identity: Rc<()>,
        retired: Rc<Cell<usize>>,
    }

    impl Drop for LivePayload {
        fn drop(&mut self) {
            self.retired.set(self.retired.get() + 1);
        }
    }

    #[test]
    fn rejected_destination_returns_the_same_live_payload_exactly_once() {
        let identity = Rc::new(());
        let retired = Rc::new(Cell::new(0));
        let mut transaction = TransferTransaction::prepare(LivePayload {
            identity: identity.clone(),
            retired: retired.clone(),
        });
        assert_eq!(
            transaction.install(|payload| Err(("destination closed", payload))),
            Err("destination closed")
        );
        assert_eq!(transaction.phase(), TransferPhase::Prepared);
        assert_eq!(retired.get(), 0);
        let restored = transaction.rollback().unwrap();
        assert!(Rc::ptr_eq(&restored.identity, &identity));
        assert_eq!(transaction.phase(), TransferPhase::RolledBack);
        assert!(transaction.rollback().is_none());
        assert_eq!(retired.get(), 0);
        drop(restored);
        assert_eq!(retired.get(), 1);
    }

    #[test]
    fn successful_install_has_one_destination_owner_and_commit_cannot_roll_back() {
        let identity = Rc::new(());
        let retired = Rc::new(Cell::new(0));
        let mut destination = None;
        let mut transaction = TransferTransaction::prepare(LivePayload {
            identity: identity.clone(),
            retired: retired.clone(),
        });
        let result: Result<(), ()> = transaction.install(|payload| {
            destination = Some(payload);
            Ok(())
        });
        assert_eq!(result, Ok(()));
        assert_eq!(transaction.phase(), TransferPhase::Installed);
        assert!(transaction.rollback().is_none());
        transaction.commit();
        assert_eq!(transaction.phase(), TransferPhase::Committed);
        assert!(transaction.rollback().is_none());
        drop(transaction);
        assert_eq!(retired.get(), 0);
        let installed = destination.unwrap();
        assert!(Rc::ptr_eq(&installed.identity, &identity));
        drop(installed);
        assert_eq!(retired.get(), 1);
    }
}
