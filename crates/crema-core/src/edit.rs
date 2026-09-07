use crate::sidecar::{ConflictKind, EditableSidecar, SaveFailure, SidecarBlockReason, SidecarOpen};
use std::error::Error;
use std::fmt;
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub struct ExposureCentistops(i16);

impl ExposureCentistops {
    pub const MIN: Self = Self(-500);
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(500);

    pub fn new(value: i16) -> Result<Self, ExposureOutOfRange> {
        if (Self::MIN.0..=Self::MAX.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ExposureOutOfRange(value))
        }
    }

    pub const fn value(self) -> i16 {
        self.0
    }

    pub fn as_stops(self) -> f32 {
        f32::from(self.0) / 100.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExposureOutOfRange(i16);

impl fmt::Display for ExposureOutOfRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "exposure {} is outside -500..=500 centistops",
            self.0
        )
    }
}

impl Error for ExposureOutOfRange {}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct EditRecipe {
    exposure: ExposureCentistops,
}

impl EditRecipe {
    pub const fn new(exposure: ExposureCentistops) -> Self {
        Self { exposure }
    }

    pub const fn exposure(&self) -> ExposureCentistops {
        self.exposure
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EditRevision(u64);

impl EditRevision {
    pub const ZERO: Self = Self(0);

    pub const fn value(self) -> u64 {
        self.0
    }

    fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("edit revision space exhausted"),
        )
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RecipeSnapshot {
    revision: EditRevision,
    recipe: EditRecipe,
}

impl RecipeSnapshot {
    pub const fn revision(&self) -> EditRevision {
        self.revision
    }

    pub const fn recipe(&self) -> &EditRecipe {
        &self.recipe
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditCommand {
    SetExposure(ExposureCentistops),
    ResetExposure,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SaveJobId(u64);

#[derive(Clone, Debug)]
pub struct SaveCommand {
    pub(crate) job: SaveJobId,
    pub(crate) submitted: RecipeSnapshot,
    pub(crate) document: EditableSidecar,
}

impl SaveCommand {
    pub const fn job(&self) -> SaveJobId {
        self.job
    }

    pub const fn submitted(&self) -> &RecipeSnapshot {
        &self.submitted
    }
}

#[derive(Clone, Debug)]
pub struct SaveReceipt {
    pub(crate) job: SaveJobId,
    pub(crate) submitted: RecipeSnapshot,
    pub(crate) document: EditableSidecar,
}

#[derive(Clone, Debug)]
pub struct SaveCompletion {
    pub(crate) job: SaveJobId,
    pub(crate) result: Result<SaveReceipt, SaveFailure>,
}

impl SaveCompletion {
    pub(crate) fn saved(receipt: SaveReceipt) -> Self {
        Self {
            job: receipt.job,
            result: Ok(receipt),
        }
    }

    pub fn failed(job: SaveJobId, failure: SaveFailure) -> Self {
        Self {
            job,
            result: Err(failure),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveState {
    Saved,
    Dirty,
    Saving {
        submitted: EditRevision,
        current: EditRevision,
    },
    ReadOnly(SidecarBlockReason),
    Conflict(ConflictKind),
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BeginSaveError {
    Busy,
    Blocked(SaveState),
}

impl fmt::Display for BeginSaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => formatter.write_str("a save is already running"),
            Self::Blocked(state) => write!(formatter, "save is blocked by {state:?}"),
        }
    }
}

impl Error for BeginSaveError {}

#[derive(Clone, Debug)]
enum DocumentAccess {
    Editable(EditableSidecar),
    ReadOnly(SidecarBlockReason),
    Conflict {
        document: EditableSidecar,
        kind: ConflictKind,
    },
}

#[derive(Clone, Debug)]
enum SaveOperation {
    Idle,
    Saving {
        job: SaveJobId,
        submitted: RecipeSnapshot,
    },
    Failed(String),
}

pub struct EditSession {
    access: DocumentAccess,
    durable: RecipeSnapshot,
    draft: RecipeSnapshot,
    operation: SaveOperation,
    next_job: u64,
}

impl EditSession {
    pub fn open(open: SidecarOpen) -> Self {
        let (recipe, access) = match open {
            SidecarOpen::Editable { recipe, document } => {
                (recipe, DocumentAccess::Editable(document))
            }
            SidecarOpen::Blocked { recipe, reason } => (recipe, DocumentAccess::ReadOnly(reason)),
        };
        let snapshot = RecipeSnapshot {
            revision: EditRevision::ZERO,
            recipe,
        };
        Self {
            access,
            durable: snapshot.clone(),
            draft: snapshot,
            operation: SaveOperation::Idle,
            next_job: 1,
        }
    }

    pub const fn recipe(&self) -> &EditRecipe {
        &self.draft.recipe
    }

    pub const fn durable_recipe(&self) -> &EditRecipe {
        &self.durable.recipe
    }

    pub const fn revision(&self) -> EditRevision {
        self.draft.revision
    }

    pub fn snapshot(&self) -> RecipeSnapshot {
        self.draft.clone()
    }

    pub fn is_dirty(&self) -> bool {
        self.draft.recipe != self.durable.recipe
    }

    pub fn apply(&mut self, command: EditCommand) -> Option<RecipeSnapshot> {
        let exposure = match command {
            EditCommand::SetExposure(exposure) => exposure,
            EditCommand::ResetExposure => ExposureCentistops::ZERO,
        };
        if exposure == self.draft.recipe.exposure {
            return None;
        }
        self.draft = RecipeSnapshot {
            revision: self.draft.revision.next(),
            recipe: EditRecipe::new(exposure),
        };
        Some(self.draft.clone())
    }

    pub fn begin_save(&mut self) -> Result<Option<SaveCommand>, BeginSaveError> {
        if matches!(self.operation, SaveOperation::Saving { .. }) {
            return Err(BeginSaveError::Busy);
        }
        let document = match &self.access {
            DocumentAccess::Editable(document) => document.clone(),
            DocumentAccess::ReadOnly(reason) => {
                return Err(BeginSaveError::Blocked(SaveState::ReadOnly(reason.clone())));
            }
            DocumentAccess::Conflict { kind, .. } => {
                return Err(BeginSaveError::Blocked(SaveState::Conflict(*kind)));
            }
        };
        if !self.is_dirty() {
            self.operation = SaveOperation::Idle;
            return Ok(None);
        }
        let job = SaveJobId(self.next_job);
        self.next_job = self
            .next_job
            .checked_add(1)
            .expect("save job space exhausted");
        let submitted = self.draft.clone();
        self.operation = SaveOperation::Saving {
            job,
            submitted: submitted.clone(),
        };
        Ok(Some(SaveCommand {
            job,
            submitted,
            document,
        }))
    }

    pub fn accept_save(&mut self, completion: SaveCompletion) {
        let SaveOperation::Saving { job, submitted } = &self.operation else {
            return;
        };
        if *job != completion.job {
            return;
        }
        match completion.result {
            Ok(receipt) => {
                debug_assert_eq!(receipt.submitted, *submitted);
                self.durable = receipt.submitted;
                self.access = DocumentAccess::Editable(receipt.document);
                self.operation = SaveOperation::Idle;
            }
            Err(SaveFailure::Conflict(kind)) => {
                let DocumentAccess::Editable(document) = &self.access else {
                    return;
                };
                self.access = DocumentAccess::Conflict {
                    document: document.clone(),
                    kind,
                };
                self.operation = SaveOperation::Idle;
            }
            Err(SaveFailure::ReadOnly(reason)) => {
                self.access = DocumentAccess::ReadOnly(reason);
                self.operation = SaveOperation::Idle;
            }
            Err(SaveFailure::Io { message, .. }) => {
                self.operation = SaveOperation::Failed(message);
            }
        }
    }

    pub fn save_state(&self) -> SaveState {
        if let SaveOperation::Saving { submitted, .. } = &self.operation {
            return SaveState::Saving {
                submitted: submitted.revision,
                current: self.draft.revision,
            };
        }
        match &self.access {
            DocumentAccess::ReadOnly(reason) => return SaveState::ReadOnly(reason.clone()),
            DocumentAccess::Conflict { kind, .. } => return SaveState::Conflict(*kind),
            DocumentAccess::Editable(_) => {}
        }
        if let SaveOperation::Failed(message) = &self.operation {
            return SaveState::Failed(message.clone());
        }
        if self.is_dirty() {
            SaveState::Dirty
        } else {
            SaveState::Saved
        }
    }

    pub fn sidecar_path(&self) -> &Path {
        match &self.access {
            DocumentAccess::Editable(document) | DocumentAccess::Conflict { document, .. } => {
                document.location().sidecar()
            }
            DocumentAccess::ReadOnly(reason) => reason.path(),
        }
    }
}
