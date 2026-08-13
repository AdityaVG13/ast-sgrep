/// Optional conformance verdict tags for table-driven tests.
///
/// Default remains panic/`assert!` (Fail). XFAIL is only valid with a
/// registered id from `docs/validation/DISCREPANCIES.md`. This is not a
/// runner -- suites keep their own asserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestVerdict {
    Pass,
    Fail,
    Ignore {
        reason: &'static str,
        disc_id: Option<&'static str>,
    },
    ExpectedFailure {
        disc_id: &'static str,
    },
    NotRun,
}

impl TestVerdict {
    pub fn disc_id(self) -> Option<&'static str> {
        match self {
            Self::Ignore { disc_id, .. } => disc_id,
            Self::ExpectedFailure { disc_id } => Some(disc_id),
            Self::Pass | Self::Fail | Self::NotRun => None,
        }
    }
}
