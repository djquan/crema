use crate::edit::{EditRecipe, ExposureCentistops, SaveCommand, SaveCompletion, SaveReceipt};
use quick_xml::escape::escape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_XMP_BYTES: usize = 64 * 1024;
const XMP_NS: &str = "adobe:ns:meta/";
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const CREMA_NS: &str = "urn:crema:xmp:edit";
const CURRENT_SCHEMA: u32 = 2;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SidecarNaming {
    ReplaceOriginalExtension,
    AppendXmpExtension,
}

pub type SidecarClassifier = fn(&Path) -> Option<SidecarNaming>;

#[derive(Clone, Debug)]
pub struct SidecarLocator {
    original: PathBuf,
    naming: SidecarNaming,
    classify: SidecarClassifier,
}

impl SidecarLocator {
    pub fn new(original: &Path, naming: SidecarNaming, classify: SidecarClassifier) -> Self {
        Self {
            original: original.to_owned(),
            naming,
            classify,
        }
    }

    pub fn sidecar(&self) -> PathBuf {
        SidecarLocation::for_original(&self.original, self.naming)
            .expect("sidecar locator requires a valid original path")
            .sidecar
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidecarLocation {
    original: PathBuf,
    sidecar: PathBuf,
}

impl SidecarLocation {
    pub fn for_original(original: &Path, naming: SidecarNaming) -> Result<Self, SidecarPathError> {
        let file_name = original
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| SidecarPathError(original.to_owned()))?;
        let parent = original.parent().unwrap_or_else(|| Path::new("."));
        let sidecar = match naming {
            SidecarNaming::ReplaceOriginalExtension => {
                original
                    .file_stem()
                    .filter(|stem| !stem.is_empty())
                    .ok_or_else(|| SidecarPathError(original.to_owned()))?;
                original.with_extension("xmp")
            }
            SidecarNaming::AppendXmpExtension => {
                let mut sidecar_name = OsString::from(file_name);
                sidecar_name.push(".xmp");
                parent.join(sidecar_name)
            }
        };
        if sidecar == original {
            return Err(SidecarPathError(original.to_owned()));
        }
        Ok(Self {
            original: original.to_owned(),
            sidecar,
        })
    }

    pub fn original(&self) -> &Path {
        &self.original
    }

    pub fn sidecar(&self) -> &Path {
        &self.sidecar
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidecarPathError(PathBuf);

impl fmt::Display for SidecarPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cannot derive an XMP sidecar path from {}",
            self.0.display()
        )
    }
}

impl Error for SidecarPathError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictKind {
    CreatedExternally,
    RemovedExternally,
    ChangedExternally,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SidecarBlockReason {
    AmbiguousOriginals {
        path: PathBuf,
        originals: Vec<PathBuf>,
    },
    AmbiguousSidecars {
        original: PathBuf,
        candidates: Vec<PathBuf>,
    },
    AssociatedWithOtherOriginal {
        path: PathBuf,
        source_file_name: String,
    },
    UnsupportedOriginalName {
        path: PathBuf,
    },
    UnrecognizedExisting {
        path: PathBuf,
    },
    NewerSchema {
        path: PathBuf,
        found: u32,
    },
    TooLarge {
        path: PathBuf,
        max_bytes: usize,
    },
    UnsafeTarget {
        path: PathBuf,
    },
    WriteDenied {
        path: PathBuf,
        message: String,
    },
}

impl SidecarBlockReason {
    pub fn path(&self) -> &Path {
        match self {
            Self::AmbiguousOriginals { path, .. }
            | Self::AssociatedWithOtherOriginal { path, .. }
            | Self::UnsupportedOriginalName { path }
            | Self::UnrecognizedExisting { path }
            | Self::NewerSchema { path, .. }
            | Self::TooLarge { path, .. }
            | Self::UnsafeTarget { path }
            | Self::WriteDenied { path, .. } => path,
            Self::AmbiguousSidecars { original, .. } => original,
        }
    }
}

#[derive(Clone, Debug)]
enum SidecarObservation {
    Absent,
    Owned(Arc<[u8]>),
}

#[derive(Clone, Debug)]
pub struct EditableSidecar {
    locator: SidecarLocator,
    location: SidecarLocation,
    observed: SidecarObservation,
    association: SidecarAssociation,
}

impl EditableSidecar {
    pub(crate) fn location(&self) -> &SidecarLocation {
        &self.location
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SidecarAssociation {
    Unique,
    Explicit,
}

#[derive(Clone, Debug)]
pub enum SidecarOpen {
    Editable {
        recipe: EditRecipe,
        document: EditableSidecar,
    },
    Blocked {
        recipe: EditRecipe,
        reason: SidecarBlockReason,
    },
}

#[derive(Debug)]
pub struct SidecarReadError {
    path: PathBuf,
    source: io::Error,
}

impl fmt::Display for SidecarReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cannot read sidecar {}: {}",
            self.path.display(),
            self.source
        )
    }
}

impl Error for SidecarReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Clone, Debug)]
pub enum SaveFailure {
    Conflict(ConflictKind),
    ReadOnly(SidecarBlockReason),
    Io { path: PathBuf, message: String },
}

enum SidecarDiscovery {
    Selected(SidecarLocation),
    Blocked(SidecarBlockReason),
}

fn discover_sidecar(locator: &SidecarLocator) -> Result<SidecarDiscovery, SidecarReadError> {
    let primary =
        SidecarLocation::for_original(&locator.original, locator.naming).map_err(|error| {
            SidecarReadError {
                path: locator.original.clone(),
                source: io::Error::new(io::ErrorKind::InvalidInput, error),
            }
        })?;
    if locator.naming == SidecarNaming::AppendXmpExtension {
        return Ok(SidecarDiscovery::Selected(primary));
    }
    let alternate =
        SidecarLocation::for_original(&locator.original, SidecarNaming::AppendXmpExtension)
            .map_err(|error| SidecarReadError {
                path: locator.original.clone(),
                source: io::Error::new(io::ErrorKind::InvalidInput, error),
            })?;
    let mut existing = Vec::new();
    for location in [&primary, &alternate] {
        match fs::symlink_metadata(location.sidecar()) {
            Ok(_) => existing.push(location.sidecar().to_owned()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(SidecarReadError {
                    path: location.sidecar().to_owned(),
                    source,
                });
            }
        }
    }
    if existing.len() > 1 {
        return Ok(SidecarDiscovery::Blocked(
            SidecarBlockReason::AmbiguousSidecars {
                original: locator.original.clone(),
                candidates: existing,
            },
        ));
    }
    Ok(SidecarDiscovery::Selected(if existing.is_empty() {
        primary
    } else if existing[0] == *alternate.sidecar() {
        alternate
    } else {
        primary
    }))
}

fn resolve_association(
    locator: &SidecarLocator,
    location: &SidecarLocation,
    associated_source: Option<&str>,
) -> Result<Result<SidecarAssociation, SidecarBlockReason>, SidecarReadError> {
    let source_file_name = match locator.original.file_name().and_then(|name| name.to_str()) {
        Some(name) => name,
        None => {
            return Ok(Err(SidecarBlockReason::UnsupportedOriginalName {
                path: locator.original.clone(),
            }));
        }
    };
    if let Some(associated_source) = associated_source {
        return Ok(if associated_source == source_file_name {
            Ok(SidecarAssociation::Explicit)
        } else {
            Err(SidecarBlockReason::AssociatedWithOtherOriginal {
                path: location.sidecar.clone(),
                source_file_name: associated_source.to_owned(),
            })
        });
    }
    if locator.naming == SidecarNaming::AppendXmpExtension {
        return Ok(Ok(SidecarAssociation::Unique));
    }
    let primary =
        SidecarLocation::for_original(&locator.original, SidecarNaming::ReplaceOriginalExtension)
            .expect("validated original path");
    if location.sidecar != primary.sidecar {
        return Ok(Ok(SidecarAssociation::Unique));
    }
    let parent = locator.original.parent().unwrap_or_else(|| Path::new("."));
    let mut originals = Vec::new();
    for entry in fs::read_dir(parent).map_err(|source| SidecarReadError {
        path: parent.to_owned(),
        source,
    })? {
        let entry = entry.map_err(|source| SidecarReadError {
            path: parent.to_owned(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| SidecarReadError {
            path: entry.path(),
            source,
        })?;
        if !file_type.is_file()
            || (locator.classify)(&entry.path()) != Some(SidecarNaming::ReplaceOriginalExtension)
        {
            continue;
        }
        let candidate =
            SidecarLocation::for_original(&entry.path(), SidecarNaming::ReplaceOriginalExtension)
                .map_err(|error| SidecarReadError {
                path: entry.path(),
                source: io::Error::new(io::ErrorKind::InvalidInput, error),
            })?;
        if same_sidecar_slot(&candidate.sidecar, &location.sidecar) {
            originals.push(entry.path());
        }
    }
    originals.sort();
    Ok(if originals.len() > 1 {
        Err(SidecarBlockReason::AmbiguousOriginals {
            path: location.sidecar.clone(),
            originals,
        })
    } else {
        Ok(SidecarAssociation::Unique)
    })
}

fn same_sidecar_slot(left: &Path, right: &Path) -> bool {
    left == right
        || (left.parent() == right.parent()
            && left
                .file_name()
                .and_then(|name| name.to_str())
                .zip(right.file_name().and_then(|name| name.to_str()))
                .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right)))
}

#[derive(Default)]
pub struct SidecarStore;

impl SidecarStore {
    pub fn open(&self, locator: SidecarLocator) -> Result<SidecarOpen, SidecarReadError> {
        let location = match discover_sidecar(&locator)? {
            SidecarDiscovery::Selected(location) => location,
            SidecarDiscovery::Blocked(reason) => {
                return Ok(SidecarOpen::Blocked {
                    recipe: EditRecipe::default(),
                    reason,
                });
            }
        };
        let path = location.sidecar.clone();
        match read_bounded_sidecar(&path) {
            Ok(CurrentSidecar::Absent) => {
                self.finish_open(locator, location, EditRecipe::default(), None, None)
            }
            Ok(CurrentSidecar::Unsafe) => Ok(SidecarOpen::Blocked {
                recipe: EditRecipe::default(),
                reason: SidecarBlockReason::UnsafeTarget { path },
            }),
            Ok(CurrentSidecar::TooLarge) => Ok(SidecarOpen::Blocked {
                recipe: EditRecipe::default(),
                reason: SidecarBlockReason::TooLarge {
                    path,
                    max_bytes: MAX_XMP_BYTES,
                },
            }),
            Ok(CurrentSidecar::Bytes(bytes)) => match parse_owned_xmp(&bytes) {
                Ok(packet) => self.finish_open(
                    locator,
                    location,
                    packet.recipe,
                    packet.source_file_name.as_deref(),
                    Some(bytes),
                ),
                Err(ParseRefusal::NewerSchema(found)) => Ok(SidecarOpen::Blocked {
                    recipe: EditRecipe::default(),
                    reason: SidecarBlockReason::NewerSchema { path, found },
                }),
                Err(ParseRefusal::Unrecognized) => Ok(SidecarOpen::Blocked {
                    recipe: EditRecipe::default(),
                    reason: SidecarBlockReason::UnrecognizedExisting { path },
                }),
            },
            Err(source) => Err(SidecarReadError { path, source }),
        }
    }

    fn finish_open(
        &self,
        locator: SidecarLocator,
        location: SidecarLocation,
        recipe: EditRecipe,
        source_file_name: Option<&str>,
        bytes: Option<Vec<u8>>,
    ) -> Result<SidecarOpen, SidecarReadError> {
        match resolve_association(&locator, &location, source_file_name)? {
            Ok(association) => Ok(SidecarOpen::Editable {
                recipe,
                document: EditableSidecar {
                    locator,
                    location,
                    observed: bytes
                        .map(|bytes| SidecarObservation::Owned(bytes.into()))
                        .unwrap_or(SidecarObservation::Absent),
                    association,
                },
            }),
            Err(reason) => Ok(SidecarOpen::Blocked { recipe, reason }),
        }
    }

    pub fn commit(&self, command: SaveCommand) -> SaveCompletion {
        self.commit_guarded(command, || Ok(()))
    }

    pub fn commit_guarded(
        &self,
        command: SaveCommand,
        validate: impl FnOnce() -> Result<(), SaveFailure>,
    ) -> SaveCompletion {
        let job = command.job;
        match commit_sidecar(&command.document, command.submitted.recipe(), validate) {
            Ok(document) => SaveCompletion::saved(SaveReceipt {
                job,
                submitted: command.submitted,
                document,
            }),
            Err(failure) => SaveCompletion::failed(job, failure),
        }
    }
}

enum CurrentSidecar {
    Absent,
    Bytes(Vec<u8>),
    TooLarge,
    Unsafe,
}

fn read_bounded_sidecar(path: &Path) -> io::Result<CurrentSidecar> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(CurrentSidecar::Absent),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() {
        return Ok(CurrentSidecar::Unsafe);
    }
    let mut bytes = Vec::with_capacity(metadata.len().min((MAX_XMP_BYTES + 1) as u64) as usize);
    File::open(path)?
        .take((MAX_XMP_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_XMP_BYTES {
        Ok(CurrentSidecar::TooLarge)
    } else {
        Ok(CurrentSidecar::Bytes(bytes))
    }
}

fn commit_sidecar(
    document: &EditableSidecar,
    recipe: &EditRecipe,
    validate: impl FnOnce() -> Result<(), SaveFailure>,
) -> Result<EditableSidecar, SaveFailure> {
    compare_observation(document)?;
    validate_association(document)?;
    let source_file_name = document
        .location
        .original
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            SaveFailure::ReadOnly(SidecarBlockReason::UnsupportedOriginalName {
                path: document.location.original.clone(),
            })
        })?;
    let bytes = serialize_owned_xmp(recipe, source_file_name);
    let (temporary_path, mut temporary) = create_temporary(&document.location)?;
    let write_result = (|| -> io::Result<()> {
        temporary.write_all(&bytes)?;
        temporary.flush()?;
        temporary.sync_all()?;
        Ok(())
    })();
    drop(temporary);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(map_write_error(&document.location.sidecar, error));
    }

    if let Err(failure) = compare_observation(document) {
        let _ = fs::remove_file(&temporary_path);
        return Err(failure);
    }
    if let Err(failure) = validate_association(document) {
        let _ = fs::remove_file(&temporary_path);
        return Err(failure);
    }
    if let Err(failure) = validate() {
        let _ = fs::remove_file(&temporary_path);
        return Err(failure);
    }

    let publication = match document.observed {
        SidecarObservation::Absent => fs::hard_link(&temporary_path, &document.location.sidecar),
        SidecarObservation::Owned(_) => fs::rename(&temporary_path, &document.location.sidecar),
    };
    if let Err(error) = publication {
        let _ = fs::remove_file(&temporary_path);
        if matches!(document.observed, SidecarObservation::Absent)
            && error.kind() == io::ErrorKind::AlreadyExists
        {
            return Err(SaveFailure::Conflict(ConflictKind::CreatedExternally));
        }
        return Err(map_write_error(&document.location.sidecar, error));
    }
    if matches!(document.observed, SidecarObservation::Absent) {
        let _ = fs::remove_file(&temporary_path);
    }

    sync_parent(&document.location.sidecar)
        .map_err(|error| map_write_error(&document.location.sidecar, error))?;
    let final_bytes = match read_bounded_sidecar(&document.location.sidecar) {
        Ok(CurrentSidecar::Bytes(final_bytes)) if final_bytes == bytes => final_bytes,
        Ok(_) => {
            return Err(SaveFailure::Io {
                path: document.location.sidecar.clone(),
                message: "published sidecar could not be verified".to_owned(),
            });
        }
        Err(error) => return Err(map_write_error(&document.location.sidecar, error)),
    };
    Ok(EditableSidecar {
        locator: document.locator.clone(),
        location: document.location.clone(),
        observed: SidecarObservation::Owned(final_bytes.into()),
        association: SidecarAssociation::Explicit,
    })
}

fn validate_association(document: &EditableSidecar) -> Result<(), SaveFailure> {
    let location = match discover_sidecar(&document.locator).map_err(|error| SaveFailure::Io {
        path: error.path,
        message: error.source.to_string(),
    })? {
        SidecarDiscovery::Selected(location) => location,
        SidecarDiscovery::Blocked(reason) => return Err(SaveFailure::ReadOnly(reason)),
    };
    if location != document.location {
        return Err(SaveFailure::ReadOnly(
            SidecarBlockReason::AmbiguousSidecars {
                original: document.location.original.clone(),
                candidates: vec![document.location.sidecar.clone(), location.sidecar],
            },
        ));
    }
    let associated_source = (document.association == SidecarAssociation::Explicit)
        .then(|| {
            document
                .location
                .original
                .file_name()
                .and_then(|name| name.to_str())
        })
        .flatten();
    match resolve_association(&document.locator, &document.location, associated_source).map_err(
        |error| SaveFailure::Io {
            path: error.path,
            message: error.source.to_string(),
        },
    )? {
        Ok(_) => Ok(()),
        Err(reason) => Err(SaveFailure::ReadOnly(reason)),
    }
}

fn compare_observation(document: &EditableSidecar) -> Result<(), SaveFailure> {
    let current = read_bounded_sidecar(&document.location.sidecar)
        .map_err(|error| map_write_error(&document.location.sidecar, error))?;
    match (&document.observed, current) {
        (SidecarObservation::Absent, CurrentSidecar::Absent) => Ok(()),
        (SidecarObservation::Absent, _) => {
            Err(SaveFailure::Conflict(ConflictKind::CreatedExternally))
        }
        (SidecarObservation::Owned(_), CurrentSidecar::Absent) => {
            Err(SaveFailure::Conflict(ConflictKind::RemovedExternally))
        }
        (SidecarObservation::Owned(expected), CurrentSidecar::Bytes(actual))
            if expected.as_ref() == actual =>
        {
            Ok(())
        }
        (SidecarObservation::Owned(_), _) => {
            Err(SaveFailure::Conflict(ConflictKind::ChangedExternally))
        }
    }
}

fn create_temporary(location: &SidecarLocation) -> Result<(PathBuf, File), SaveFailure> {
    let parent = usable_parent(&location.sidecar);
    let file_name = location
        .sidecar
        .file_name()
        .expect("validated sidecar path must have a filename")
        .to_string_lossy();
    for _ in 0..100 {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{file_name}.crema-tmp-{}-{sequence}",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(map_write_error(&location.sidecar, error)),
        }
    }
    Err(SaveFailure::Io {
        path: location.sidecar.clone(),
        message: "could not reserve a unique sidecar temporary file".to_owned(),
    })
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(usable_parent(path))?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn usable_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn map_write_error(path: &Path, error: io::Error) -> SaveFailure {
    if error.kind() == io::ErrorKind::PermissionDenied {
        SaveFailure::ReadOnly(SidecarBlockReason::WriteDenied {
            path: path.to_owned(),
            message: error.to_string(),
        })
    } else {
        SaveFailure::Io {
            path: path.to_owned(),
            message: error.to_string(),
        }
    }
}

fn serialize_owned_xmp(recipe: &EditRecipe, source_file_name: &str) -> Vec<u8> {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<x:xmpmeta xmlns:x=\"{XMP_NS}\">\n",
            "  <rdf:RDF xmlns:rdf=\"{RDF_NS}\">\n",
            "    <rdf:Description rdf:about=\"\" xmlns:crema=\"{CREMA_NS}\" crema:owner=\"Crema\" crema:schemaVersion=\"{CURRENT_SCHEMA}\" crema:sourceFileName=\"{}\" crema:exposureCentistops=\"{}\"/>\n",
            "  </rdf:RDF>\n",
            "</x:xmpmeta>\n",
        ),
        escape(source_file_name),
        recipe.exposure().value(),
        XMP_NS = XMP_NS,
        RDF_NS = RDF_NS,
        CREMA_NS = CREMA_NS,
        CURRENT_SCHEMA = CURRENT_SCHEMA,
    )
    .into_bytes()
}

struct OwnedPacket {
    recipe: EditRecipe,
    source_file_name: Option<String>,
}

enum ParseRefusal {
    Unrecognized,
    NewerSchema(u32),
}

#[derive(Clone)]
struct ElementName {
    namespace: Option<String>,
    local: String,
}

struct ElementFrame {
    name: ElementName,
    namespaces: HashMap<String, String>,
}

struct InspectedElement {
    name: ElementName,
    namespaces: HashMap<String, String>,
    packet: Option<OwnedPacket>,
}

#[derive(Default)]
struct DocumentStructure {
    rdf_containers: u8,
    descriptions: u8,
}

impl DocumentStructure {
    fn observe(&mut self, name: &ElementName) -> Result<(), ParseRefusal> {
        match (name.namespace.as_deref(), name.local.as_str()) {
            (Some(RDF_NS), "RDF") => {
                if self.rdf_containers != 0 {
                    return Err(ParseRefusal::Unrecognized);
                }
                self.rdf_containers = 1;
            }
            (Some(RDF_NS), "Description") => {
                if self.descriptions != 0 {
                    return Err(ParseRefusal::Unrecognized);
                }
                self.descriptions = 1;
            }
            _ => {}
        }
        Ok(())
    }

    fn require_owned_shape(self) -> Result<(), ParseRefusal> {
        if self.rdf_containers == 1 && self.descriptions == 1 {
            Ok(())
        } else {
            Err(ParseRefusal::Unrecognized)
        }
    }
}

fn parse_owned_xmp(bytes: &[u8]) -> Result<OwnedPacket, ParseRefusal> {
    if std::str::from_utf8(bytes).is_err() {
        return Err(ParseRefusal::Unrecognized);
    }
    let mut reader = Reader::from_reader(BufReader::new(bytes));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut stack: Vec<ElementFrame> = Vec::new();
    let mut packet = None;
    let mut structure = DocumentStructure::default();
    let mut saw_declaration = false;
    let mut saw_root = false;

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|_| ParseRefusal::Unrecognized)?;
        match event {
            Event::Decl(_) if !saw_declaration && !saw_root => saw_declaration = true,
            Event::Start(start) => {
                if stack.is_empty() && saw_root {
                    return Err(ParseRefusal::Unrecognized);
                }
                saw_root = true;
                let inspected = inspect_element(&start, &stack)?;
                structure.observe(&inspected.name)?;
                if let Some(parsed_packet) = inspected.packet {
                    packet = Some(parsed_packet);
                }
                stack.push(ElementFrame {
                    name: inspected.name,
                    namespaces: inspected.namespaces,
                });
            }
            Event::Empty(start) => {
                if stack.is_empty() && saw_root {
                    return Err(ParseRefusal::Unrecognized);
                }
                saw_root = true;
                let inspected = inspect_element(&start, &stack)?;
                structure.observe(&inspected.name)?;
                if let Some(parsed_packet) = inspected.packet {
                    packet = Some(parsed_packet);
                }
            }
            Event::End(end) => {
                let frame = stack.pop().ok_or(ParseRefusal::Unrecognized)?;
                let name = resolve_name(end.name().as_ref(), &frame.namespaces, false)?;
                if name.namespace != frame.name.namespace || name.local != frame.name.local {
                    return Err(ParseRefusal::Unrecognized);
                }
            }
            Event::Text(text) => {
                let text = text.xml10_content();
                if !text.chars().all(char::is_whitespace) {
                    return Err(ParseRefusal::Unrecognized);
                }
            }
            Event::Eof => break,
            _ => return Err(ParseRefusal::Unrecognized),
        }
        buffer.clear();
    }
    if !stack.is_empty() || !saw_root {
        return Err(ParseRefusal::Unrecognized);
    }
    structure.require_owned_shape()?;
    packet.ok_or(ParseRefusal::Unrecognized)
}

fn inspect_element(
    start: &BytesStart<'_>,
    stack: &[ElementFrame],
) -> Result<InspectedElement, ParseRefusal> {
    let mut namespaces = stack
        .last()
        .map(|frame| frame.namespaces.clone())
        .unwrap_or_default();
    let mut raw_attributes = Vec::new();
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|_| ParseRefusal::Unrecognized)?;
        let key = attribute.key.as_ref().to_owned();
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|_| ParseRefusal::Unrecognized)?
            .into_owned();
        if key == "xmlns" {
            namespaces.insert(String::new(), value);
        } else if let Some(prefix) = key.strip_prefix("xmlns:") {
            namespaces.insert(prefix.to_owned(), value);
        } else {
            raw_attributes.push((key, value));
        }
    }
    let name = resolve_name(start.name().as_ref(), &namespaces, false)?;
    let depth = stack.len();
    match depth {
        0 if name.namespace.as_deref() == Some(XMP_NS) && name.local == "xmpmeta" => {
            if !raw_attributes.is_empty() {
                return Err(ParseRefusal::Unrecognized);
            }
            Ok(InspectedElement {
                name,
                namespaces,
                packet: None,
            })
        }
        1 if name.namespace.as_deref() == Some(RDF_NS) && name.local == "RDF" => {
            if stack[0].name.namespace.as_deref() != Some(XMP_NS)
                || stack[0].name.local != "xmpmeta"
                || !raw_attributes.is_empty()
            {
                return Err(ParseRefusal::Unrecognized);
            }
            Ok(InspectedElement {
                name,
                namespaces,
                packet: None,
            })
        }
        2 if name.namespace.as_deref() == Some(RDF_NS) && name.local == "Description" => {
            if stack[1].name.namespace.as_deref() != Some(RDF_NS) || stack[1].name.local != "RDF" {
                return Err(ParseRefusal::Unrecognized);
            }
            let packet = parse_description_attributes(raw_attributes, &namespaces)?;
            Ok(InspectedElement {
                name,
                namespaces,
                packet: Some(packet),
            })
        }
        _ => Err(ParseRefusal::Unrecognized),
    }
}

fn parse_description_attributes(
    raw_attributes: Vec<(String, String)>,
    namespaces: &HashMap<String, String>,
) -> Result<OwnedPacket, ParseRefusal> {
    let mut about = None;
    let mut owner = None;
    let mut schema = None;
    let mut source_file_name = None;
    let mut exposure = None;
    for (key, value) in raw_attributes {
        let name = resolve_name(&key, namespaces, true)?;
        match (name.namespace.as_deref(), name.local.as_str()) {
            (Some(RDF_NS), "about") if about.is_none() => about = Some(value),
            (Some(CREMA_NS), "owner") if owner.is_none() => owner = Some(value),
            (Some(CREMA_NS), "schemaVersion") if schema.is_none() => schema = Some(value),
            (Some(CREMA_NS), "sourceFileName") if source_file_name.is_none() => {
                source_file_name = Some(value)
            }
            (Some(CREMA_NS), "exposureCentistops") if exposure.is_none() => exposure = Some(value),
            _ => return Err(ParseRefusal::Unrecognized),
        }
    }
    if about.as_deref() != Some("") || owner.as_deref() != Some("Crema") {
        return Err(ParseRefusal::Unrecognized);
    }
    let schema = schema
        .ok_or(ParseRefusal::Unrecognized)?
        .parse::<u32>()
        .map_err(|_| ParseRefusal::Unrecognized)?;
    if schema > CURRENT_SCHEMA {
        return Err(ParseRefusal::NewerSchema(schema));
    }
    if schema == 0 || (schema == 1 && source_file_name.is_some()) {
        return Err(ParseRefusal::Unrecognized);
    }
    if schema == CURRENT_SCHEMA && source_file_name.as_deref().is_none_or(str::is_empty) {
        return Err(ParseRefusal::Unrecognized);
    }
    let exposure = exposure
        .ok_or(ParseRefusal::Unrecognized)?
        .parse::<i16>()
        .map_err(|_| ParseRefusal::Unrecognized)?;
    let exposure = ExposureCentistops::new(exposure).map_err(|_| ParseRefusal::Unrecognized)?;
    Ok(OwnedPacket {
        recipe: EditRecipe::new(exposure),
        source_file_name,
    })
}

fn resolve_name(
    raw: &str,
    namespaces: &HashMap<String, String>,
    attribute: bool,
) -> Result<ElementName, ParseRefusal> {
    let (prefix, local) = match raw.split_once(':') {
        Some(parts) => parts,
        None => ("", raw),
    };
    if local.is_empty() {
        return Err(ParseRefusal::Unrecognized);
    }
    let namespace = if prefix.is_empty() && attribute {
        None
    } else {
        namespaces.get(prefix).cloned()
    };
    if !prefix.is_empty() && namespace.is_none() {
        return Err(ParseRefusal::Unrecognized);
    }
    Ok(ElementName {
        namespace,
        local: local.to_owned(),
    })
}
